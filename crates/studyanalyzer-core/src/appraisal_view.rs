//! The reader's appraisal as the Evidence weight tab shows it (design
//! 2026-11-26, §3–§7): the stored appraisal and its paired machine run as
//! the serialisable shapes the frontend shows, spans in UTF-16 units. Plan
//! 2's views ([`crate::evidence_view`]) are reused for the machine's side.
//!
//! **Nothing the machine judged about RoB 2 is read before the reveal.**
//! While the appraisal has no `revealed_at`, the machine is shown only as
//! step counts, its failed steps (whose stored messages are neutral) and its
//! design and outcome proposals; its answers, verdicts, ladders and numbers
//! are not even read. Once revealed, the run's own view, its departures and
//! the side-by-side compare are added.
//!
//! The reader's verdicts are shown as stored (derived values are written,
//! not computed on read); only conformance is computed here, and not for a
//! version made under earlier rules.

use bmlib::db::{fetch_all, fetch_one, transaction_with, Db, Value};
use bmlib::quality::rob2_questions::{domain_title, question_text};
use bmlib::quality::{
    allowed_answers, applicable_questions, needs_escalation_decision, questions, Answers, Domain,
    Judgement, ReportedResult,
};
use serde::Serialize;

use crate::appraisal::{
    all_known, complete, departures, effective, Departure, DepartureScope, RaterRecord,
    OUTCOME_DOMAINS,
};
use crate::appraisal_rows::{
    appraisal, design_covered, AppraisalState, ConfirmedOutcome, ReaderAnswer, ReaderOverall,
    StoredAppraisal, StoredVersion,
};
use crate::appraisal_store::rater_record;
use crate::evidence::{step_label, EvidenceEvent, HIDDEN_STEP_LABEL};
use crate::evidence_rows::{self, stored_design, stored_outcomes, wire};
use crate::evidence_store::{
    domain_name, latest_paired_run, run as stored_run, StepStatus, StoredEvidenceRun, StoredStep,
};
use crate::evidence_view::{
    design_view, read_run_view, result_view, DesignView, EvidenceRunView, LadderStepView,
    LadderView, NamedErrorView, QuotesView, ResultView, RunText,
};
use crate::pico_store::{optional, text_of};
use crate::store::{PaperId, StoreError};
use crate::view::{span_to_utf16, utf16_offsets, Utf16Span};

/// A paper's appraisal as the Evidence weight tab's opening view names it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppraisalSummary {
    /// The appraisal's id.
    pub id: i64,
    /// Where it stands.
    pub state: AppraisalState,
    /// Whether it counts as blind.
    pub blind: bool,
    /// Whether it was set aside.
    pub set_aside: bool,
}

/// One signalling question of the reader's version, with what they gave.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuestionView {
    /// RoB 2's number (`2.3`).
    pub question_id: String,
    /// Whether today's rules ask it. A stored answer to a question they no
    /// longer have (after a re-pin) is kept and shown, not counted: such a
    /// row has `known` false, no wording, nothing allowed and is never
    /// applicable, and the tab says *An answer to a question this version of
    /// the rules no longer has.*
    pub known: bool,
    /// The question (`question_text`'s first line); empty when not `known`.
    pub headline: String,
    /// The rest of bmlib's wording: *Look in* and the answer definitions.
    pub details: String,
    /// Whether it applies given the earlier answers that count.
    pub applicable: bool,
    /// The answers it allows (`allowed_answers`, so no `NI` on 3.2).
    pub allowed: Vec<String>,
    /// The reader's answer as stored, counted or not.
    pub answer: Option<String>,
    /// The sentence the answer rests on, in UTF-16 units of the appraisal's
    /// text.
    pub mark: Option<Utf16Span>,
    /// *I also used: protocol*.
    pub protocol: bool,
    /// *I also used: registry*.
    pub registry: bool,
    /// *I also used: supplement*.
    pub supplement: bool,
}

/// A verdict that departs from RoB 2's rules on its own inputs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DepartureView {
    /// The domain (`D1` … `D5`), `None` for an outcome's overall verdict.
    pub domain: Option<String>,
    /// The outcome's index, `None` for D1.
    pub outcome: Option<usize>,
    /// The verdict given (`low`, `some_concerns`, `high`).
    pub given: String,
    /// The verdict the rules give.
    pub rules: String,
}

/// One domain of the reader's version.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReaderDomainView {
    /// The domain (`D1` … `D5`).
    pub domain: String,
    /// Its title (bmlib's `domain_title`).
    pub name: String,
    /// Its questions in RoB 2's order, then any stored answer to a question
    /// the rules no longer have.
    pub questions: Vec<QuestionView>,
    /// The verdict RoB 2's algorithm gave the answers, as stored.
    pub derived: Option<String>,
    /// The reader's own verdict over it.
    #[serde(rename = "override")]
    pub override_: Option<String>,
    /// The verdict as the reader leaves it: the override, else the derived.
    pub verdict: Option<String>,
    /// The reader's note.
    pub note: Option<String>,
    /// The departure of `verdict` from the rules, when the version's
    /// conformance is checked (current rules) and it departs.
    pub departure: Option<DepartureView>,
}

/// One confirmed outcome in the reader's version.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReaderOutcomeView {
    /// Its index among the confirmed outcomes, from 0.
    pub index: usize,
    /// Its confirmed name.
    pub name: String,
    /// When it was measured.
    pub timepoint: Option<String>,
    /// Its confirmed result.
    pub result: Option<ResultView>,
    /// D2 to D5, in order.
    pub domains: Vec<ReaderDomainView>,
    /// Whether the escalation question is asked: every domain has a
    /// verdict, none high, and at least two raise some concerns.
    pub needs_escalation: bool,
    /// The reader's answer to it.
    pub escalate: Option<bool>,
    /// The overall verdict the rules gave, as stored.
    pub derived_overall: Option<String>,
    /// The reader's own overall verdict.
    pub override_overall: Option<String>,
    /// The overall verdict as the reader leaves it.
    pub overall: Option<String>,
    /// The reader's ladder; its weight is their number.
    pub ladder: Option<LadderView>,
    /// Why bmlib built no ladder from the reader's overall verdict.
    pub ladder_error: Option<String>,
    /// The departure of `overall` from the rules, as for a domain.
    pub overall_departure: Option<DepartureView>,
}

