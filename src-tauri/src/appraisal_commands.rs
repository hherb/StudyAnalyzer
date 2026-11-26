//! The reader's appraisal's nine commands (design 2026-11-26, §3–§7), shaped
//! as the evidence weight's in `evidence_commands.rs`, each returning the
//! appraisal's view ([`AppraisalView`]).
//!
//! The paired run shares the evidence weight's one slot (`evidence_stop`;
//! stopping it is `stop_evidence`), and records in `evidence_appraisal`
//! which appraisal holds it. Its events reach the tab only as counts
//! ([`AppraisalEvent::Progress`]). A view served once the appraisal is
//! revealed marks the paired run shown (`evidence_store::mark_shown`).

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use studyanalyzer_core::appraisal_rows;
use studyanalyzer_core::appraisal_store::{self, AppraisalEdit, ConfirmInput};
use studyanalyzer_core::appraisal_view::{self, AppraisalEvent, AppraisalView};
use studyanalyzer_core::config::RETRY_BASE_DELAY;
use studyanalyzer_core::evidence::{self, EvidenceContext, EvidenceEvent};
use studyanalyzer_core::evidence_store;
use studyanalyzer_core::interrogate;
use studyanalyzer_core::llm_settings;
use studyanalyzer_core::pico;
use studyanalyzer_core::store::StoreError;
use studyanalyzer_core::view::{span_from_utf16, utf16_offsets, Utf16Span};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime};

use crate::commands::{
    blocking, known_paper, not_found_is_input, run_room, stored_view, with_claim,
};
use crate::error::{CommandError, ErrorKind};
use crate::state::{lock, AppState};

/// What a request for an appraisal that is not stored is refused with.
const APPRAISAL_NOT_STORED: &str = "That appraisal is not stored.";

/// What setting an appraisal aside is refused with while its paired run
/// holds the slot; the tab presses Stop first.
const STOP_FIRST: &str = "Stop the machine's assessment first.";

/// What a mark the webview sent outside the paper's text, or ending inside a
/// character, is refused with.
const SELECTION_OUTSIDE: &str = "That selection is not inside the paper's text.";

/// What a paired run is refused with while another evidence run goes: the
/// evidence weight's own words, as they share one slot.
const EVIDENCE_RUNNING: &str = "An evidence weight assessment is already running.";

/// What the stored-view error names the run as.
const WHAT: &str = "evidence weight";

/// Whether an evidence run other than appraisal `appraisal_id`'s paired run
/// holds the slot, so its run waits.
fn running_elsewhere(s: &AppState, appraisal_id: i64) -> bool {
    s.evidence_stop.busy() && *lock(&s.evidence_appraisal) != Some(appraisal_id)
}

/// Appraisal `appraisal_id`'s view of version `version` (`None`: the
/// editable one, else the first record). Once revealed, its paired run is
/// marked shown (only once a RoB 2 result is there, keeping the first time)
/// and the view read again to carry it.
fn served(
    s: &AppState,
    db: &mut dyn bmlib::db::Db,
    appraisal_id: i64,
    version: Option<u32>,
) -> Result<AppraisalView, StoreError> {
    let read = |db: &mut dyn bmlib::db::Db| {
        appraisal_view::appraisal_view(
            db,
            appraisal_id,
            version,
            running_elsewhere(s, appraisal_id),
        )
    };
    let view = read(db)?;
    match (&view.revealed_at, view.machine.run_id) {
        (Some(_), Some(run_id)) if evidence_store::mark_shown(db, run_id)? => read(db),
        _ => Ok(view),
    }
}

/// Runs `write` on appraisal `appraisal_id`, then serves its view
/// ([`served`]); no such appraisal is `invalid_input`.
fn after(
    s: &AppState,
    appraisal_id: i64,
    write: impl FnOnce(&mut dyn bmlib::db::Db) -> Result<(), StoreError>,
) -> Result<AppraisalView, CommandError> {
    let mut db = s.connect()?;
    write(&mut db)
        .and_then(|()| served(s, &mut db, appraisal_id, None))
        .map_err(|e| not_found_is_input(e, APPRAISAL_NOT_STORED))
}

