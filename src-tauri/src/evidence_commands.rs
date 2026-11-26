//! The Evidence weight tab's four commands (design 2026-11-24, §7), shaped as
//! PRISMA's in `commands.rs` and built from its helpers; kept apart because
//! that file is already long.
//!
//! Serving a run records when the tab first showed it with a RoB 2 result
//! (`evidence_store::mark_shown`), which phase 3 reads to know whether an
//! appraisal saved later was blind.

use std::sync::atomic::AtomicBool;

use studyanalyzer_core::config::{NO_FULL_TEXT_YET, RETRY_BASE_DELAY};
use studyanalyzer_core::evidence::{self, EvidenceContext, EvidenceEvent};
use studyanalyzer_core::evidence_store;
use studyanalyzer_core::evidence_view::{self, EvidenceRunView, EvidenceView};
use studyanalyzer_core::interrogate;
use studyanalyzer_core::llm_settings;
use studyanalyzer_core::pico;
use studyanalyzer_core::store::{PaperId, StoreError};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime};

use crate::commands::{
    blocking, event_sink, known_paper, not_found_is_input, run_room, stop_slot, stored_view,
    with_claim,
};
use crate::error::{CommandError, ErrorKind};
use crate::state::AppState;

/// What a second evidence weight run is refused with while one goes.
const EVIDENCE_RUNNING: &str = "An evidence weight assessment is already running.";

/// What a request for an evidence run that is not stored (or not this
/// paper's) is refused with.
const EVIDENCE_RUN_NOT_STORED: &str = "That evidence weight run is not stored.";

/// What the stop log and the stored-view error name the run as.
const WHAT: &str = "evidence weight";

/// `view` as served: a run with a RoB 2 result not yet marked shown is marked
/// now (only while empty, so the first time is kept), and read again so it
/// carries the time stored.
fn served(
    db: &mut dyn bmlib::db::Db,
    view: EvidenceRunView,
) -> Result<EvidenceRunView, StoreError> {
    if !view.has_rob2_result || view.first_shown_at.is_some() {
        return Ok(view);
    }
    evidence_store::mark_shown(db, view.id)?;
    evidence_view::evidence_run_view(db, view.id)
}

/// Whether the tab opens on the machine's runs, so its current run is on
/// screen (design 2026-11-26, §3): a run was shown and no appraisal is open.
/// Otherwise it offers the choice or opens the appraisal, and showing the run
/// is left to the reader (*Show the machine's verdict* serves it through
/// [`evidence_run`]).
fn shows_runs(view: &EvidenceView) -> bool {
    view.any_shown && !matches!(&view.appraisal, Some(a) if !a.set_aside)
}

/// The tab of `paper` with its current run served ([`served`]); with
/// `opening`, only when the tab opens on the runs ([`shows_runs`]), so a run
/// beside the choice is never recorded as shown. A run the reader started is
/// on screen, so its view is always served.
fn paper_view(
    db: &mut dyn bmlib::db::Db,
    paper: &PaperId,
    opening: bool,
) -> Result<EvidenceView, StoreError> {
    let mut view = evidence_view::evidence_view(db, paper)?;
    if opening && !shows_runs(&view) {
        return Ok(view);
    }
    if let Some(run) = view.current.take() {
        view.current = Some(served(db, run)?);
    }
    Ok(view)
}

/// What the Evidence weight tab of paper `paper_id` opens with;
/// `not_full_text` for a paper with no text.
fn evidence_with(s: &AppState, paper_id: &str) -> Result<EvidenceView, CommandError> {
    let mut db = s.connect()?;
    let paper = known_paper(&mut db, paper_id)?;
    paper_view(&mut db, &paper, true).map_err(|e| match e {
        StoreError::NotFound(_) => {
            CommandError::new(ErrorKind::NotFullText, NO_FULL_TEXT_YET.into())
        }
        e => e.into(),
    })
}