/// One version of the reader's record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VersionView {
    /// 0 for the first record, then *after reveal 1*, 2, ….
    pub number: u32,
    /// Whether an edit writes to it now: neither frozen nor closed, in an
    /// appraisal confirmed on a covered design and not set aside.
    pub editable: bool,
    /// When it was frozen (the first record's save).
    pub frozen_at: Option<String>,
    /// When it was closed (*Done revising*).
    pub closed_at: Option<String>,
    /// Whether it was made under earlier rules: shown as made, *Not checked
    /// (earlier rules).*
    pub earlier_rules: bool,
    /// Whether every question that applies, and each escalation asked, is
    /// answered: *Save and compare* is offered.
    pub complete: bool,
    /// D1, judged once for the trial.
    pub study: ReaderDomainView,
    /// The confirmed outcomes, in order.
    pub outcomes: Vec<ReaderOutcomeView>,
}

/// The paired machine run as the appraisal shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MachineView {
    /// The latest paired run, `None` before one is made.
    pub run_id: Option<i64>,
    /// Its steps done.
    pub done_steps: usize,
    /// Its steps planned so far.
    pub total_steps: usize,
    /// Whether another evidence run holds the one slot: *The machine's
    /// assessment waits for another paper's run.*
    pub running_elsewhere: bool,
    /// Its failed steps (or ones set back with their error kept), with their
    /// stored, neutral messages: before the reveal each labelled
    /// `HIDDEN_STEP_LABEL` (which step failed can tell a judgement), after it
    /// by its progress label.
    pub failed: Vec<NamedErrorView>,
    /// Whether it was made under earlier rules: only *Run the machine again
    /// on these outcomes* is offered.
    pub earlier_rules: bool,
    /// Whether it stopped because its outcomes left no room in the window.
    pub too_long: bool,
    /// The run's own view; `Some` only once the appraisal is revealed.
    pub revealed: Option<EvidenceRunView>,
    /// The machine's departures from the rules (a defect); empty unless
    /// revealed and made under the current rules.
    pub departures: Vec<DepartureView>,
}

/// One signalling question side by side.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompareRowView {
    /// The domain (`D1` … `D5`).
    pub domain: String,
    /// RoB 2's number.
    pub question_id: String,
    /// The reader's answer, when it counts (asked under their answers).
    pub reader: Option<String>,
    /// The machine's answer, once its domain is done.
    pub machine: Option<String>,
    /// Whether the two differ.
    pub differs: bool,
}

/// One confirmed outcome's questions side by side: D1's, then its D2–D5's.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompareOutcomeView {
    /// The outcome's index.
    pub index: usize,
    /// Every question either rater answered, in RoB 2's order.
    pub questions: Vec<CompareRowView>,
}

/// The reader's version against the paired run, once revealed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompareView {
    /// Per confirmed outcome.
    pub outcomes: Vec<CompareOutcomeView>,
}

/// One outcome the machine proposed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProposedOutcomeView {
    /// The outcome as the paper names it.
    pub name: String,
    /// When it was measured.
    pub timepoint: Option<String>,
    /// Its reported effect.
    pub result: Option<ResultView>,
    /// Why its reported effect was refused.
    pub result_error: Option<String>,
    /// The quotes naming it.
    pub quotes: QuotesView,
}

/// The machine's proposals, which steps 1 and 2 confirm.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProposalsView {
    /// The proposed design with its quotes.
    pub design: DesignView,
    /// The proposed primary outcomes; empty until their step is done.
    pub outcomes: Vec<ProposedOutcomeView>,
}

/// One outcome as the reader confirmed it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfirmedOutcomeView {
    /// The index of the proposed outcome it came from; `None` for one the
    /// reader added.
    pub from_proposed: Option<usize>,
    /// Its name.
    pub name: String,
    /// When it was measured.
    pub timepoint: Option<String>,
    /// Its result.
    pub result: Option<ResultView>,
}

/// What the reader confirmed. A design not covered is named from here
/// (*No method for this design yet: {design}*), never from the proposal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfirmedView {
    /// The design's bmlib name.
    pub design: String,
    /// A randomised trial's layout.
    pub rct_layout: Option<String>,
    /// The confirmed outcomes, in order.
    pub outcomes: Vec<ConfirmedOutcomeView>,
}

/// One version in the version picker.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VersionSummary {
    /// Its number.
    pub number: u32,
    /// When it was frozen.
    pub frozen_at: Option<String>,
    /// When it was closed.
    pub closed_at: Option<String>,
}

/// Everything the appraisal pane shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppraisalView {
    /// The appraisal's id.
    pub id: i64,
    /// The paper text its spans point into.
    pub canonical_id: i64,
    /// Where it stands.
    pub state: AppraisalState,
    /// Whether it counts as blind.
    pub blind: bool,
    /// When it was started.
    pub started_at: String,
    /// When the machine's verdict was first shown.
    pub revealed_at: Option<String>,
    /// When the first record was saved.
    pub saved_at: Option<String>,
    /// When it was set aside.
    pub set_aside_at: Option<String>,
    /// The machine's design and outcome proposals, once its design is stored.
    pub proposals: Option<ProposalsView>,
    /// What the reader confirmed, once confirmed.
    pub confirmed: Option<ConfirmedView>,
    /// Every version, first record first.
    pub versions: Vec<VersionSummary>,
    /// The version shown.
    pub version: VersionView,
    /// The paired run.
    pub machine: MachineView,
    /// Both raters side by side; `Some` only once revealed.
    pub compare: Option<CompareView>,
}