/// Appraisal `appraisal_id`'s view of `version`, its editable version
/// re-derived under the current rules first (`refresh_editable`).
fn appraisal_with(
    s: &AppState,
    appraisal_id: i64,
    version: Option<u32>,
) -> Result<AppraisalView, CommandError> {
    let mut db = s.connect()?;
    appraisal_store::refresh_editable(&mut db, appraisal_id)
        .and_then(|_| served(s, &mut db, appraisal_id, version))
        .map_err(|e| not_found_is_input(e, APPRAISAL_NOT_STORED))
}

/// What the appraisal pane shows: version `version` of appraisal
/// `appraisal_id` (`None`: the editable one, else the first record). Never
/// runs anything; an editable version made under earlier rules is derived
/// again under the current ones.
#[tauri::command]
pub async fn appraisal<R: Runtime>(
    app: AppHandle<R>,
    appraisal_id: i64,
    version: Option<u32>,
) -> Result<AppraisalView, CommandError> {
    blocking(app, move |s| appraisal_with(s, appraisal_id, version)).await
}

/// Starts an appraisal of paper `paper_id`, refused as the evidence weight's
/// runs are without full text (`not_full_text`), and returns its view.
fn start_appraisal_with(s: &AppState, paper_id: &str) -> Result<AppraisalView, CommandError> {
    let mut db = s.connect()?;
    let paper = known_paper(&mut db, paper_id)?;
    // Never a chat error, so no provider to name.
    interrogate::full_text(&mut db, &paper).map_err(|e| CommandError::from_ask(e, ""))?;
    let id = appraisal_store::start_appraisal(&mut db, &paper)?;
    served(s, &mut db, id, None).map_err(CommandError::from)
}

/// *Appraise it yourself (first)*: starts an appraisal of the paper, blind
/// only if no run of it was shown. Its paired run is started by
/// [`run_appraisal`]. A paper with an open appraisal is `invalid_input`.
#[tauri::command]
pub async fn start_appraisal<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
) -> Result<AppraisalView, CommandError> {
    blocking(app, move |s| start_appraisal_with(s, &paper_id)).await
}

/// Confirms the appraisal's design and primary outcomes (steps 1 and 2),
/// readying its paired run's RoB 2 steps; what the core refuses is
/// `invalid_input`.
#[tauri::command]
pub async fn confirm_appraisal<R: Runtime>(
    app: AppHandle<R>,
    appraisal_id: i64,
    input: ConfirmInput,
) -> Result<AppraisalView, CommandError> {
    blocking(app, move |s| {
        after(s, appraisal_id, |db| {
            appraisal_store::confirm(db, appraisal_id, &input)
        })
    })
    .await
}

/// `edit` as the core takes it: a mark's span arrives in UTF-16 units of
/// appraisal `appraisal_id`'s text (the webview's), and is converted into
/// code points here, the one boundary. A span ending inside a character or
/// past the text is `invalid_input`; every other edit passes as it is.
fn from_webview(
    db: &mut dyn bmlib::db::Db,
    appraisal_id: i64,
    edit: AppraisalEdit,
) -> Result<AppraisalEdit, CommandError> {
    let (domain, outcome, question_id, units) = match edit {
        AppraisalEdit::Mark {
            domain,
            outcome,
            question_id,
            span: Some(units),
        } => (domain, outcome, question_id, units),
        other => return Ok(other),
    };
    let a = appraisal_rows::appraisal(db, appraisal_id)?
        .ok_or_else(|| CommandError::invalid_input(APPRAISAL_NOT_STORED))?;
    let units = Utf16Span {
        start: units.start,
        end: units.end,
    };
    let span = span_from_utf16(&utf16_offsets(&a.text.text), units).ok_or_else(|| {
        log::warn!("appraisal {appraisal_id}: a mark at UTF-16 {units:?} is not on the text");
        CommandError::invalid_input(SELECTION_OUTSIDE)
    })?;
    Ok(AppraisalEdit::Mark {
        domain,
        outcome,
        question_id,
        span: Some(span),
    })
}