/// What the Evidence weight tab opens with (never runs anything);
/// `not_full_text` for a paper with no text. A current run with a RoB 2
/// result is recorded as shown only when the tab opens on the runs: not
/// beside the choice, nor an open appraisal.
#[tauri::command]
pub async fn evidence<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
) -> Result<EvidenceView, CommandError> {
    blocking(app, move |s| evidence_with(s, &paper_id)).await
}

/// The stored evidence run `run_id`, served ([`served`]), or `invalid_input`
/// when there is none.
fn evidence_run_with(s: &AppState, run_id: i64) -> Result<EvidenceRunView, CommandError> {
    let mut db = s.connect()?;
    evidence_view::evidence_run_view(&mut db, run_id)
        .and_then(|view| served(&mut db, view))
        .map_err(|e| not_found_is_input(e, EVIDENCE_RUN_NOT_STORED))
}

/// One stored evidence run (an earlier one, from the tab's list), recorded as
/// shown when it has a RoB 2 result.
#[tauri::command]
pub async fn evidence_run<R: Runtime>(
    app: AppHandle<R>,
    run_id: i64,
) -> Result<EvidenceRunView, CommandError> {
    blocking(app, move |s| evidence_run_with(s, run_id)).await
}

/// The `provider:model` an evidence call on `paper` uses, as PRISMA's: a new
/// run (`continue_run` `None`) takes the model chosen in Settings, checked as
/// an ask is (none is `needs_model`, then a paper without full text is
/// refused); Continue takes run `continue_run`'s own model and text, so
/// neither check applies. A run that is not the paper's is `invalid_input`.
fn evidence_model(
    db: &mut dyn bmlib::db::Db,
    paper: &PaperId,
    continue_run: Option<i64>,
) -> Result<String, CommandError> {
    let Some(run_id) = continue_run else {
        // Never a chat error, so no provider to name.
        return interrogate::ready_to_ask(db, paper).map_err(|e| CommandError::from_ask(e, ""));
    };
    match evidence_store::runs(db, paper)?
        .into_iter()
        .find(|r| r.id == run_id)
    {
        Some(r) => Ok(r.model),
        None => {
            log::warn!(
                "evidence run {run_id} is not one of paper {}'s",
                paper.as_str()
            );
            Err(CommandError::invalid_input(EVIDENCE_RUN_NOT_STORED))
        }
    }
}

/// Assesses the evidence weight of paper `paper_id` and returns the tab's
/// view (its current run served): a new run with the chosen model, or the
/// rest of run `continue_run` with its own ([`evidence_model`]). The model
/// (and, for a new run, the paper's text) and the provider's key are checked
/// before the provider is reached. Steps and condensations report to
/// `on_event`. A run that stops or fails still has its current run served
/// before its error returns, since the tab shows what it finished.
fn run_evidence_with(
    s: &AppState,
    paper_id: &str,
    continue_run: Option<i64>,
    stop: &AtomicBool,
    on_event: &Channel<EvidenceEvent>,
) -> Result<EvidenceView, CommandError> {
    let mut db = s.connect()?;
    let paper = known_paper(&mut db, paper_id)?;
    let model = evidence_model(&mut db, &paper, continue_run)?;
    let (provider, client) = llm_settings::llm_client(&model, s.secrets.as_ref())?;
    // bmlib would report a missing key as a failed request, after the paper was read.
    if let Some(env_var) = llm_settings::missing_key(&provider, &client) {
        return Err(CommandError::needs_api_key(&provider, env_var));
    }
    let (window, reply_tokens) = run_room(s, &mut db, &client, &provider, &model)?;
    // PICO's builders: the chat and the condenser share the run's stop.
    let mut chat = pico::pico_chat(&client, &model, window, stop);
    let condenser = pico::pico_condenser_model(&client, &model, window, stop);
    let mut ctx = EvidenceContext {
        db: &mut db,
        chat: &mut chat,
        condenser_model: &condenser,
        model,
        window,
        reply_tokens,
        retry_delay: RETRY_BASE_DELAY,
        on_event: event_sink(on_event, WHAT),
    };
    let ran = match continue_run {
        Some(run_id) => evidence::continue_evidence(&mut ctx, run_id, stop),
        None => evidence::run_evidence(&mut ctx, &paper, stop),
    };
    if let Err(e) = ran {
        // The tab shows what a stopped or failed run finished, so it is
        // recorded as shown too, or a later appraisal would count as blind.
        if let Err(view) = paper_view(&mut db, &paper, false) {
            log::error!(
                "paper {}: the run that ended could not be recorded as shown: {view}",
                paper.as_str()
            );
        }
        return Err(CommandError::from_evidence(e, &provider));
    }
    stored_view(WHAT, paper_view(&mut db, &paper, false))
}