/// What arrives while an appraisal's paired run works: counts only, never a
/// step's label or anything it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppraisalEvent {
    /// A step has begun: `done` of `total` planned steps are done.
    Progress {
        /// Steps done.
        done: usize,
        /// Steps planned so far.
        total: usize,
    },
}

/// The appraisal event for the paired run's `event`: a step beginning is
/// [`progress`]'s count of appraisal `appraisal_id`'s latest paired run;
/// condensing and any other event is dropped (`None`), so no label reaches
/// the tab.
///
/// # Errors
///
/// What [`progress`] returns.
pub fn appraisal_event(
    db: &mut dyn Db,
    appraisal_id: i64,
    event: &EvidenceEvent,
) -> Result<Option<AppraisalEvent>, StoreError> {
    match event {
        EvidenceEvent::Step { .. } => progress(db, appraisal_id).map(Some),
        EvidenceEvent::Condensing { .. }
        | EvidenceEvent::Condensed
        | EvidenceEvent::Other { .. } => Ok(None),
    }
}

/// The progress of appraisal `appraisal_id`'s latest paired run: its steps
/// done of its steps planned so far (0 of 0 before one is made).
///
/// # Errors
///
/// Any store failure as [`StoreError`].
pub fn progress(db: &mut dyn Db, appraisal_id: i64) -> Result<AppraisalEvent, StoreError> {
    let (done, total) = match latest_paired_run(db, appraisal_id)? {
        Some(run_id) => step_counts(&existing_run(db, run_id)?),
        None => (0, 0),
    };
    Ok(AppraisalEvent::Progress { done, total })
}

/// The summary of `paper`'s newest appraisal, if it has one, read from its
/// `evidence_appraisals` row alone: its state follows from the row's
/// confirmation, save and design (`design_covered`), with no version read,
/// so a damaged version never breaks the tab that opens on it (the
/// `appraisal` command reports that damage, and it can still be set aside).
///
/// # Errors
///
/// A design or layout bmlib does not know, or a column of the wrong type, as
/// [`StoreError::Corrupt`]; any other store failure as [`StoreError`].
pub fn appraisal_summary(
    db: &mut dyn Db,
    paper: &PaperId,
) -> Result<Option<AppraisalSummary>, StoreError> {
    // A paper has at most one open appraisal, and a new one starts only once
    // the last is set aside, so the newest is the open one when there is one.
    let Some(r) = fetch_one(
        db,
        "SELECT id, blind, confirmed_at, saved_at, set_aside_at, design, rct_layout
         FROM evidence_appraisals WHERE paper_id = ? ORDER BY id DESC LIMIT 1",
        &[Value::from(paper.as_str())],
    )?
    else {
        return Ok(None);
    };
    let id = r.get_i64("id")?;
    let bad = |why: String| StoreError::Corrupt(format!("appraisal {id}: {why}"));
    let opt = |name: &str| optional(&r, name, text_of).map_err(bad);
    let covered = match opt("design")? {
        Some(d) => design_covered(&d, opt("rct_layout")?.as_deref()).map_err(bad)?,
        None => true,
    };
    let state = match (opt("confirmed_at")?, opt("saved_at")?) {
        (None, _) => AppraisalState::Confirming,
        (Some(_), _) if !covered => AppraisalState::NotCovered,
        (Some(_), None) => AppraisalState::Answering,
        (Some(_), Some(_)) => AppraisalState::Saved,
    };
    Ok(Some(AppraisalSummary {
        id,
        state,
        blind: r.get_i64("blind")? != 0,
        set_aside: opt("set_aside_at")?.is_some(),
    }))
}

/// Appraisal `appraisal_id` as the pane shows it, read in one transaction:
/// version `version` (`None`: the editable version if there is one, else the
/// first record), its paired run, and `running_elsewhere` (another evidence
/// run holds the slot, as the command knows) passed through.
///
/// # Errors
///
/// [`StoreError::NotFound`] when there is no such appraisal;
/// [`StoreError::Invalid`] when it has no version `version`; a damaged row
/// as [`StoreError::Corrupt`]; any other store failure as [`StoreError`].
pub fn appraisal_view(
    db: &mut dyn Db,
    appraisal_id: i64,
    version: Option<u32>,
    running_elsewhere: bool,
) -> Result<AppraisalView, StoreError> {
    transaction_with(db, |tx| {
        read_view(tx, appraisal_id, version, running_elsewhere)
    })
}

/// [`appraisal_view`]'s reads, on a transaction it opened.
fn read_view(
    db: &mut dyn Db,
    appraisal_id: i64,
    version: Option<u32>,
    running_elsewhere: bool,
) -> Result<AppraisalView, StoreError> {
    let a = existing(db, appraisal_id)?;
    let offsets = utf16_offsets(&a.text.text);
    let v = pick(&a, version)?;
    let revealed = a.revealed_at.is_some();
    let run_id = latest_paired_run(db, appraisal_id)?;
    let run = run_id.map(|id| existing_run(db, id)).transpose()?;
    // Before the reveal nothing the machine judged is even read.
    let shown = match (revealed, run_id) {
        (true, Some(id)) => Some(read_run_view(db, id)?),
        _ => None,
    };
    let machine_departures = match (&shown, &run) {
        (Some(_), Some(r)) if !r.earlier_rules => {
            departures(&evidence_rows::rater_record(db, r.id)?)
                .iter()
                .map(departure_view)
                .collect::<Result<_, _>>()?
        }
        _ => Vec::new(),
    };
    let (done_steps, total_steps) = run.as_ref().map_or((0, 0), step_counts);
    // Before the reveal every step reads alike: which one failed can tell a
    // judgement (an escalation is asked only on some verdicts).
    let label = |s: &StoredStep| {
        if revealed {
            step_label(&s.step, a.outcomes.len())
        } else {
            HIDDEN_STEP_LABEL.to_string()
        }
    };
    let failed = run.as_ref().map_or_else(Vec::new, |r| {
        r.steps
            .iter()
            .filter_map(|s| {
                s.failure.as_ref().map(|f| NamedErrorView {
                    label: label(s),
                    error: f.message.clone(),
                })
            })
            .collect()
    });
    let record = rater_record(v);
    let compare = revealed
        .then(|| compare_view(&a, &record, shown.as_ref()))
        .transpose()?;
    Ok(AppraisalView {
        id: a.id,
        canonical_id: a.canonical_id,
        state: a.state,
        blind: a.blind,
        started_at: a.started_at.clone(),
        revealed_at: a.revealed_at.clone(),
        saved_at: a.saved_at.clone(),
        set_aside_at: a.set_aside_at.clone(),
        proposals: proposals(db, &a, &offsets)?,
        confirmed: confirmed_view(&a)?,
        versions: a
            .versions
            .iter()
            .map(|v| VersionSummary {
                number: v.number,
                frozen_at: v.frozen_at.clone(),
                closed_at: v.closed_at.clone(),
            })
            .collect(),
        version: version_view(&a, v, &record, &offsets)?,
        machine: MachineView {
            run_id,
            done_steps,
            total_steps,
            running_elsewhere,
            failed,
            earlier_rules: run.as_ref().is_some_and(|r| r.earlier_rules),
            too_long: run.as_ref().is_some_and(|r| r.too_long),
            revealed: shown,
            departures: machine_departures,
        },
        compare,
    })
}