/// Writes one of the reader's edits (an answer, a mark in UTF-16 units, the
/// sources ticks, an override, an escalation answer, an overall override or
/// a note) to the appraisal's editable version; what the core refuses is
/// `invalid_input`.
#[tauri::command]
pub async fn edit_appraisal<R: Runtime>(
    app: AppHandle<R>,
    appraisal_id: i64,
    edit: AppraisalEdit,
) -> Result<AppraisalView, CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let edit = from_webview(&mut db, appraisal_id, edit)?;
        after(s, appraisal_id, |db| {
            appraisal_store::edit(db, appraisal_id, &edit)
        })
    })
    .await
}

/// *Save and compare*: freezes the first record and reveals the machine's
/// verdict, marking its paired run shown; an incomplete record is
/// `invalid_input`.
#[tauri::command]
pub async fn save_appraisal<R: Runtime>(
    app: AppHandle<R>,
    appraisal_id: i64,
) -> Result<AppraisalView, CommandError> {
    blocking(app, move |s| {
        after(s, appraisal_id, |db| {
            appraisal_store::save(db, appraisal_id)
        })
    })
    .await
}

/// *Show the machine's verdict now*: reveals it before a save, marking the
/// appraisal not blind and its paired run shown.
#[tauri::command]
pub async fn reveal_appraisal<R: Runtime>(
    app: AppHandle<R>,
    appraisal_id: i64,
) -> Result<AppraisalView, CommandError> {
    blocking(app, move |s| {
        after(s, appraisal_id, |db| {
            appraisal_store::reveal_now(db, appraisal_id)
        })
    })
    .await
}

/// *Done revising*: closes the open revision.
#[tauri::command]
pub async fn close_revision<R: Runtime>(
    app: AppHandle<R>,
    appraisal_id: i64,
) -> Result<AppraisalView, CommandError> {
    blocking(app, move |s| {
        after(s, appraisal_id, |db| {
            appraisal_store::close_revision(db, appraisal_id)
        })
    })
    .await
}

/// Sets appraisal `appraisal_id` aside, refused (`invalid_input`) while its
/// paired run holds the slot.
fn set_aside_with(s: &AppState, appraisal_id: i64) -> Result<AppraisalView, CommandError> {
    if *lock(&s.evidence_appraisal) == Some(appraisal_id) {
        log::info!("appraisal {appraisal_id}: set aside refused while its paired run goes");
        return Err(CommandError::invalid_input(STOP_FIRST));
    }
    let mut db = s.connect()?;
    appraisal_store::set_aside(&mut db, appraisal_id)
        .map_err(|e| not_found_is_input(e, APPRAISAL_NOT_STORED))?;
    // Set aside reads only the row, so a damaged appraisal is set aside too;
    // its view may still fail, which must not read as the set aside failing.
    served(s, &mut db, appraisal_id, None).map_err(|e| {
        CommandError::new(
            ErrorKind::Storage,
            format!("The appraisal was set aside but could not be shown: {e}"),
        )
    })
}

/// *Set this appraisal aside*: kept with every version, no longer open, so a
/// new one can start. Refused while its paired run is running.
#[tauri::command]
pub async fn set_aside_appraisal<R: Runtime>(
    app: AppHandle<R>,
    appraisal_id: i64,
) -> Result<AppraisalView, CommandError> {
    blocking(app, move |s| set_aside_with(s, appraisal_id)).await
}

/// The model a paired run of appraisal `a` runs on: going on with its latest
/// paired run (`again` false), that run's own; a new or re-paired run, the
/// one chosen in Settings, checked with the paper's text as an ask is.
fn paired_model(
    db: &mut dyn bmlib::db::Db,
    a: &appraisal_rows::StoredAppraisal,
    again: bool,
) -> Result<String, CommandError> {
    match (evidence_store::latest_paired_run(db, a.id)?, again) {
        (Some(run_id), false) => Ok(evidence_store::run(db, run_id)?
            .ok_or_else(|| StoreError::NotFound(format!("evidence run {run_id}")))?
            .model),
        // Never a chat error, so no provider to name.
        _ => interrogate::ready_to_ask(db, &a.paper).map_err(|e| CommandError::from_ask(e, "")),
    }
}