/// Assesses the paper's evidence weight: with `continue_run` `None`, a new
/// run with the model chosen in Settings (the design, the primary outcomes,
/// then RoB 2 step by step); with a run's id, the steps that run has left, on
/// its own model and text (a run made under earlier rules is refused). One
/// run at a time: a second is `invalid_input`. [`stop_evidence`] stops it
/// before its next model call; the steps finished so far are kept for
/// Continue. Steps and condensations report to `on_event`. Returns the tab's
/// view.
#[tauri::command]
pub async fn run_evidence<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
    continue_run: Option<i64>,
    on_event: Channel<EvidenceEvent>,
) -> Result<EvidenceView, CommandError> {
    // Claimed before the blocking task is queued, so a Stop pressed at once is not lost.
    let stop = app
        .try_state::<AppState>()
        .map(|s| s.evidence_stop.try_begin());
    blocking(app, move |s| {
        let claim = stop.ok_or_else(|| CommandError::internal("the app did not start"))?;
        with_claim(&s.evidence_stop, claim, EVIDENCE_RUNNING, |flag| {
            run_evidence_with(s, &paper_id, continue_run, flag, &on_event)
        })
    })
    .await
}

/// Stops the running evidence weight assessment, if there is one ([`stop_slot`]).
#[tauri::command]
pub fn stop_evidence<R: Runtime>(app: AppHandle<R>) -> Result<bool, CommandError> {
    stop_slot(&app, WHAT, |s| &s.evidence_stop)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::commands::tests::{mock_app, temp_state};
    use crate::state::Release;
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_markdown};
    use bmlib::quality::{
        applicable_questions, domain_judgement, Answer, Answers, DesignProposal, Domain,
        DomainAssessment, DomainResult, MachineAnswer, OutcomeProposal, ProposedOutcome, RctLayout,
        Rob2Step, StudyDesign, DESIGN_PROMPT_VERSION, OUTCOME_PROMPT_VERSION,
    };
    use studyanalyzer_core::evidence_store::EvidenceStep;
    use studyanalyzer_core::intake::SourceKind;
    use studyanalyzer_core::store;

    /// A paper with a full text (confirmed) and an evidence run on it by
    /// `model`, its design a parallel-group trial and its one outcome saved,
    /// so RoB 2's steps are pending; returns the paper and the run's id.
    fn paper_with_evidence_run(db: &mut dyn bmlib::db::Db, model: &str) -> (PaperId, i64) {
        let p = store::create_paper(db, &Default::default(), None).unwrap();
        let source = store::NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: b"# Methods\n\nWe randomised.".to_vec(),
            decoded: None,
        };
        let sid = store::save_source(db, &p, source, store::SourcePlace::App).unwrap();
        let text = canonical_from_markdown("# Methods\n\nWe randomised.");
        let assessment = assess_full_text(&text).unwrap();
        let cid = store::save_canonical(db, &p, sid, &text, &assessment).unwrap();
        store::confirm_full_text(db, &p, cid).unwrap();
        let run = proposed_run(db, &p, cid, model);
        (p, run)
    }

    /// A run of paper `p` on text `cid` by `model`, its design a
    /// parallel-group trial and its one outcome saved, so RoB 2's steps are
    /// pending; returns its id.
    fn proposed_run(db: &mut dyn bmlib::db::Db, p: &PaperId, cid: i64, model: &str) -> i64 {
        let run = evidence_store::create_run(db, p, cid, model, None).unwrap();
        let design = DesignProposal {
            design: StudyDesign::Rct,
            rct_layout: Some(RctLayout::Parallel),
            citations: vec![],
            quotes_not_found: vec![],
            contradicted: vec![],
            quotes_dropped: 0,
            reasoning: "r".into(),
            condensed: false,
            prompt_version: DESIGN_PROMPT_VERSION,
        };
        let step = evidence_store::pending_steps(db, run).unwrap()[0].id;
        evidence_store::save_design(db, step, &design).unwrap();
        let outcome = ProposedOutcome {
            name: "pain".into(),
            timepoint: None,
            result: None,
            result_error: Some("no estimate".into()),
            citations: vec![],
            quotes_not_found: vec![],
            contradicted: vec![],
            quotes_dropped: 0,
        };
        let outcomes = OutcomeProposal {
            outcomes: vec![outcome],
            condensed: false,
            prompt_version: OUTCOME_PROMPT_VERSION,
        };
        let step = evidence_store::pending_steps(db, run).unwrap()[0].id;
        evidence_store::save_outcomes(db, step, &outcomes).unwrap();
        run
    }

    /// Saves run `run`'s D1 as assessed, every applicable question answered Y.
    pub(crate) fn save_d1(db: &mut dyn bmlib::db::Db, run: i64) {
        let d1 = EvidenceStep::Rob2(Rob2Step::Domain {
            domain: Domain::D1,
            outcome: None,
        });
        let step = evidence_store::pending_steps(db, run)
            .unwrap()
            .into_iter()
            .find(|s| s.step == d1)
            .unwrap()
            .id;
        let mut given = Answers::new();
        loop {
            let next: Vec<&str> = applicable_questions(Domain::D1, &given)
                .into_iter()
                .filter(|q| !given.contains_key(*q))
                .collect();
            if next.is_empty() {
                break;
            }
            for q in next {
                given.insert(q.to_string(), Answer::Y);
            }
        }
        let judgement = domain_judgement(Domain::D1, &given).unwrap();
        let answers = given
            .iter()
            .map(|(q, a)| {
                let machine = MachineAnswer {
                    answer: *a,
                    citations: vec![],
                    quotes_not_found: vec![],
                    contradicted: vec![],
                    quotes_dropped: 0,
                    reasoning: "a".into(),
                };
                (q.clone(), machine)
            })
            .collect();
        let result = DomainResult::Assessed(DomainAssessment {
            domain: Domain::D1,
            answers,
            judgement,
            condensed: false,
            ignored: vec![],
            facts: None,
        });
        evidence_store::save_domain(db, step, &result).unwrap();
    }

    /// A fresh run with no model chosen is refused before the provider is
    /// reached, and stores nothing.
    #[test]
    fn run_evidence_needs_a_model() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let p = store::create_paper(&mut db, &Default::default(), None).unwrap();
        let events = Channel::new(|_| Ok(()));
        let flag = AtomicBool::new(false);
        let e = run_evidence_with(&s, p.as_str(), None, &flag, &events).unwrap_err();
        let runs = evidence_store::runs(&mut db, &p).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(e.kind, ErrorKind::NeedsModel);
        assert!(runs.is_empty());
    }

    /// A run is marked shown only once it has a RoB 2 result, by the run's
    /// own view (*Show the machine's verdict*, or an earlier run from the
    /// list) and, once a run was shown, by the tab's; the first time is kept
    /// on every later serving.
    #[test]
    fn evidence_marks_first_shown_once() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let (p, run) = paper_with_evidence_run(&mut db, "anthropic:m");
        let shown = |s: &AppState| evidence_with(s, p.as_str()).unwrap().current.unwrap();
        assert_eq!(evidence_run_with(&s, run).unwrap().first_shown_at, None);
        save_d1(&mut db, run);
        let first = evidence_run_with(&s, run).unwrap().first_shown_at;
        assert!(first.is_some());
        // A later serving neither changes nor clears it.
        assert!(!evidence_store::mark_shown(&mut db, run).unwrap());
        assert_eq!(shown(&s).first_shown_at, first);
        assert_eq!(evidence_run_with(&s, run).unwrap().first_shown_at, first);
        // Once a run of the paper was shown, the tab's view marks its current run.
        let cid = evidence_store::run(&mut db, run)
            .unwrap()
            .unwrap()
            .canonical_id;
        let next = proposed_run(&mut db, &p, cid, "anthropic:m");
        save_d1(&mut db, next);
        let current = shown(&s);
        assert_eq!(current.id, next);
        assert!(current.first_shown_at.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Beside the choice (no run shown, no appraisal open) the tab's opening
    /// view records nothing as shown, even with a RoB 2 result there, so an
    /// appraisal started then is blind; *Show the machine's verdict* serves
    /// the run through its own view, which marks it.
    #[test]
    fn the_opening_view_marks_nothing_beside_the_choice() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let (p, run) = paper_with_evidence_run(&mut db, "anthropic:m");
        save_d1(&mut db, run);
        let stored = |db: &mut dyn bmlib::db::Db| {
            evidence_store::run(db, run)
                .unwrap()
                .unwrap()
                .first_shown_at
        };
        let view = evidence_with(&s, p.as_str()).unwrap();
        assert!(!view.any_shown);
        assert!(view.current.unwrap().has_rob2_result);
        assert_eq!(stored(&mut db), None);
        let a = studyanalyzer_core::appraisal_store::start_appraisal(&mut db, &p).unwrap();
        let blind = studyanalyzer_core::appraisal_rows::appraisal(&mut db, a)
            .unwrap()
            .unwrap()
            .blind;
        assert!(blind);
        // An open appraisal keeps the run unshown too.
        assert!(!evidence_with(&s, p.as_str()).unwrap().any_shown);
        assert_eq!(stored(&mut db), None);
        assert!(evidence_run_with(&s, run).unwrap().first_shown_at.is_some());
        assert!(stored(&mut db).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A run the reader started that stops (or fails) after a RoB 2 result
    /// was stored is still recorded as shown, since the tab shows what it
    /// finished: so an appraisal started after it is not blind.
    #[test]
    fn a_stopped_run_is_recorded_as_shown() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let (p, run) = paper_with_evidence_run(&mut db, "ollama:m");
        save_d1(&mut db, run);
        crate::state::lock(&s.limits).insert("ollama:m".into(), Default::default());
        let events = Channel::new(|_| Ok(()));
        let stopped = AtomicBool::new(true);
        let e = run_evidence_with(&s, p.as_str(), Some(run), &stopped, &events).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert!(evidence_with(&s, p.as_str()).unwrap().any_shown);
        let a = studyanalyzer_core::appraisal_store::start_appraisal(&mut db, &p).unwrap();
        let blind = studyanalyzer_core::appraisal_rows::appraisal(&mut db, a)
            .unwrap()
            .unwrap()
            .blind;
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!blind);
    }

    /// A damaged appraisal row leaves the tab open, the appraisal held apart
    /// by its id, which *Set this appraisal aside* takes: once set aside
    /// (saying it could not be shown), nothing is held apart.
    #[test]
    fn a_damaged_appraisal_can_be_set_aside_from_the_tab() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let (p, _) = paper_with_evidence_run(&mut db, "anthropic:m");
        let a = studyanalyzer_core::appraisal_store::start_appraisal(&mut db, &p).unwrap();
        bmlib::db::execute(
            &mut db,
            "UPDATE evidence_appraisals SET design = 'renamed_design'",
            &[],
        )
        .unwrap();
        let (_app, h) = mock_app(s);
        let tab = |h: &AppHandle<tauri::test::MockRuntime>| {
            tauri::async_runtime::block_on(evidence(h.clone(), p.as_str().to_string())).unwrap()
        };
        let v = tab(&h);
        assert_eq!(v.appraisal, None);
        assert_eq!(v.appraisal_error.map(|d| d.id), Some(a));
        let aside = crate::appraisal_commands::set_aside_appraisal(h.clone(), a);
        let e = tauri::async_runtime::block_on(aside).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Storage);
        assert!(
            e.message.starts_with("The appraisal was set aside"),
            "{}",
            e.message
        );
        let v = tab(&h);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!((v.appraisal, v.appraisal_error), (None, None));
    }

    /// Continue runs on the run's own model, even with another chosen; a run
    /// that is not the paper's, or not stored, is `invalid_input`.
    #[test]
    fn continue_uses_the_runs_model() {
        let mut db = store::open_memory_store().unwrap();
        let (p, run) = paper_with_evidence_run(&mut db, "anthropic:run-model");
        llm_settings::set_model(&mut db, "ollama:chosen").unwrap();
        assert_eq!(
            evidence_model(&mut db, &p, Some(run)).unwrap(),
            "anthropic:run-model"
        );
        assert_eq!(evidence_model(&mut db, &p, None).unwrap(), "ollama:chosen");
        let other = store::create_paper(&mut db, &Default::default(), None).unwrap();
        let e = evidence_model(&mut db, &other, Some(run)).unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::InvalidInput, EVIDENCE_RUN_NOT_STORED)
        );
    }

    /// Setting or clearing the context-window limit clears a run's too-long
    /// mark, since the window it was judged against changed (#166).
    #[test]
    fn a_window_setting_change_clears_too_long() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let (_, run) = paper_with_evidence_run(&mut db, "ollama:m");
        let marked =
            |db: &mut dyn bmlib::db::Db| evidence_store::run(db, run).unwrap().unwrap().too_long;
        let (_app, h) = mock_app(s);
        for change in [true, false] {
            evidence_store::mark_too_long(&mut db, run).unwrap();
            assert!(marked(&mut db));
            let done = if change {
                tauri::async_runtime::block_on(crate::commands::set_context_tokens(
                    h.clone(),
                    32_768,
                ))
            } else {
                tauri::async_runtime::block_on(crate::commands::clear_context_tokens(h.clone()))
            };
            done.unwrap();
            assert!(!marked(&mut db), "set {change}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The tab of a paper with no text is `not_full_text`; a run not stored
    /// is `invalid_input`; a second run is refused in the evidence weight's
    /// words; a stop reaches only the evidence run's slot.
    #[test]
    fn refusals_and_stop() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let p = store::create_paper(&mut db, &Default::default(), None).unwrap();
        let e = evidence_with(&s, p.as_str()).unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::NotFullText, NO_FULL_TEXT_YET)
        );
        let e = evidence_run_with(&s, 7).unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::InvalidInput, EVIDENCE_RUN_NOT_STORED)
        );
        let (_app, h) = mock_app(s);
        let state = h.state::<AppState>();
        let held = Release::new(
            &state.evidence_stop,
            state.evidence_stop.try_begin().unwrap(),
        );
        // The command itself refuses a second run while one holds the slot.
        let events = Channel::new(|_| Ok(()));
        let second = run_evidence(h.clone(), p.as_str().to_string(), None, events);
        let e = tauri::async_runtime::block_on(second).unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (
                ErrorKind::InvalidInput,
                "An evidence weight assessment is already running."
            )
        );
        assert!(!crate::commands::stop_prisma(h.clone()).unwrap());
        assert!(stop_evidence(h.clone()).unwrap());
        assert!(held.flag().load(std::sync::atomic::Ordering::SeqCst));
        drop(held);
        assert!(!stop_evidence(h.clone()).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