/// Appraisal `appraisal_id`, read; no such appraisal is [`StoreError::NotFound`].
fn existing(db: &mut dyn Db, appraisal_id: i64) -> Result<StoredAppraisal, StoreError> {
    appraisal(db, appraisal_id)?
        .ok_or_else(|| StoreError::NotFound(format!("appraisal {appraisal_id}")))
}

/// Run `run_id`, read; no such run is [`StoreError::NotFound`].
fn existing_run(db: &mut dyn Db, run_id: i64) -> Result<StoredEvidenceRun, StoreError> {
    stored_run(db, run_id)?.ok_or_else(|| StoreError::NotFound(format!("evidence run {run_id}")))
}

/// `run`'s steps done and its steps planned so far.
fn step_counts(run: &StoredEvidenceRun) -> (usize, usize) {
    let done = run
        .steps
        .iter()
        .filter(|s| s.status == StepStatus::Done)
        .count();
    (done, run.steps.len())
}

/// Whether version `v` is open to writes: neither frozen nor closed.
fn is_open(v: &StoredVersion) -> bool {
    v.frozen_at.is_none() && v.closed_at.is_none()
}

/// The version of `a` to show: number `version`, or with `None` the open one
/// if there is one, else the first record. A number `a` has no version of is
/// [`StoreError::Invalid`].
fn pick(a: &StoredAppraisal, version: Option<u32>) -> Result<&StoredVersion, StoreError> {
    let found = match version {
        Some(n) => a.versions.iter().find(|v| v.number == n),
        None => a
            .versions
            .last()
            .filter(|v| is_open(v))
            .or(a.versions.first()),
    };
    found.ok_or_else(|| {
        log::info!("appraisal {}: no version {version:?} to show", a.id);
        StoreError::Invalid("That version of the appraisal is not stored.".into())
    })
}

/// The machine's proposals from `a`'s newest paired run with a stored
/// design, its spans in UTF-16 units of `a`'s text (`offsets`); `None` before
/// one is stored. Neither is a RoB 2 judgement, so both are shown before the
/// reveal.
fn proposals(
    db: &mut dyn Db,
    a: &StoredAppraisal,
    offsets: &[usize],
) -> Result<Option<ProposalsView>, StoreError> {
    let rows = fetch_all(
        db,
        "SELECT id FROM evidence_runs WHERE appraisal_id = ? AND canonical_id = ? ORDER BY id DESC",
        &[Value::from(a.id), Value::from(a.canonical_id)],
    )?;
    for r in rows {
        let run_id = r.get_i64("id")?;
        let Some(design) = stored_design(db, run_id)? else {
            continue;
        };
        let text = RunText {
            run_id,
            canonical_id: a.canonical_id,
            text: &a.text,
            offsets: offsets.to_vec(),
        };
        let outcomes = stored_outcomes(db, run_id)?
            .into_iter()
            .map(|o| {
                Ok(ProposedOutcomeView {
                    name: o.name,
                    timepoint: o.timepoint,
                    result: o.result.map(result_view),
                    result_error: o.result_error,
                    quotes: text.stored_quotes(o.quotes)?,
                })
            })
            .collect::<Result<_, StoreError>>()?;
        return Ok(Some(ProposalsView {
            design: design_view(&text, design)?,
            outcomes,
        }));
    }
    Ok(None)
}

/// A confirmed result as shown.
fn reported_view(r: &ReportedResult) -> Result<ResultView, StoreError> {
    Ok(ResultView {
        measure: r.measure.clone(),
        scale: wire(&r.scale)?,
        estimate: r.estimate,
        ci_low: r.ci_low,
        ci_high: r.ci_high,
        ci_level: r.ci_level,
        n_analysed: r.n_analysed,
    })
}

/// What `a`'s reader confirmed, once confirmed.
fn confirmed_view(a: &StoredAppraisal) -> Result<Option<ConfirmedView>, StoreError> {
    let (Some(_), Some(design)) = (&a.confirmed_at, &a.design) else {
        return Ok(None);
    };
    let outcomes = a
        .outcomes
        .iter()
        .map(|o| {
            Ok(ConfirmedOutcomeView {
                from_proposed: o.from_proposed,
                name: o.name.clone(),
                timepoint: o.timepoint.clone(),
                result: o.result.as_ref().map(reported_view).transpose()?,
            })
        })
        .collect::<Result<_, StoreError>>()?;
    Ok(Some(ConfirmedView {
        design: design.clone(),
        rct_layout: a.rct_layout.clone(),
        outcomes,
    }))
}