/// Where a paired run's events go: each step's beginning as its counts
/// ([`appraisal_view::appraisal_event`], read on `db`, a connection of its
/// own), nothing else. A count that cannot be read, or a send that cannot
/// reach the window, is logged.
fn progress_sink(
    db: rusqlite::Connection,
    appraisal_id: i64,
    on_event: &Channel<AppraisalEvent>,
) -> Arc<dyn Fn(EvidenceEvent) + Send + Sync> {
    let channel = on_event.clone();
    let db = Mutex::new(db);
    Arc::new(move |event| {
        match appraisal_view::appraisal_event(&mut *lock(&db), appraisal_id, &event) {
            Ok(Some(e)) => {
                if let Err(e) = channel.send(e) {
                    log::warn!("an appraisal event could not reach the window: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => log::warn!("appraisal {appraisal_id}: its progress could not be read: {e}"),
        }
    })
}

/// Runs appraisal `appraisal_id`'s paired run (`evidence::run_paired`) on
/// [`paired_model`]'s model, its key and the paper's room checked before the
/// provider is reached, and returns the appraisal's view. Progress reports
/// to `on_event` as counts only.
fn run_appraisal_with(
    s: &AppState,
    appraisal_id: i64,
    again: bool,
    stop: &AtomicBool,
    on_event: &Channel<AppraisalEvent>,
) -> Result<AppraisalView, CommandError> {
    let mut db = s.connect()?;
    let a = appraisal_rows::appraisal(&mut db, appraisal_id)?
        .ok_or_else(|| CommandError::invalid_input(APPRAISAL_NOT_STORED))?;
    if a.set_aside_at.is_some() {
        return Err(CommandError::invalid_input("This appraisal was set aside."));
    }
    let model = paired_model(&mut db, &a, again)?;
    let (provider, client) = llm_settings::llm_client(&model, s.secrets.as_ref())?;
    // bmlib would report a missing key as a failed request, after the paper was read.
    if let Some(env_var) = llm_settings::missing_key(&provider, &client) {
        return Err(CommandError::needs_api_key(&provider, env_var));
    }
    let (window, reply_tokens) = run_room(s, &mut db, &client, &provider, &model)?;
    let mut chat = pico::pico_chat(&client, &model, window, stop);
    let condenser = pico::pico_condenser_model(&client, &model, window, stop);
    {
        let mut ctx = EvidenceContext {
            db: &mut db,
            chat: &mut chat,
            condenser_model: &condenser,
            model,
            window,
            reply_tokens,
            retry_delay: RETRY_BASE_DELAY,
            on_event: progress_sink(s.connect()?, appraisal_id, on_event),
        };
        evidence::run_paired(&mut ctx, appraisal_id, again, stop)
            .map_err(|e| CommandError::from_evidence(e, &provider))?;
    }
    stored_view(WHAT, served(s, &mut db, appraisal_id, None))
}

/// Clears the appraisal holding the slot when dropped, a panic's unwind
/// included, so a set aside is never refused for a run that is over.
struct Holding<'a>(&'a Mutex<Option<i64>>);

impl Drop for Holding<'_> {
    /// Clears the holder.
    fn drop(&mut self) {
        *lock(self.0) = None;
    }
}

/// Runs the appraisal's paired machine run: with `again` false, the first
/// run (design and outcome proposals, then it waits for confirmation) or the
/// rest of the latest, on its own model; with `again`, *Run the machine
/// again on these outcomes* with the model chosen in Settings. One evidence
/// run at a time: a second is `invalid_input`. [`stop_evidence`] stops it
/// before its next model call. Progress reports to `on_event` as counts
/// only. Returns the appraisal's view.
///
/// [`stop_evidence`]: crate::evidence_commands::stop_evidence
#[tauri::command]
pub async fn run_appraisal<R: Runtime>(
    app: AppHandle<R>,
    appraisal_id: i64,
    again: bool,
    on_event: Channel<AppraisalEvent>,
) -> Result<AppraisalView, CommandError> {
    // Claimed before the blocking task is queued, so a Stop pressed at once is not lost.
    let stop = app
        .try_state::<AppState>()
        .map(|s| s.evidence_stop.try_begin());
    blocking(app, move |s| {
        let claim = stop.ok_or_else(|| CommandError::internal("the app did not start"))?;
        with_claim(&s.evidence_stop, claim, EVIDENCE_RUNNING, |flag| {
            *lock(&s.evidence_appraisal) = Some(appraisal_id);
            let _holding = Holding(&s.evidence_appraisal);
            run_appraisal_with(s, appraisal_id, again, flag, &on_event)
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tests::{mock_app, temp_state};
    use crate::state::Release;
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_markdown, TextSpan};
    use bmlib::quality::Domain;
    use studyanalyzer_core::intake::SourceKind;
    use studyanalyzer_core::store::{self, PaperId};

    /// A paper's text, with an astral character before "twice".
    const TEXT: &str = "# Methods\n\nWe randomised 😀 twice.";

    /// A paper with [`TEXT`] as its confirmed full text.
    fn paper(db: &mut dyn bmlib::db::Db) -> PaperId {
        let p = store::create_paper(db, &Default::default(), None).unwrap();
        let source = store::NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: TEXT.as_bytes().to_vec(),
            decoded: None,
        };
        let sid = store::save_source(db, &p, source, store::SourcePlace::App).unwrap();
        let text = canonical_from_markdown(TEXT);
        let assessment = assess_full_text(&text).unwrap();
        let cid = store::save_canonical(db, &p, sid, &text, &assessment).unwrap();
        store::confirm_full_text(db, &p, cid).unwrap();
        p
    }

    /// A paired run with no model chosen is refused before the provider is
    /// reached, and makes no run.
    #[test]
    fn run_appraisal_needs_a_model() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let p = paper(&mut db);
        let id = start_appraisal_with(&s, p.as_str()).unwrap().id;
        let events = Channel::new(|_| Ok(()));
        let flag = AtomicBool::new(false);
        let e = run_appraisal_with(&s, id, false, &flag, &events).unwrap_err();
        let run = evidence_store::latest_paired_run(&mut db, id).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(e.kind, ErrorKind::NeedsModel);
        assert_eq!(run, None);
    }

    /// While an appraisal's paired run holds the slot, setting it aside is
    /// refused in the pane's words and changes nothing, a second run is
    /// refused as the evidence weight's, and only another appraisal says its
    /// run waits; once the run is over, it can be set aside.
    #[test]
    fn set_aside_is_refused_while_running() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let p = paper(&mut db);
        let id = start_appraisal_with(&s, p.as_str()).unwrap().id;
        let (_app, h) = mock_app(s);
        let state = h.state::<AppState>();
        let held = Release::new(
            &state.evidence_stop,
            state.evidence_stop.try_begin().unwrap(),
        );
        *lock(&state.evidence_appraisal) = Some(id);
        let e = set_aside_with(&state, id).unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::InvalidInput, STOP_FIRST)
        );
        assert_eq!(
            appraisal_store::open_appraisal(&mut db, &p).unwrap(),
            Some(id)
        );
        assert!(!running_elsewhere(&state, id) && running_elsewhere(&state, id + 1));
        assert!(
            !appraisal_with(&state, id, None)
                .unwrap()
                .machine
                .running_elsewhere
        );
        let events = Channel::new(|_| Ok(()));
        let second = run_appraisal(h.clone(), id, false, events);
        let e = tauri::async_runtime::block_on(second).unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::InvalidInput, EVIDENCE_RUNNING)
        );
        drop(Holding(&state.evidence_appraisal));
        drop(held);
        assert!(!state.evidence_stop.busy());
        let set_aside = set_aside_appraisal(h.clone(), id);
        let view = tauri::async_runtime::block_on(set_aside).unwrap();
        assert!(view.set_aside_at.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Appraisal `id` of paper `p` (text [`TEXT`]) confirmed on a parallel
    /// trial with one outcome, its paired run's D1 done, so a RoB 2 result is
    /// there to be shown; returns the run.
    fn confirmed_with_d1(db: &mut dyn bmlib::db::Db, p: &PaperId, id: i64) -> i64 {
        use bmlib::quality::{
            DesignProposal, OutcomeProposal, ProposedOutcome, RctLayout, StudyDesign,
            DESIGN_PROMPT_VERSION, OUTCOME_PROMPT_VERSION,
        };
        let cid = store::latest_canonical_id(db, p).unwrap().unwrap();
        let run = evidence_store::create_run(db, p, cid, "anthropic:m", Some(id)).unwrap();
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
        let outcomes = OutcomeProposal {
            outcomes: vec![ProposedOutcome {
                name: "pain".into(),
                timepoint: None,
                result: None,
                result_error: Some("no estimate".into()),
                citations: vec![],
                quotes_not_found: vec![],
                contradicted: vec![],
                quotes_dropped: 0,
            }],
            condensed: false,
            prompt_version: OUTCOME_PROMPT_VERSION,
        };
        let step = evidence_store::pending_steps(db, run).unwrap()[0].id;
        evidence_store::save_outcomes(db, step, &outcomes).unwrap();
        let input = ConfirmInput {
            design: "rct".into(),
            rct_layout: Some("parallel".into()),
            outcomes: vec![appraisal_store::ConfirmedOutcomeInput {
                from_proposed: Some(0),
                name: "pain".into(),
                timepoint: None,
                result: None,
            }],
        };
        appraisal_store::confirm(db, id, &input).unwrap();
        crate::evidence_commands::tests::save_d1(db, run);
        run
    }

    /// When paired run `run` was first shown, and whether the tab's opening
    /// view says a run of `p` was.
    fn shown(s: &AppState, p: &PaperId, run: i64) -> (Option<String>, bool) {
        let mut db = s.connect().unwrap();
        let at = evidence_store::run(&mut db, run)
            .unwrap()
            .unwrap()
            .first_shown_at;
        let any = studyanalyzer_core::evidence_view::evidence_view(&mut db, p)
            .unwrap()
            .any_shown;
        (at, any)
    }

    /// Serving the appraisal before its reveal never marks its paired run
    /// shown, even with a RoB 2 result there; *Show the machine's verdict
    /// now* and *Save and compare* do, and the tab's view then says a run was
    /// shown.
    #[test]
    fn the_reveal_marks_the_paired_run_shown() {
        use bmlib::quality::Answer;
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let (revealed, saved) = (paper(&mut db), paper(&mut db));
        let a = start_appraisal_with(&s, revealed.as_str()).unwrap().id;
        let b = start_appraisal_with(&s, saved.as_str()).unwrap().id;
        let run_a = confirmed_with_d1(&mut db, &revealed, a);
        let run_b = confirmed_with_d1(&mut db, &saved, b);
        assert!(appraisal_with(&s, a, None)
            .unwrap()
            .machine
            .revealed
            .is_none());
        assert_eq!(shown(&s, &revealed, run_a), (None, false));
        let (_app, h) = mock_app(s);
        let state = h.state::<AppState>();
        let view = tauri::async_runtime::block_on(reveal_appraisal(h.clone(), a)).unwrap();
        assert!(view.machine.revealed.unwrap().first_shown_at.is_some());
        let (at, any) = shown(&state, &revealed, run_a);
        assert!(at.is_some() && any);
        // The second paper's run is untouched until its own save.
        assert_eq!(shown(&state, &saved, run_b), (None, false));
        let answers = [
            (Domain::D1, None, "1.1", Answer::Y),
            (Domain::D1, None, "1.2", Answer::Y),
            (Domain::D1, None, "1.3", Answer::N),
            (Domain::D2, Some(0), "2.1", Answer::N),
            (Domain::D2, Some(0), "2.2", Answer::N),
            (Domain::D2, Some(0), "2.6", Answer::Y),
            (Domain::D3, Some(0), "3.1", Answer::Y),
            (Domain::D4, Some(0), "4.1", Answer::N),
            (Domain::D4, Some(0), "4.2", Answer::N),
            (Domain::D4, Some(0), "4.3", Answer::N),
            (Domain::D5, Some(0), "5.1", Answer::Y),
            (Domain::D5, Some(0), "5.2", Answer::N),
            (Domain::D5, Some(0), "5.3", Answer::N),
        ];
        for (domain, outcome, q, answer) in answers {
            let e = AppraisalEdit::Answer {
                domain,
                outcome,
                question_id: q.into(),
                answer: Some(answer),
            };
            appraisal_store::edit(&mut db, b, &e).unwrap();
        }
        assert_eq!(shown(&state, &saved, run_b), (None, false));
        tauri::async_runtime::block_on(save_appraisal(h.clone(), b)).unwrap();
        let (at, any) = shown(&state, &saved, run_b);
        assert!(at.is_some() && any);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A damaged appraisal leaves the Evidence tab open on its summary, and
    /// can be set aside through the command: the set aside is kept, and only
    /// the view after it reports the damage.
    #[test]
    fn a_damaged_appraisal_can_be_set_aside() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let p = paper(&mut db);
        let id = start_appraisal_with(&s, p.as_str()).unwrap().id;
        bmlib::db::execute(
            &mut db,
            "UPDATE evidence_appraisal_versions SET number = 5",
            &[],
        )
        .unwrap();
        assert_eq!(
            appraisal_with(&s, id, None).unwrap_err().kind,
            ErrorKind::Storage
        );
        let (_app, h) = mock_app(s);
        let tab = crate::evidence_commands::evidence(h.clone(), p.as_str().to_string());
        assert_eq!(
            tauri::async_runtime::block_on(tab)
                .unwrap()
                .appraisal
                .unwrap()
                .id,
            id
        );
        let e = tauri::async_runtime::block_on(set_aside_appraisal(h.clone(), id)).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Storage);
        assert!(
            e.message.starts_with("The appraisal was set aside"),
            "{}",
            e.message
        );
        assert_eq!(appraisal_store::open_appraisal(&mut db, &p).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The paired run's events reach the tab as counts only: a step's label
    /// never, condensing not at all.
    #[test]
    fn progress_events_carry_counts_only() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let p = paper(&mut db);
        let id = start_appraisal_with(&s, p.as_str()).unwrap().id;
        let cid = store::latest_canonical_id(&mut db, &p).unwrap().unwrap();
        evidence_store::create_run(&mut db, &p, cid, "m", Some(id)).unwrap();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let got = sent.clone();
        let events = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(json) = body {
                lock(&got).push(json);
            }
            Ok(())
        });
        let sink = progress_sink(s.connect().unwrap(), id, &events);
        sink(EvidenceEvent::Step {
            label: "Study design".into(),
        });
        sink(EvidenceEvent::Condensed);
        std::fs::remove_dir_all(&dir).unwrap();
        let sent: Vec<serde_json::Value> = lock(&sent)
            .iter()
            .map(|j| serde_json::from_str(j).unwrap())
            .collect();
        assert_eq!(
            sent,
            [serde_json::json!({"kind": "progress", "done": 0, "total": 1})]
        );
    }

    /// A paper without full text starts no appraisal; a mark arrives in
    /// UTF-16 units and is stored in code points, and one ending inside the
    /// astral character is refused; other edits pass unchanged.
    #[test]
    fn start_needs_full_text_and_marks_arrive_in_utf16() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = s.connect().unwrap();
        let bare = store::create_paper(&mut db, &Default::default(), None).unwrap();
        let e = start_appraisal_with(&s, bare.as_str()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFullText);
        let p = paper(&mut db);
        let id = start_appraisal_with(&s, p.as_str()).unwrap().id;
        let t = store::latest_canonical(&mut db, &p)
            .unwrap()
            .unwrap()
            .text
            .text;
        let units: Vec<u16> = t.encode_utf16().collect();
        let twice: Vec<u16> = "twice".encode_utf16().collect();
        let at = units.windows(twice.len()).position(|w| w == twice).unwrap();
        let mark = |start, end| AppraisalEdit::Mark {
            domain: Domain::D1,
            outcome: None,
            question_id: "1.1".into(),
            span: Some(TextSpan { start, end }),
        };
        let start = t[..t.find("twice").unwrap()].chars().count();
        let want = mark(start, start + "twice".len());
        let got = from_webview(&mut db, id, mark(at, at + twice.len())).unwrap();
        assert_eq!(got, want);
        // "😀 " (two units, then one) comes just before "twice".
        let emoji = at - 3;
        let e = from_webview(&mut db, id, mark(emoji + 1, at)).unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::InvalidInput, SELECTION_OUTSIDE)
        );
        let note = AppraisalEdit::Note {
            domain: Domain::D1,
            outcome: None,
            note: Some("n".into()),
        };
        assert_eq!(from_webview(&mut db, id, note.clone()).unwrap(), note);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