/// Departure `d` as shown.
fn departure_view(d: &Departure) -> Result<DepartureView, StoreError> {
    let (domain, outcome) = match d.scope {
        DepartureScope::Domain(domain, outcome) => (Some(domain_name(domain).to_string()), outcome),
        DepartureScope::Overall(outcome) => (None, Some(outcome)),
    };
    Ok(DepartureView {
        domain,
        outcome,
        given: wire(&d.given)?,
        rules: wire(&d.rules)?,
    })
}

/// The departure at `scope` among `checked`, as shown.
fn departure_at(
    checked: &[Departure],
    scope: DepartureScope,
) -> Result<Option<DepartureView>, StoreError> {
    checked
        .iter()
        .find(|d| d.scope == scope)
        .map(departure_view)
        .transpose()
}

/// Version `v` of `a` as shown, `record` its [`rater_record`], spans in
/// UTF-16 units (`offsets`). Conformance is checked only under the current
/// rules: a version made under earlier ones shows no departure.
fn version_view(
    a: &StoredAppraisal,
    v: &StoredVersion,
    record: &RaterRecord,
    offsets: &[usize],
) -> Result<VersionView, StoreError> {
    let checked = if v.earlier_rules {
        Vec::new()
    } else {
        departures(record)
    };
    let shown = Shown {
        v,
        record,
        checked: &checked,
        offsets,
    };
    let outcomes = a
        .outcomes
        .iter()
        .map(|o| shown.outcome(o.position, o))
        .collect::<Result<_, _>>()?;
    Ok(VersionView {
        number: v.number,
        editable: is_open(v)
            && a.set_aside_at.is_none()
            && matches!(a.state, AppraisalState::Answering | AppraisalState::Saved),
        frozen_at: v.frozen_at.clone(),
        closed_at: v.closed_at.clone(),
        earlier_rules: v.earlier_rules,
        complete: complete(record, a.outcomes.len()),
        study: shown.domain(Domain::D1, None)?,
        outcomes,
    })
}

/// One version being shown, with what its views read.
struct Shown<'a> {
    /// The version.
    v: &'a StoredVersion,
    /// Its record as completeness and conformance read it.
    record: &'a RaterRecord,
    /// Its departures, empty when not checked.
    checked: &'a [Departure],
    /// [`utf16_offsets`] of the appraisal's text.
    offsets: &'a [usize],
}

impl Shown<'_> {
    /// Question `q` of `domain` with stored row `row`; `known` when today's
    /// rules ask it, `applicable` when they ask it given the earlier answers.
    fn question(
        &self,
        q: &str,
        known: bool,
        applicable: bool,
        row: Option<&ReaderAnswer>,
    ) -> Result<QuestionView, StoreError> {
        // `question_text` and `allowed_answers` panic on an id they lack.
        let (headline, details, allowed) = if known {
            let text = question_text(q);
            let (headline, details) = text.split_once('\n').unwrap_or((text, ""));
            let allowed = allowed_answers(q)
                .iter()
                .map(wire)
                .collect::<Result<_, _>>()?;
            (headline.to_string(), details.to_string(), allowed)
        } else {
            (String::new(), String::new(), Vec::new())
        };
        Ok(QuestionView {
            question_id: q.to_string(),
            known,
            headline,
            details,
            applicable,
            allowed,
            answer: row.and_then(|x| x.answer).map(|a| wire(&a)).transpose()?,
            // The reader's rows hold only marks inside the text (`appraisal_rows`).
            mark: row
                .and_then(|x| x.mark)
                .map(|m| span_to_utf16(self.offsets, m)),
            protocol: row.is_some_and(|x| x.protocol),
            registry: row.is_some_and(|x| x.registry),
            supplement: row.is_some_and(|x| x.supplement),
        })
    }

    /// `domain` of `outcome` (`None`: D1) as shown: today's questions in
    /// RoB 2's order, then any stored answer to one the rules no longer have.
    fn domain(
        &self,
        domain: Domain,
        outcome: Option<usize>,
    ) -> Result<ReaderDomainView, StoreError> {
        let counted = self
            .record
            .domain(domain, outcome)
            .map_or_else(Answers::new, |d| d.answers.clone());
        let asked = applicable_questions(domain, &counted);
        let known = questions(domain);
        let rows: Vec<&ReaderAnswer> = self
            .v
            .answers
            .iter()
            .filter(|x| x.domain == domain && x.outcome == outcome)
            .collect();
        let row = |q: &str| rows.iter().copied().find(|x| x.question_id == q);
        let mut shown = known
            .iter()
            .map(|&q| self.question(q, true, asked.contains(&q), row(q)))
            .collect::<Result<Vec<_>, _>>()?;
        for x in rows
            .iter()
            .filter(|x| !known.contains(&x.question_id.as_str()))
        {
            shown.push(self.question(&x.question_id, false, false, Some(x))?);
        }
        let stored = self
            .v
            .domains
            .iter()
            .find(|d| d.domain == domain && d.outcome == outcome);
        let derived = stored.and_then(|d| d.derived);
        let override_ = stored.and_then(|d| d.override_);
        let name = |j: Option<Judgement>| j.as_ref().map(wire).transpose();
        Ok(ReaderDomainView {
            domain: domain_name(domain).to_string(),
            name: domain_title(domain).to_string(),
            questions: shown,
            derived: name(derived)?,
            override_: name(override_)?,
            verdict: name(effective(derived, override_))?,
            note: stored.and_then(|d| d.note.clone()),
            departure: departure_at(self.checked, DepartureScope::Domain(domain, outcome))?,
        })
    }

    /// Confirmed outcome `o` at `index` as shown: its D2–D5, escalation,
    /// overall verdict and ladder.
    fn outcome(&self, index: usize, o: &ConfirmedOutcome) -> Result<ReaderOutcomeView, StoreError> {
        let domains = OUTCOME_DOMAINS
            .iter()
            .map(|&d| self.domain(d, Some(index)))
            .collect::<Result<_, _>>()?;
        let row = self.v.overalls.iter().find(|r| r.outcome == index);
        let derived = row.and_then(|r| r.derived);
        let override_ = row.and_then(|r| r.override_);
        let name = |j: Option<Judgement>| j.as_ref().map(wire).transpose();
        Ok(ReaderOutcomeView {
            index,
            name: o.name.clone(),
            timepoint: o.timepoint.clone(),
            result: o.result.as_ref().map(reported_view).transpose()?,
            domains,
            needs_escalation: all_known(self.record.outcome_domains(index))
                .is_some_and(|d| needs_escalation_decision(&d)),
            escalate: row.and_then(|r| r.escalate),
            derived_overall: name(derived)?,
            override_overall: name(override_)?,
            overall: name(effective(derived, override_))?,
            ladder: row.map(|r| self.ladder(r)).transpose()?.flatten(),
            ladder_error: row.and_then(|r| r.ladder_error.clone()),
            overall_departure: departure_at(self.checked, DepartureScope::Overall(index))?,
        })
    }

    /// Overall row `r`'s ladder, stored whole or not at all; half stored, or
    /// steps that are not `LadderView.steps`' JSON, is [`StoreError::Corrupt`].
    fn ladder(&self, r: &ReaderOverall) -> Result<Option<LadderView>, StoreError> {
        let bad = |why: String| {
            StoreError::Corrupt(format!(
                "appraisal version {}: outcome {}'s ladder {why}",
                self.v.id, r.outcome
            ))
        };
        match (
            &r.ladder_start,
            &r.ladder_level,
            r.ladder_weight,
            &r.ladder_steps_json,
        ) {
            (None, None, None, None) => Ok(None),
            (Some(start), Some(level), Some(weight), Some(json)) => {
                let steps: Vec<LadderStepView> = serde_json::from_str(json)
                    .map_err(|e| bad(format!("steps are not readable: {e}")))?;
                Ok(Some(LadderView {
                    start: start.clone(),
                    level: level.clone(),
                    weight,
                    steps,
                }))
            }
            _ => Err(bad("is half stored".into())),
        }
    }
}

/// Both raters side by side, per confirmed outcome of `a`: D1's questions,
/// then the outcome's D2–D5's, each in RoB 2's order, wherever either
/// answered. The reader's answer counts only where it is asked under their
/// answers (`record`); the machine's is read from its revealed run view
/// (`machine`, as text, so a run under earlier rules is compared as made).
fn compare_view(
    a: &StoredAppraisal,
    record: &RaterRecord,
    machine: Option<&EvidenceRunView>,
) -> Result<CompareView, StoreError> {
    let machine_answer = |domain: Domain, outcome: Option<usize>, q: &str| {
        let d = match outcome {
            None => machine.and_then(|m| m.study.as_ref()),
            Some(o) => machine
                .and_then(|m| m.outcomes.iter().find(|x| x.index == o))
                .and_then(|x| x.domains.iter().find(|d| d.domain == domain_name(domain))),
        };
        d.and_then(|d| d.answers.iter().find(|x| x.question_id == q))
            .map(|x| x.answer.clone())
    };
    let mut outcomes = Vec::new();
    for o in &a.outcomes {
        let mut rows = Vec::new();
        let scopes = std::iter::once((Domain::D1, None))
            .chain(OUTCOME_DOMAINS.map(|d| (d, Some(o.position))));
        for (domain, outcome) in scopes {
            let counted = record
                .domain(domain, outcome)
                .map_or_else(Answers::new, |d| d.answers.clone());
            let asked = applicable_questions(domain, &counted);
            for &q in questions(domain) {
                let reader = counted
                    .get(q)
                    .filter(|_| asked.contains(&q))
                    .map(wire)
                    .transpose()?;
                let machine = machine_answer(domain, outcome, q);
                if reader.is_none() && machine.is_none() {
                    continue;
                }
                rows.push(CompareRowView {
                    domain: domain_name(domain).to_string(),
                    question_id: q.to_string(),
                    differs: reader != machine,
                    reader,
                    machine,
                });
            }
        }
        outcomes.push(CompareOutcomeView {
            index: o.position,
            questions: rows,
        });
    }
    Ok(CompareView { outcomes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appraisal_store::tests::{answer, input, low_answers};
    use crate::appraisal_store::{confirm, edit, reveal_now, start_appraisal, AppraisalEdit};
    use crate::evidence_store::tests::{assessed, save_pending, save_step};
    use crate::evidence_store::{create_run, pending_steps, EvidenceStep};
    use crate::evidence_view::tests::add_text;
    use crate::identifiers::Identifiers;
    use crate::pico_store::tests::paper_with_text;
    use crate::store::{canonical_by_id, create_paper, open_memory_store};
    use crate::view::span_from_utf16;
    use bmlib::fulltext::canonical::TextSpan;
    use bmlib::quality::{Answer, Judgement, Rob2Step};
    use rusqlite::Connection;

    /// The view of appraisal `aid`'s default version, nothing running elsewhere.
    fn view(c: &mut Connection, aid: i64) -> AppraisalView {
        appraisal_view(c, aid, None, false).unwrap()
    }

    /// A new appraisal of `p` (text `cid`) with its paired run's proposals
    /// stored and `n` outcomes confirmed; returns the appraisal and the run.
    fn confirmed(c: &mut Connection, p: &PaperId, cid: i64, n: usize) -> (i64, i64) {
        let aid = start_appraisal(c, p).unwrap();
        let run = create_run(c, p, cid, "m", Some(aid)).unwrap();
        save_pending(c, p, cid, run, &[]);
        confirm(c, aid, &input(n)).unwrap();
        (aid, run)
    }

    /// `v` as JSON text with every `allowed` list taken out: those are the
    /// rules' answer choices, offered to the reader, not anyone's answer.
    fn judged_text(v: &AppraisalView) -> String {
        fn strip(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Object(m) => {
                    m.remove("allowed");
                    m.values_mut().for_each(strip);
                }
                serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
                _ => {}
            }
        }
        let mut json = serde_json::to_value(v).unwrap();
        strip(&mut json);
        json.to_string()
    }

    /// The JSON names of every judgement and answer.
    const JUDGED: [&str; 8] = [
        "\"low\"",
        "\"some_concerns\"",
        "\"high\"",
        "\"Y\"",
        "\"PY\"",
        "\"PN\"",
        "\"N\"",
        "\"NI\"",
    ];

    /// A hidden paired run with every RoB 2 step done shows the tab only its
    /// counts: no run view, no departure, no compare, and no judgement or
    /// answer anywhere in the view's JSON (the reader has answered nothing, so
    /// any would be the machine's). After the reveal the same check finds
    /// them, so it is not vacuous.
    #[test]
    fn no_machine_verdict_before_reveal() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let (aid, run) = confirmed(&mut c, &p, cid, 2);
        save_pending(&mut c, &p, cid, run, &[]);
        let v = view(&mut c, aid);
        assert_eq!(v.machine.run_id, Some(run));
        assert!(v.machine.total_steps > 2);
        assert_eq!(v.machine.done_steps, v.machine.total_steps);
        assert_eq!(v.machine.revealed, None);
        assert!(v.machine.departures.is_empty());
        assert_eq!(v.compare, None);
        let text = judged_text(&v);
        for judged in JUDGED {
            assert!(!text.contains(judged), "{judged} before the reveal: {text}");
        }
        reveal_now(&mut c, aid).unwrap();
        let v = view(&mut c, aid);
        assert!(v.machine.revealed.is_some() && v.compare.is_some());
        let text = judged_text(&v);
        assert!(text.contains("\"Y\""));
        assert!(JUDGED[..3].iter().any(|j| text.contains(j)));
    }

    /// The paired run's progress is its steps done of those planned, and the
    /// event carries those two numbers and nothing else; a step's label never
    /// reaches the tab, and condensing is dropped.
    #[test]
    fn progress_events_carry_counts_only() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let aid = start_appraisal(&mut c, &p).unwrap();
        let counts = |c: &mut Connection| match progress(c, aid).unwrap() {
            AppraisalEvent::Progress { done, total } => (done, total),
        };
        assert_eq!(counts(&mut c), (0, 0));
        let run = create_run(&mut c, &p, cid, "m", Some(aid)).unwrap();
        // A new run plans its design step; the outcomes step follows it.
        assert_eq!(counts(&mut c), (0, 1));
        save_pending(&mut c, &p, cid, run, &[]);
        assert_eq!(counts(&mut c), (2, 2));
        confirm(&mut c, aid, &input(1)).unwrap();
        let first = pending_steps(&mut c, run).unwrap().remove(0);
        save_step(&mut c, &p, cid, &first);
        let (done, total) = counts(&mut c);
        assert_eq!(done, 3);
        assert!(total > done);
        let step = EvidenceEvent::Step {
            label: "Outcome 1 of 1: the overall judgement".into(),
        };
        let e = appraisal_event(&mut c, aid, &step).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"kind": "progress", "done": done, "total": total})
        );
        assert_eq!(
            appraisal_event(&mut c, aid, &EvidenceEvent::Condensed).unwrap(),
            None
        );
        let condensing = EvidenceEvent::Condensing {
            batch: 1,
            batches: 2,
            level: 0,
        };
        assert_eq!(appraisal_event(&mut c, aid, &condensing).unwrap(), None);
    }

    /// Compare puts each question's answers side by side and marks a
    /// difference: the reader's 2.1 Y against the machine's PN differs, their
    /// shared 1.1 Y does not.
    #[test]
    fn compare_marks_differences() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let (aid, run) = confirmed(&mut c, &p, cid, 1);
        let d2 = EvidenceStep::Rob2(Rob2Step::Domain {
            domain: Domain::D2,
            outcome: Some(0),
        });
        save_pending(&mut c, &p, cid, run, &[d2]);
        let step = pending_steps(&mut c, run).unwrap().remove(0);
        assert_eq!(step.step, d2);
        crate::evidence_store::save_domain(
            &mut c,
            step.id,
            &assessed(Domain::D2, Answer::Pn, None),
        )
        .unwrap();
        for e in low_answers(1) {
            edit(&mut c, aid, &e).unwrap();
        }
        edit(&mut c, aid, &answer(Domain::D2, Some(0), "2.1", Answer::Y)).unwrap();
        reveal_now(&mut c, aid).unwrap();
        let compare = view(&mut c, aid).compare.unwrap();
        let rows = &compare.outcomes[0].questions;
        let row = |domain: &str, q: &str| {
            rows.iter()
                .find(|r| r.domain == domain && r.question_id == q)
                .unwrap()
                .clone()
        };
        let r = row("D2", "2.1");
        assert_eq!(
            (r.reader.as_deref(), r.machine.as_deref()),
            (Some("Y"), Some("PN"))
        );
        assert!(r.differs);
        let r = row("D1", "1.1");
        assert_eq!(
            (r.reader.as_deref(), r.machine.as_deref()),
            (Some("Y"), Some("Y"))
        );
        assert!(!r.differs);
    }

    /// One override is one departure: D1 overridden to high departs from the
    /// rules on D1 only; the overall verdict follows the override, so it
    /// conforms, and nothing else is flagged.
    #[test]
    fn a_departure_is_shown_once() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let (aid, _) = confirmed(&mut c, &p, cid, 1);
        for e in low_answers(1) {
            edit(&mut c, aid, &e).unwrap();
        }
        let high = AppraisalEdit::Override {
            domain: Domain::D1,
            outcome: None,
            judgement: Some(Judgement::High),
        };
        edit(&mut c, aid, &high).unwrap();
        let v = view(&mut c, aid).version;
        let want = DepartureView {
            domain: Some("D1".into()),
            outcome: None,
            given: "high".into(),
            rules: "low".into(),
        };
        assert_eq!(v.study.departure, Some(want));
        assert_eq!(v.study.verdict.as_deref(), Some("high"));
        let o = &v.outcomes[0];
        assert_eq!(o.overall.as_deref(), Some("high"));
        assert_eq!(o.overall_departure, None);
        assert!(o.domains.iter().all(|d| d.departure.is_none()));
    }

    /// A mark after astral characters is shown in UTF-16 units that slice
    /// the paper to its sentence, and `span_from_utf16` takes it back to the
    /// stored code points; a span ending inside a surrogate pair, or past
    /// the text, is `None`.
    #[test]
    fn spans_are_utf16() {
        const NEEDLE: &str = "Body 😀 text.";
        let mut c = open_memory_store().unwrap();
        let p = create_paper(&mut c, &Identifiers::default(), Some("Trial")).unwrap();
        let md = "# Trial\n\nPara one.\n\nPara two.\n\n## Méthods 𝛼\n\nBody 😀 text.\n";
        let cid = add_text(&mut c, &p, md);
        let (aid, _) = confirmed(&mut c, &p, cid, 1);
        let t = canonical_by_id(&mut c, &p, cid).unwrap().unwrap().text;
        let start = t.text[..t.text.find(NEEDLE).unwrap()].chars().count();
        let span = TextSpan {
            start,
            end: start + NEEDLE.chars().count(),
        };
        let mark = AppraisalEdit::Mark {
            domain: Domain::D1,
            outcome: None,
            question_id: "1.1".into(),
            span: Some(span),
        };
        edit(&mut c, aid, &mark).unwrap();
        let v = view(&mut c, aid);
        let q = v
            .version
            .study
            .questions
            .iter()
            .find(|q| q.question_id == "1.1");
        let m = q.unwrap().mark.unwrap();
        assert_ne!(m.start, span.start);
        let units: Vec<u16> = t.text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[m.start..m.end]).unwrap(), NEEDLE);
        let offsets = utf16_offsets(&t.text);
        assert_eq!(span_from_utf16(&offsets, m), Some(span));
        let emoji = offsets[start + "Body ".chars().count()];
        let split = Utf16Span {
            start: m.start,
            end: emoji + 1,
        };
        assert_eq!(span_from_utf16(&offsets, split), None);
        let past = Utf16Span {
            start: m.start,
            end: offsets[offsets.len() - 1] + 1,
        };
        assert_eq!(span_from_utf16(&offsets, past), None);
    }

    /// The tab's opening view names the paper's newest appraisal and whether
    /// a run was shown: none, then the open one, then the set-aside one.
    #[test]
    fn the_evidence_view_names_the_newest_appraisal() {
        let mut c = open_memory_store().unwrap();
        let (p, _) = paper_with_text(&mut c);
        let v = crate::evidence_view::evidence_view(&mut c, &p).unwrap();
        assert_eq!((v.appraisal, v.any_shown), (None, false));
        let aid = start_appraisal(&mut c, &p).unwrap();
        let summary = |c: &mut Connection| appraisal_summary(c, &p).unwrap().unwrap();
        let want = AppraisalSummary {
            id: aid,
            state: AppraisalState::Confirming,
            blind: true,
            set_aside: false,
        };
        assert_eq!(summary(&mut c), want);
        let v = crate::evidence_view::evidence_view(&mut c, &p).unwrap();
        assert_eq!(v.appraisal, Some(want));
        crate::appraisal_store::set_aside(&mut c, aid).unwrap();
        assert!(summary(&mut c).set_aside);
    }

    /// A damaged version does not break the tab's opening view: the summary
    /// is read from the appraisal's row, while the appraisal's own view
    /// reports the damage, and it can still be set aside.
    #[test]
    fn a_damaged_version_leaves_the_tab_open() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let (aid, _) = confirmed(&mut c, &p, cid, 1);
        bmlib::db::execute(
            &mut c,
            "UPDATE evidence_appraisal_versions SET number = 5",
            &[],
        )
        .unwrap();
        assert!(matches!(
            appraisal_view(&mut c, aid, None, false),
            Err(StoreError::Corrupt(_))
        ));
        let v = crate::evidence_view::evidence_view(&mut c, &p).unwrap();
        let summary = v.appraisal.unwrap();
        assert_eq!(
            (summary.id, summary.state),
            (aid, AppraisalState::Answering)
        );
        crate::appraisal_store::set_aside(&mut c, aid).unwrap();
        assert!(appraisal_summary(&mut c, &p).unwrap().unwrap().set_aside);
    }

    /// An answer to a question today's rules lack is shown after D1's
    /// questions as not known, with no wording and nothing allowed, and not
    /// applicable; asking for a version not stored is refused.
    #[test]
    fn an_unknown_question_is_shown_as_not_known() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let (aid, _) = confirmed(&mut c, &p, cid, 1);
        edit(&mut c, aid, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
        bmlib::db::execute(
            &mut c,
            "INSERT INTO evidence_appraisal_answers (version_id, domain, question_id, answer)
             SELECT id, 'D1', '1.9', 'NI' FROM evidence_appraisal_versions",
            &[],
        )
        .unwrap();
        let v = view(&mut c, aid).version;
        let q = v.study.questions.last().unwrap();
        assert_eq!(q.question_id, "1.9");
        assert!(!q.known && !q.applicable && q.headline.is_empty() && q.allowed.is_empty());
        assert_eq!(q.answer.as_deref(), Some("NI"));
        let first = &v.study.questions[0];
        assert!(first.known && first.applicable && !first.headline.is_empty());
        assert_eq!(first.allowed, ["Y", "PY", "PN", "N", "NI"]);
        assert!(matches!(
            appraisal_view(&mut c, aid, Some(3), false),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(
            appraisal_view(&mut c, aid, Some(0), true)
                .unwrap()
                .version
                .number,
            0
        );
    }
}
