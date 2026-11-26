//! Evidence weight runs on one paper (design 2026-11-24, §5–§6), stored step
//! by step, so a run that is stopped or fails keeps what it finished and
//! Continue runs only what is pending. A run starts with one pending `design`
//! step. Saving the design adds the `outcomes` step, or ends the run as *not
//! covered*; saving the outcomes adds RoB 2's plan (`plan_rob2`), or ends it
//! as *no primary outcome*. A run is on one canonical text, so every stored
//! span points into it.
//!
//! A *paired* run (design 2026-11-26, §4) belongs to a reader's appraisal: it
//! proposes both the design and the outcomes (even on a design not covered),
//! then waits, with no kind, until the appraisal's confirmation plans it
//! (`plan_paired`). A re-paired run (`create_repaired_run`) has neither step
//! and starts assessed. Its RoB 2 steps assess the confirmed outcomes, and it
//! is hidden ([`is_hidden`]) until the appraisal is revealed.
//!
//! This module keeps the runs and their steps; the steps' results are rows
//! of their own ([`crate::evidence_rows`]).

use std::collections::BTreeSet;

use bmlib::db::{execute, fetch_all, fetch_one, transaction_with, Db, Row, Value};
use bmlib::fulltext::canonical::CanonicalText;
use bmlib::quality::rob2_questions::ROB2_PROMPT_VERSION;
use bmlib::quality::{
    plan_rob2, start_level, AssessedOutcome, DesignProposal, Domain, DomainResult, LadderSettings,
    OutcomeProposal, RctLayout, Rob2Escalation, Rob2Step, StudyDesign, DESIGN_PROMPT_VERSION,
    LADDER_VERSION, OUTCOME_PROMPT_VERSION, ROB2_ALGORITHM_VERSION,
};
use serde::Serialize;

use crate::appraisal_rows::{appraisal, AppraisalState};
use crate::evidence_rows::{
    delete_results, insert_design, insert_domain, insert_outcome, insert_outcome_result,
    rob2_outcomes,
};
use crate::pico_store::{optional, text_of};
use crate::prisma_store::vocabulary;
pub use crate::prisma_store::RunStatus;
use crate::store::{canonical_by_id, last_id, PaperId, StoreError};

// The stored vocabulary below mirrors the `CHECK` constraints of migration 15
// (`schema.rs`); it is the schema's, not a setting, so it lives here.

vocabulary! {
    /// Where a step stands, as `evidence_steps.status` stores it.
    StepStatus {
        /// Not run yet, or set back by [`reset_for_continue`]: Continue runs it.
        Pending = "pending",
        /// Run and stored, with its result.
        Done = "done",
        /// Its call failed; it holds no result, except a failed escalation,
        /// which keeps its outcome's row ([`save_escalation`]).
        Failed = "failed",
    }
}

vocabulary! {
    /// What a run found, as `evidence_runs.kind` stores it once it is known.
    RunKind {
        /// The design is not a parallel-group randomised trial.
        NotCovered = "not_covered",
        /// The paper names no primary outcome.
        NoPrimaryOutcome = "no_primary_outcome",
        /// Its primary outcomes are taken through RoB 2 and the ladder.
        Assessed = "assessed",
    }
}

/// The position of a run's design step, its first.
const DESIGN_POSITION: usize = 0;

/// The position of a run's outcomes step, after the design.
const OUTCOMES_POSITION: usize = 1;

/// The position of RoB 2's first step, after the outcomes.
const FIRST_ROB2_POSITION: usize = 2;

/// One step of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceStep {
    /// The study design (`DesignAgent::propose`).
    Design,
    /// The primary outcomes (`OutcomeAgent::propose`).
    Outcomes,
    /// One step of RoB 2's plan: a domain or an outcome's escalation.
    Rob2(Rob2Step),
}

/// Why a step failed, as it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFailure {
    /// The failure's message, as the tab shows it.
    pub message: String,
    /// Whether sending the failed request again may succeed.
    pub retryable: bool,
    /// The HTTP status the failed request was answered with, if any.
    pub status: Option<u16>,
}

/// A step still to run, as [`pending_steps`] gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStep {
    /// The step's row id.
    pub id: i64,
    /// What it is.
    pub step: EvidenceStep,
}

/// The rules a run was made under: the prompt and algorithm versions, the
/// ladder's version and its settings. A run whose versions are not
/// [`RunVersions::current`] was made under earlier rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunVersions {
    /// The design step's wording (`DESIGN_PROMPT_VERSION`).
    pub design_prompt: u32,
    /// The outcome step's wording (`OUTCOME_PROMPT_VERSION`).
    pub outcome_prompt: u32,
    /// RoB 2's wording (`ROB2_PROMPT_VERSION`).
    pub rob2_prompt: u32,
    /// RoB 2's decision rules (`ROB2_ALGORITHM_VERSION`).
    pub algorithm: String,
    /// The ladder's rules (`LADDER_VERSION`).
    pub ladder: String,
    /// The ladder's numbers, as JSON.
    pub ladder_settings_json: String,
}

impl RunVersions {
    /// The rules of the pinned bmlib, with the default ladder settings: what
    /// a new run is made under.
    pub fn current() -> Self {
        RunVersions {
            design_prompt: DESIGN_PROMPT_VERSION,
            outcome_prompt: OUTCOME_PROMPT_VERSION,
            rob2_prompt: ROB2_PROMPT_VERSION,
            algorithm: ROB2_ALGORITHM_VERSION.to_string(),
            ladder: LADDER_VERSION.to_string(),
            // Whole numbers and floats only: serde_json cannot fail on them.
            ladder_settings_json: serde_json::to_string(&LadderSettings::default())
                .expect("ladder settings serialise"),
        }
    }
}

/// One stored step of a run, whatever its status.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredStep {
    /// The step's row id.
    pub id: i64,
    /// What it is.
    pub step: EvidenceStep,
    /// Where it stands.
    pub status: StepStatus,
    /// Whether its paper was condensed: set on a done design, outcomes or
    /// domain step, never on an escalation.
    pub condensed: Option<bool>,
    /// Why it failed: always set on a failed step, and kept on one set back
    /// to pending by [`reset_for_continue`] until it is saved.
    pub failure: Option<StepFailure>,
}

/// A stored run, as [`run`] reads it back.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvidenceRun {
    /// The run's row id.
    pub id: i64,
    /// The canonical text it runs on.
    pub canonical_id: i64,
    /// The model that ran it.
    pub model: String,
    /// When it was started (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// When the tab first showed it with a RoB 2 result ([`mark_shown`]).
    pub first_shown_at: Option<String>,
    /// Where it stands.
    pub status: RunStatus,
    /// What it found, once the design (or the outcomes) said.
    pub kind: Option<RunKind>,
    /// The rules it was made under.
    pub versions: RunVersions,
    /// Whether those are not [`RunVersions::current`].
    pub earlier_rules: bool,
    /// Whether an attempt stopped because its stored outcomes left no room
    /// for the paper in its window, with the context-window setting unchanged
    /// since ([`mark_too_long`], [`clear_too_long`]).
    pub too_long: bool,
    /// Every step, in plan order.
    pub steps: Vec<StoredStep>,
    /// The canonical text it runs on, as read to check it is stored.
    pub text: CanonicalText,
    /// The appraisal it is paired with, `None` for an unpaired run.
    pub appraisal: Option<i64>,
}

/// One line of a paper's list of evidence runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceRunSummary {
    /// The run's row id.
    pub id: i64,
    /// The canonical text it ran on.
    pub canonical_id: i64,
    /// The model that ran it.
    pub model: String,
    /// When it was started (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// Where it stands.
    pub status: RunStatus,
}

/// A RoB 2 domain's stored name (`D1` … `D5`).
pub(crate) fn domain_name(domain: Domain) -> &'static str {
    match domain {
        Domain::D1 => "D1",
        Domain::D2 => "D2",
        Domain::D3 => "D3",
        Domain::D4 => "D4",
        Domain::D5 => "D5",
    }
}

/// The domain stored as `name`, or `None`.
pub(crate) fn domain_of(name: &str) -> Option<Domain> {
    Domain::ALL.into_iter().find(|d| domain_name(*d) == name)
}

/// `evidence_steps.kind` of the design step.
pub(crate) const STEP_DESIGN: &str = "design";
/// `evidence_steps.kind` of the outcomes step.
pub(crate) const STEP_OUTCOMES: &str = "outcomes";
/// `evidence_steps.kind` of a RoB 2 domain step.
pub(crate) const STEP_DOMAIN: &str = "domain";
/// `evidence_steps.kind` of an outcome's overall judgement (escalation) step.
pub(crate) const STEP_ESCALATION: &str = "escalation";

/// A step as `evidence_steps` stores it: its kind, domain and outcome index.
fn step_columns(step: EvidenceStep) -> (&'static str, Option<&'static str>, Option<usize>) {
    match step {
        EvidenceStep::Design => (STEP_DESIGN, None, None),
        EvidenceStep::Outcomes => (STEP_OUTCOMES, None, None),
        EvidenceStep::Rob2(Rob2Step::Domain { domain, outcome }) => {
            (STEP_DOMAIN, Some(domain_name(domain)), outcome)
        }
        EvidenceStep::Rob2(Rob2Step::Escalation { outcome }) => {
            (STEP_ESCALATION, None, Some(outcome))
        }
    }
}

/// The step stored as `kind`, `domain` and `outcome`; a combination no plan
/// holds (D1 with an outcome, D2–D5 without, an escalation with a domain…)
/// is an error.
fn step_of(kind: &str, domain: Option<&str>, outcome: Option<i64>) -> Result<EvidenceStep, String> {
    let outcome = outcome
        .map(|o| usize::try_from(o).map_err(|_| format!("outcome {o} is not an index")))
        .transpose()?;
    let domain = domain
        .map(|d| domain_of(d).ok_or_else(|| format!("unknown domain {d:?}")))
        .transpose()?;
    match (kind, domain, outcome) {
        (STEP_DESIGN, None, None) => Ok(EvidenceStep::Design),
        (STEP_OUTCOMES, None, None) => Ok(EvidenceStep::Outcomes),
        (STEP_DOMAIN, Some(domain), outcome) if (domain == Domain::D1) == outcome.is_none() => {
            Ok(EvidenceStep::Rob2(Rob2Step::Domain { domain, outcome }))
        }
        (STEP_ESCALATION, None, Some(outcome)) => {
            Ok(EvidenceStep::Rob2(Rob2Step::Escalation { outcome }))
        }
        _ => Err(format!(
            "a {kind:?} step with domain {domain:?} and outcome {outcome:?} is no plan's"
        )),
    }
}

/// Every step of a run with `outcomes` primary outcomes, in order: the
/// design, the outcomes, then RoB 2's plan.
fn plan_of(outcomes: usize) -> Vec<EvidenceStep> {
    [EvidenceStep::Design, EvidenceStep::Outcomes]
        .into_iter()
        .chain(plan_rob2(outcomes).into_iter().map(EvidenceStep::Rob2))
        .collect()
}

/// The columns [`step_from`] reads.
pub(crate) const STEP_COLUMNS: &str = "id, run_id, position, kind, domain, outcome, status, \
    condensed, error_message, error_retryable, error_status";

/// One step row (of [`STEP_COLUMNS`]) read back, its shape checked: a known
/// step and status, an error on a failed step and none on a done one, and
/// `condensed` exactly on a done step that is not an escalation.
pub(crate) fn step_from(r: &Row) -> Result<StoredStep, StoreError> {
    let id = r.get_i64("id")?;
    let bad = |why: String| StoreError::Corrupt(format!("evidence step {id}: {why}"));
    let step = step_of(
        r.get_str("kind")?,
        optional(r, "domain", text_of).map_err(bad)?.as_deref(),
        optional(r, "outcome", Value::as_i64).map_err(bad)?,
    )
    .map_err(bad)?;
    let status = r.get_str("status")?;
    let status =
        StepStatus::parse(status).ok_or_else(|| bad(format!("unknown status {status:?}")))?;
    let failure = match (
        optional(r, "error_message", text_of).map_err(bad)?,
        optional(r, "error_retryable", Value::as_i64).map_err(bad)?,
        optional(r, "error_status", Value::as_i64).map_err(bad)?,
    ) {
        (None, None, None) => None,
        (Some(message), Some(retryable), status) => Some(StepFailure {
            message,
            retryable: retryable != 0,
            status: status
                .map(|s| u16::try_from(s).map_err(|_| bad(format!("{s} is not an HTTP status"))))
                .transpose()?,
        }),
        _ => return Err(bad("its error is half stored".into())),
    };
    match (status, &failure) {
        (StepStatus::Failed, None) => {
            return Err(bad("it failed, but its error is not stored".into()))
        }
        (StepStatus::Done, Some(_)) => return Err(bad("it holds an error, but it is done".into())),
        _ => {}
    }
    let condensed = optional(r, "condensed", Value::as_i64)
        .map_err(bad)?
        .map(|c| c != 0);
    let escalation = matches!(step, EvidenceStep::Rob2(Rob2Step::Escalation { .. }));
    if condensed.is_some() != (status == StepStatus::Done && !escalation) {
        return Err(bad(format!("{} condensation stored", status.as_str())));
    }
    Ok(StoredStep {
        id,
        step,
        status,
        condensed,
        failure,
    })
}

/// Step `step_id` and the run it belongs to, or `None` if there is none.
pub(crate) fn step_by_id(
    db: &mut dyn Db,
    step_id: i64,
) -> Result<Option<(i64, StoredStep)>, StoreError> {
    let Some(r) = fetch_one(
        db,
        &format!("SELECT {STEP_COLUMNS} FROM evidence_steps WHERE id = ?"),
        &[Value::from(step_id)],
    )?
    else {
        return Ok(None);
    };
    Ok(Some((r.get_i64("run_id")?, step_from(&r)?)))
}

/// Every step of run `run_id`, in position order.
fn steps(db: &mut dyn Db, run_id: i64) -> Result<Vec<StoredStep>, StoreError> {
    fetch_all(
        db,
        &format!("SELECT {STEP_COLUMNS} FROM evidence_steps WHERE run_id = ? ORDER BY position"),
        &[Value::from(run_id)],
    )?
    .iter()
    .map(step_from)
    .collect()
}

/// Inserts `step` at `position` of run `run_id`, pending, and returns its id.
fn insert_step(
    db: &mut dyn Db,
    run_id: i64,
    position: usize,
    step: EvidenceStep,
) -> Result<i64, StoreError> {
    let (kind, domain, outcome) = step_columns(step);
    execute(
        db,
        "INSERT INTO evidence_steps (run_id, position, kind, domain, outcome, status)
         VALUES (?, ?, ?, ?, ?, ?)",
        &[
            Value::from(run_id),
            Value::from(position),
            Value::from(kind),
            Value::from(domain),
            Value::from(outcome),
            Value::from(StepStatus::Pending.as_str()),
        ],
    )?;
    last_id(db)
}

/// Starts a run on canonical text `canonical_id` of `paper` by `model`: an
/// `incomplete` run under [`RunVersions::current`], with one pending design
/// step, paired with `appraisal` when one is given (it then waits after its
/// proposals, [`save_outcomes`]). Returns its id. A text that is not
/// `paper`'s, or an appraisal not on that text, is [`StoreError::Corrupt`],
/// and nothing is written.
pub fn create_run(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
    model: &str,
    appraisal: Option<i64>,
) -> Result<i64, StoreError> {
    if canonical_by_id(db, paper, canonical_id)?.is_none() {
        return Err(StoreError::Corrupt(format!(
            "canonical text {canonical_id} is not paper {}'s",
            paper.as_str()
        )));
    }
    if let Some(a) = appraisal {
        let on_text = fetch_one(
            db,
            "SELECT 1 AS one FROM evidence_appraisals
             WHERE id = ? AND paper_id = ? AND canonical_id = ?",
            &[
                Value::from(a),
                Value::from(paper.as_str()),
                Value::from(canonical_id),
            ],
        )?
        .is_some();
        if !on_text {
            return Err(StoreError::Corrupt(format!(
                "appraisal {a} is not on canonical text {canonical_id} of paper {}",
                paper.as_str()
            )));
        }
    }
    transaction_with(db, |tx| {
        let run_id = insert_run(tx, paper, canonical_id, model, appraisal)?;
        insert_step(tx, run_id, DESIGN_POSITION, EvidenceStep::Design)?;
        Ok(run_id)
    })
}

/// Inserts an `incomplete` run on canonical text `canonical_id` of `paper`
/// by `model` under [`RunVersions::current`], paired with `appraisal` if
/// given, with no step, and returns its id.
fn insert_run(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
    model: &str,
    appraisal: Option<i64>,
) -> Result<i64, StoreError> {
    let v = RunVersions::current();
    execute(
        db,
        "INSERT INTO evidence_runs (paper_id, canonical_id, model, design_prompt_version,
                                    outcome_prompt_version, rob2_prompt_version,
                                    algorithm_version, ladder_version, ladder_settings_json,
                                    status, appraisal_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::from(paper.as_str()),
            Value::from(canonical_id),
            Value::from(model),
            Value::from(i64::from(v.design_prompt)),
            Value::from(i64::from(v.outcome_prompt)),
            Value::from(i64::from(v.rob2_prompt)),
            Value::from(v.algorithm.as_str()),
            Value::from(v.ladder.as_str()),
            Value::from(v.ladder_settings_json.as_str()),
            Value::from(RunStatus::Incomplete.as_str()),
            Value::from(appraisal),
        ],
    )?;
    last_id(db)
}

/// Starts a re-paired run of appraisal `appraisal_id` by `model` (*Run the
/// machine again on these outcomes*): on the appraisal's text, under
/// [`RunVersions::current`], with no design or outcomes step of its own. It
/// is *assessed* from the start, `plan_rob2(n)`'s steps pending from
/// `FIRST_ROB2_POSITION` for the `n` confirmed outcomes, which its RoB 2
/// steps read ([`crate::evidence_rows::rob2_outcomes`]). Returns its id. An
/// appraisal not confirmed, or confirmed on a design RoB 2 does not cover, is
/// [`StoreError::Invalid`], and nothing is written; no such appraisal is
/// [`StoreError::NotFound`].
pub fn create_repaired_run(
    db: &mut dyn Db,
    appraisal_id: i64,
    model: &str,
) -> Result<i64, StoreError> {
    transaction_with(db, |tx| {
        let a = appraisal(tx, appraisal_id)?
            .ok_or_else(|| StoreError::NotFound(format!("appraisal {appraisal_id}")))?;
        if !matches!(a.state, AppraisalState::Answering | AppraisalState::Saved) {
            log::info!(
                "appraisal {appraisal_id}: a re-paired run refused, it is {:?}",
                a.state
            );
            return Err(StoreError::Invalid(
                "The machine runs again only on confirmed outcomes of a design RoB 2 covers."
                    .into(),
            ));
        }
        let run_id = insert_run(tx, &a.paper, a.canonical_id, model, Some(appraisal_id))?;
        plan_assessed(tx, run_id, a.outcomes.len())?;
        log::info!(
            "appraisal {appraisal_id}: run {run_id} made again on its {} outcome(s)",
            a.outcomes.len()
        );
        Ok(run_id)
    })
}

/// The run's pending steps in plan order: what is left to run.
pub fn pending_steps(db: &mut dyn Db, run_id: i64) -> Result<Vec<PendingStep>, StoreError> {
    Ok(steps(db, run_id)?
        .into_iter()
        .filter(|s| s.status == StepStatus::Pending)
        .map(|s| PendingStep {
            id: s.id,
            step: s.step,
        })
        .collect())
}

/// What a save needs of the run a pending step belongs to.
struct PendingRun {
    /// The run's id.
    run_id: i64,
    /// The step, which is pending.
    step: EvidenceStep,
    /// The rules the run is made under.
    versions: RunVersions,
    /// The run's canonical text, which every citation must fall inside.
    text: CanonicalText,
    /// Whether the run is paired with an appraisal.
    paired: bool,
}

/// Step `step_id`, which must be pending, and its run: a step that is done
/// or failed is never saved again, except after [`reset_for_continue`]. No
/// such step is [`StoreError::NotFound`]; one not pending, or a run whose
/// text is not stored, is [`StoreError::Corrupt`].
fn pending_step(db: &mut dyn Db, step_id: i64) -> Result<PendingRun, StoreError> {
    let (run_id, s) = step_by_id(db, step_id)?
        .ok_or_else(|| StoreError::NotFound(format!("evidence step {step_id}")))?;
    if s.status != StepStatus::Pending {
        return Err(StoreError::Corrupt(format!(
            "evidence step {step_id} is {}, not pending",
            s.status.as_str()
        )));
    }
    let r = fetch_one(
        db,
        &format!(
            "SELECT paper_id, canonical_id, appraisal_id, {VERSION_COLUMNS}
             FROM evidence_runs WHERE id = ?"
        ),
        &[Value::from(run_id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("evidence run {run_id}")))?;
    let paper = PaperId(r.get_str("paper_id")?.to_string());
    let canonical_id = r.get_i64("canonical_id")?;
    let text = canonical_by_id(db, &paper, canonical_id)?
        .ok_or_else(|| {
            StoreError::Corrupt(format!("evidence run {run_id}: its text is not stored"))
        })?
        .text;
    Ok(PendingRun {
        run_id,
        step: s.step,
        versions: versions_from(&r, run_id)?,
        text,
        paired: !r.get("appraisal_id")?.is_null(),
    })
}

/// Refuses to save step `step_id` as `wanted` when it is `got`.
fn expect_step(step_id: i64, got: EvidenceStep, wanted: &str) -> Result<(), StoreError> {
    Err(StoreError::Corrupt(format!(
        "evidence step {step_id} is {got:?}, not {wanted}"
    )))
}

/// Marks step `step_id` done with `condensed`, clearing any error an earlier
/// attempt left.
fn set_done(db: &mut dyn Db, step_id: i64, condensed: Option<bool>) -> Result<(), StoreError> {
    execute(
        db,
        "UPDATE evidence_steps SET status = ?, condensed = ?, error_message = NULL,
                                   error_retryable = NULL, error_status = NULL
         WHERE id = ?",
        &[
            Value::from(StepStatus::Done.as_str()),
            Value::from(condensed),
            Value::from(step_id),
        ],
    )?;
    Ok(())
}

/// Marks step `step_id` failed with `failure`.
fn set_failed(db: &mut dyn Db, step_id: i64, failure: &StepFailure) -> Result<(), StoreError> {
    execute(
        db,
        "UPDATE evidence_steps SET status = ?, condensed = NULL, error_message = ?,
                                   error_retryable = ?, error_status = ?
         WHERE id = ?",
        &[
            Value::from(StepStatus::Failed.as_str()),
            Value::from(failure.message.as_str()),
            Value::from(failure.retryable),
            Value::from(failure.status.map(i64::from)),
            Value::from(step_id),
        ],
    )?;
    Ok(())
}

/// Sets run `run_id`'s kind, and its status `complete` when `complete`.
fn set_kind(db: &mut dyn Db, run_id: i64, kind: RunKind, complete: bool) -> Result<(), StoreError> {
    let status = if complete {
        RunStatus::Complete
    } else {
        RunStatus::Incomplete
    };
    execute(
        db,
        "UPDATE evidence_runs SET kind = ?, status = ? WHERE id = ?",
        &[
            Value::from(kind.as_str()),
            Value::from(status.as_str()),
            Value::from(run_id),
        ],
    )?;
    Ok(())
}

/// Refuses a proposal asked with `got`'s wording on a run made with `run`'s.
fn same_version(what: &str, got: u32, run: u32) -> Result<(), StoreError> {
    if got == run {
        return Ok(());
    }
    Err(StoreError::Corrupt(format!(
        "the {what} were asked with prompt version {got}, the run with {run}"
    )))
}

/// Whether `design` with `layout` is one RoB 2's parallel-group questions
/// cover, as `assess_unattended` decides: the design has a ladder start
/// (`start_level`) and its layout is covered (`RctLayout::is_covered`). An
/// unpaired run's design and a reader's confirmed one are judged by it alike.
pub(crate) fn covered(design: StudyDesign, layout: Option<RctLayout>) -> bool {
    start_level(design).is_some() && layout.is_some_and(RctLayout::is_covered)
}

/// Stores pending design step `step_id`'s `proposal` in one transaction: its
/// row and citations, the step as done, then the outcomes step, or the run as
/// *not covered* and complete. It is covered as `assess_unattended` decides:
/// the design has a ladder start (`start_level`) and its layout is covered
/// (`RctLayout::is_covered`). A paired run always gets its outcomes step,
/// since the reader may correct the design. Refused as
/// [`StoreError::Corrupt`], with nothing written: a step not pending or not
/// the design, a prompt version that is not the run's, or a citation outside
/// the run's text.
pub fn save_design(
    db: &mut dyn Db,
    step_id: i64,
    proposal: &DesignProposal,
) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let run = pending_step(tx, step_id)?;
        if run.step != EvidenceStep::Design {
            return expect_step(step_id, run.step, "the design");
        }
        same_version(
            "design",
            proposal.prompt_version,
            run.versions.design_prompt,
        )?;
        insert_design(tx, &run.text, step_id, proposal)?;
        set_done(tx, step_id, Some(proposal.condensed))?;
        if run.paired || covered(proposal.design, proposal.rct_layout) {
            insert_step(tx, run.run_id, OUTCOMES_POSITION, EvidenceStep::Outcomes)?;
        } else {
            set_kind(tx, run.run_id, RunKind::NotCovered, true)?;
        }
        Ok(())
    })
}

/// Stores pending outcomes step `step_id`'s `proposal` in one transaction:
/// each outcome's row and citations in order, the step as done, then either
/// the run as *no primary outcome* and complete (no outcome), or as
/// *assessed* with `plan_rob2(n)`'s steps pending. A paired run stops there,
/// with no kind and no plan: it waits for its appraisal's confirmation, which
/// plans it (`plan_paired`). Refused as [`save_design`] refuses.
pub fn save_outcomes(
    db: &mut dyn Db,
    step_id: i64,
    proposal: &OutcomeProposal,
) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let run = pending_step(tx, step_id)?;
        if run.step != EvidenceStep::Outcomes {
            return expect_step(step_id, run.step, "the outcomes");
        }
        same_version(
            "outcomes",
            proposal.prompt_version,
            run.versions.outcome_prompt,
        )?;
        for (position, outcome) in proposal.outcomes.iter().enumerate() {
            insert_outcome(tx, &run.text, step_id, position, outcome)?;
        }
        set_done(tx, step_id, Some(proposal.condensed))?;
        if run.paired {
            return Ok(());
        }
        let n = proposal.outcomes.len();
        if n == 0 {
            return set_kind(tx, run.run_id, RunKind::NoPrimaryOutcome, true);
        }
        plan_assessed(tx, run.run_id, n)
    })
}

/// Marks run `run_id` *assessed* and adds `plan_rob2(n)`'s steps, pending,
/// from [`FIRST_ROB2_POSITION`] on.
fn plan_assessed(db: &mut dyn Db, run_id: i64, n: usize) -> Result<(), StoreError> {
    set_kind(db, run_id, RunKind::Assessed, false)?;
    for (i, step) in plan_rob2(n).into_iter().enumerate() {
        insert_step(
            db,
            run_id,
            FIRST_ROB2_POSITION + i,
            EvidenceStep::Rob2(step),
        )?;
    }
    Ok(())
}

/// Readies paired run `run_id` once its appraisal is confirmed with `n`
/// outcomes, inside the caller's transaction `tx`: a covered design makes it
/// *assessed* with `plan_rob2(n)`'s steps pending, as [`save_outcomes`] plans
/// an unpaired run; any other ends it as *not covered* and complete, as
/// [`save_design`] ends an unpaired run on such a design. A covered design
/// with no outcome is [`StoreError::Corrupt`]: confirming refuses it first.
pub(crate) fn plan_paired(
    tx: &mut dyn Db,
    run_id: i64,
    design_covered: bool,
    n: usize,
) -> Result<(), StoreError> {
    match (design_covered, n) {
        (false, _) => set_kind(tx, run_id, RunKind::NotCovered, true),
        (true, 0) => Err(StoreError::Corrupt(format!(
            "evidence run {run_id}: a covered design needs an outcome to plan RoB 2 for"
        ))),
        (true, n) => plan_assessed(tx, run_id, n),
    }
}

/// What a paired run stores as a failed RoB 2 step's message, in place of
/// bmlib's reason, which may name a judgement the reader must not see before
/// the reveal (design 2026-11-26, §2).
pub const PAIRED_STEP_FAILED: &str = "This step could not be completed; Continue asks it again.";

/// Whether a failed domain or overall judgement came from a request (the
/// failure may be cured by sending it again, or the server answered with an
/// HTTP status), in the transport's or the server's words, which name no
/// judgement. The driver then ends the call, since the next step would meet
/// the same server; any other failure (an unreadable reply, a failed
/// condensation) is stored and the run goes on.
pub(crate) fn from_a_request(retryable: bool, status: Option<u16>) -> bool {
    retryable || status.is_some()
}

/// The message stored for RoB 2 step `step_id` of `run` failed with
/// `reason`: the reason itself, except on a paired run for a failure not
/// from a request, which stores [`PAIRED_STEP_FAILED`] and logs the reason.
fn failure_message(
    run: &PendingRun,
    step_id: i64,
    reason: &str,
    retryable: bool,
    status: Option<u16>,
) -> String {
    if run.paired && !from_a_request(retryable, status) {
        log::warn!(
            "evidence step {step_id} of paired run {} failed: {reason}",
            run.run_id
        );
        return PAIRED_STEP_FAILED.to_string();
    }
    reason.to_string()
}

/// Stores pending domain step `step_id`'s `result` in one transaction. An
/// assessed domain stores its answers (D5's facts, other measurements and
/// analysis check included) with their citations, and is done. A failed one
/// is failed with its reason (on a paired run, `failure_message`'s),
/// `retryable` and `status`, and holds no result. Refused as
/// [`StoreError::Corrupt`], with nothing written: a step not pending, not a
/// domain step or of another domain, facts on a domain that is not D5 (or D5
/// without them), or a citation outside the run's text.
pub fn save_domain(db: &mut dyn Db, step_id: i64, result: &DomainResult) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let run = pending_step(tx, step_id)?;
        let EvidenceStep::Rob2(Rob2Step::Domain { domain, .. }) = run.step else {
            return expect_step(step_id, run.step, "a domain");
        };
        if result.domain() != domain {
            return Err(StoreError::Corrupt(format!(
                "evidence step {step_id} is {}, the result {}'s",
                domain_name(domain),
                domain_name(result.domain())
            )));
        }
        match result {
            DomainResult::Assessed(assessment) => {
                insert_domain(tx, &run.text, step_id, assessment)?;
                set_done(tx, step_id, Some(assessment.condensed))
            }
            DomainResult::Failed {
                reason,
                retryable,
                status,
                ..
            } => set_failed(
                tx,
                step_id,
                &StepFailure {
                    message: failure_message(&run, step_id, reason, *retryable, *status),
                    retryable: *retryable,
                    status: *status,
                },
            ),
        }
    })
}

/// Stores pending escalation step `step_id`'s `escalation` and its outcome as
/// `assessed` (`evidence_weight::assess_outcome`'s result) in one
/// transaction: the escalation, the overall judgement, and the ladder or its
/// error. The step is done, unless the escalation failed: then it is failed
/// with that failure (its message `failure_message`'s), and the outcome's row
/// is stored all the same (overall not assessed, with the ladder error) until
/// Continue sets it back. A step not pending or not an escalation, or an
/// `assessed` whose RoB 2 was not combined with `escalation`, is
/// [`StoreError::Corrupt`], with nothing written.
pub fn save_escalation(
    db: &mut dyn Db,
    step_id: i64,
    escalation: &Rob2Escalation,
    assessed: &AssessedOutcome,
) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let run = pending_step(tx, step_id)?;
        if !matches!(run.step, EvidenceStep::Rob2(Rob2Step::Escalation { .. })) {
            return expect_step(step_id, run.step, "an escalation");
        }
        // As bmlib's `combine_outcome` lays an escalation out on `OutcomeRob2`.
        let expected = match escalation {
            Rob2Escalation::NotNeeded => (None, None, false, None),
            Rob2Escalation::Asked(answer) => (Some(answer), None, false, None),
            Rob2Escalation::Failed {
                reason,
                retryable,
                status,
            } => (None, Some(reason), *retryable, *status),
        };
        let rob2 = &assessed.rob2;
        let given = (
            rob2.escalation.as_ref(),
            rob2.escalation_error.as_ref(),
            rob2.escalation_retryable,
            rob2.escalation_status,
        );
        if given != expected {
            return Err(StoreError::Corrupt(format!(
                "evidence step {step_id}: the escalation {escalation:?} is not the one its \
                 outcome's RoB 2 was combined with"
            )));
        }
        insert_outcome_result(tx, step_id, escalation, assessed)?;
        match escalation {
            Rob2Escalation::Failed {
                reason,
                retryable,
                status,
            } => set_failed(
                tx,
                step_id,
                &StepFailure {
                    message: failure_message(&run, step_id, reason, *retryable, *status),
                    retryable: *retryable,
                    status: *status,
                },
            ),
            Rob2Escalation::NotNeeded | Rob2Escalation::Asked(_) => set_done(tx, step_id, None),
        }
    })
}

/// Marks pending step `step_id` failed with `failure`, of any kind: a design
/// or outcomes step that could not be had, or a step the run could not take.
/// A step not pending is [`StoreError::Corrupt`]; no such step is
/// [`StoreError::NotFound`].
pub fn fail_step(db: &mut dyn Db, step_id: i64, failure: &StepFailure) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        pending_step(tx, step_id)?;
        set_failed(tx, step_id, failure)
    })
}

/// The done D5 steps of run `run_id` holding an other measurement whose
/// second look failed.
fn steps_with_a_failed_look(db: &mut dyn Db, run_id: i64) -> Result<BTreeSet<i64>, StoreError> {
    fetch_all(
        db,
        "SELECT DISTINCT m.step_id FROM evidence_d5_measurements m
         JOIN evidence_steps s ON s.id = m.step_id
         WHERE s.run_id = ? AND s.status = ? AND m.second_look_error IS NOT NULL",
        &[Value::from(run_id), Value::from(StepStatus::Done.as_str())],
    )?
    .iter()
    .map(|r| Ok(r.get_i64("step_id")?))
    .collect()
}

/// Sets run `run_id`'s failed steps, and its D5 steps with a failed second
/// look, back to pending, in one transaction, and returns how many steps went
/// back: Continue calls it before running what is pending. A domain set back
/// sets back its outcome's escalation too (D1 every outcome's), since an
/// escalation made from other domains no longer fits them. Each step set back
/// loses its stored result; a failed one keeps its error until it is saved,
/// so a Continue that stops before reaching it still shows why it failed.
pub fn reset_for_continue(db: &mut dyn Db, run_id: i64) -> Result<usize, StoreError> {
    transaction_with(db, |tx| {
        let steps = steps(tx, run_id)?;
        let looked = steps_with_a_failed_look(tx, run_id)?;
        let mut back: BTreeSet<i64> = steps
            .iter()
            .filter(|s| s.status == StepStatus::Failed || looked.contains(&s.id))
            .map(|s| s.id)
            .collect();
        let redone: Vec<Option<usize>> = steps
            .iter()
            .filter(|s| back.contains(&s.id))
            .filter_map(|s| match s.step {
                EvidenceStep::Rob2(Rob2Step::Domain { outcome, .. }) => Some(outcome),
                _ => None,
            })
            .collect();
        for s in &steps {
            if let EvidenceStep::Rob2(Rob2Step::Escalation { outcome }) = s.step {
                // `None` is D1, which every outcome's escalation was made from.
                let stale = redone.iter().any(|o| o.is_none_or(|o| o == outcome));
                if stale && s.status != StepStatus::Pending {
                    back.insert(s.id);
                }
            }
        }
        for &id in &back {
            delete_results(tx, id)?;
            execute(
                tx,
                "UPDATE evidence_steps SET status = ?, condensed = NULL WHERE id = ?",
                &[Value::from(StepStatus::Pending.as_str()), Value::from(id)],
            )?;
        }
        if !back.is_empty() {
            log::info!(
                "evidence run {run_id}: {} step(s) set back to pending for Continue",
                back.len()
            );
        }
        Ok(back.len())
    })
}

/// Marks run `run_id` `complete` once its kind is known, no step is pending
/// or failed, and no D5 holds a failed second look, and returns whether it
/// did; otherwise the run stays `incomplete` and the result is `false`. No
/// such run is [`StoreError::NotFound`].
pub fn finish_run(db: &mut dyn Db, run_id: i64) -> Result<bool, StoreError> {
    let r = fetch_one(
        db,
        "SELECT kind FROM evidence_runs WHERE id = ?",
        &[Value::from(run_id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("evidence run {run_id}")))?;
    if r.get("kind")?.is_null() {
        return Ok(false);
    }
    let open = steps(db, run_id)?
        .iter()
        .any(|s| s.status != StepStatus::Done);
    if open || !steps_with_a_failed_look(db, run_id)?.is_empty() {
        return Ok(false);
    }
    execute(
        db,
        "UPDATE evidence_runs SET status = ? WHERE id = ?",
        &[
            Value::from(RunStatus::Complete.as_str()),
            Value::from(run_id),
        ],
    )?;
    Ok(true)
}

/// Sets run `run_id`'s `first_shown_at` to now, only while it is empty and a
/// domain step is done (a RoB 2 result is on show), and returns whether it
/// did. Never changed afterwards: phase 3 reads it to know an appraisal
/// saved later was not blind. An unpaired run shown this way also clears
/// `blind` on the paper's open appraisal not yet revealed: its reader has now
/// seen a machine verdict before saving (design 2026-11-26, §2).
pub fn mark_shown(db: &mut dyn Db, run_id: i64) -> Result<bool, StoreError> {
    // First, so a failure between the two errs towards "not blind".
    execute(
        db,
        "UPDATE evidence_appraisals SET blind = 0
         WHERE blind = 1 AND set_aside_at IS NULL AND revealed_at IS NULL
           AND paper_id = (SELECT paper_id FROM evidence_runs r
                           WHERE r.id = ? AND r.appraisal_id IS NULL AND r.first_shown_at IS NULL
                             AND EXISTS (SELECT 1 FROM evidence_steps
                                         WHERE run_id = r.id AND kind = ? AND status = ?))",
        &[
            Value::from(run_id),
            Value::from(STEP_DOMAIN),
            Value::from(StepStatus::Done.as_str()),
        ],
    )?;
    let n = execute(
        db,
        "UPDATE evidence_runs SET first_shown_at = datetime('now')
         WHERE id = ? AND first_shown_at IS NULL
           AND EXISTS (SELECT 1 FROM evidence_steps
                       WHERE run_id = evidence_runs.id AND kind = ? AND status = ?)",
        &[
            Value::from(run_id),
            Value::from(STEP_DOMAIN),
            Value::from(StepStatus::Done.as_str()),
        ],
    )?;
    Ok(n > 0)
}

/// Marks run `run_id` as stopped because its stored outcomes leave no room
/// for the paper in its window, so the tab stops offering a Continue that
/// cannot succeed (#166). The window is the model's or the user's
/// context-window setting, the smaller, so a change of that setting clears
/// the mark ([`clear_too_long`]). No such run is [`StoreError::NotFound`].
pub fn mark_too_long(db: &mut dyn Db, run_id: i64) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE evidence_runs SET too_long = 1 WHERE id = ?",
        &[Value::from(run_id)],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("evidence run {run_id}")));
    }
    Ok(())
}

/// Clears every run's [`mark_too_long`] mark, since the window it was judged
/// against has changed (the user's context-window setting): Continue is
/// offered again, and sizes its budget again before any call, marking the run
/// anew if it still does not fit.
///
/// shortcut: a model's own window, cached per session, can also grow after a
/// restart and does not clear the mark; Re-run gets past it then.
pub fn clear_too_long(db: &mut dyn Db) -> Result<(), StoreError> {
    execute(
        db,
        "UPDATE evidence_runs SET too_long = 0 WHERE too_long = 1",
        &[],
    )?;
    Ok(())
}

/// The version columns of `evidence_runs`, as [`versions_from`] reads them.
const VERSION_COLUMNS: &str = "design_prompt_version, outcome_prompt_version, \
    rob2_prompt_version, algorithm_version, ladder_version, ladder_settings_json";

/// Whether run `run_id` was made under earlier rules (its versions are not
/// [`RunVersions::current`]). No such run is [`StoreError::NotFound`].
pub(crate) fn earlier_rules(db: &mut dyn Db, run_id: i64) -> Result<bool, StoreError> {
    let r = fetch_one(
        db,
        &format!("SELECT {VERSION_COLUMNS} FROM evidence_runs WHERE id = ?"),
        &[Value::from(run_id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("evidence run {run_id}")))?;
    Ok(versions_from(&r, run_id)? != RunVersions::current())
}

/// Run `run_id`'s versions from a row holding [`VERSION_COLUMNS`].
fn versions_from(r: &Row, run_id: i64) -> Result<RunVersions, StoreError> {
    let version = |name: &str| -> Result<u32, StoreError> {
        let v = r.get_i64(name)?;
        u32::try_from(v)
            .map_err(|_| StoreError::Corrupt(format!("evidence run {run_id}: {name} is {v}")))
    };
    Ok(RunVersions {
        design_prompt: version("design_prompt_version")?,
        outcome_prompt: version("outcome_prompt_version")?,
        rob2_prompt: version("rob2_prompt_version")?,
        algorithm: r.get_str("algorithm_version")?.to_string(),
        ladder: r.get_str("ladder_version")?.to_string(),
        ladder_settings_json: r.get_str("ladder_settings_json")?.to_string(),
    })
}

/// Whether `steps` are the plan a run of `kind` and `status` with `outcomes`
/// outcomes holds. An unpaired run's `outcomes` are its stored ones: the
/// design alone (pending or failed, or done and not covered), then the
/// outcomes (not done, or done with none), then RoB 2's whole plan for
/// `outcomes` once assessed. A paired run's are its appraisal's confirmed
/// ones: the design and outcomes steps (both done while it waits, and once
/// confirmed on a design not covered), then RoB 2's plan for `outcomes`, or
/// that plan alone on a re-paired run.
fn check_plan(
    kind: Option<RunKind>,
    status: RunStatus,
    steps: &[StoredStep],
    outcomes: usize,
    paired: bool,
) -> Result<(), String> {
    let plan: Vec<EvidenceStep> = steps.iter().map(|s| s.step).collect();
    // Today's `plan_rob2` (via `plan_of`), on earlier-rules runs too: a bmlib
    // whose plan changes would make every older run read as Corrupt.
    let full = plan_of(outcomes);
    let done = |i: usize| steps.get(i).is_some_and(|s| s.status == StepStatus::Done);
    let proposed = plan == full[..2] && done(0) && done(1);
    let fits = match (kind, paired) {
        (None, _) => {
            status == RunStatus::Incomplete
                && outcomes == 0
                && ((plan == full[..1] && !done(0))
                    || (plan == full[..2] && done(0) && !done(1))
                    || (paired && proposed))
        }
        (Some(RunKind::NotCovered), false) => plan == full[..1] && done(0),
        (Some(RunKind::NotCovered), true) => proposed,
        (Some(RunKind::NoPrimaryOutcome), false) => outcomes == 0 && proposed,
        (Some(RunKind::NoPrimaryOutcome), true) => false,
        (Some(RunKind::Assessed), false) => outcomes > 0 && plan == full && done(0) && done(1),
        (Some(RunKind::Assessed), true) => {
            outcomes > 0 && ((plan == full && done(0) && done(1)) || plan == full[2..])
        }
    };
    if fits {
        return Ok(());
    }
    let kind = kind.map_or("unknown", RunKind::as_str);
    let paired = if paired { "paired " } else { "" };
    Err(format!(
        "its steps {plan:?} are not the plan of a {} {paired}run of kind {kind} with \
         {outcomes} outcome(s)",
        status.as_str()
    ))
}

/// The stored run `run_id`, or `None` if there is none: the run row and every
/// step in plan order. A row this code did not write is
/// [`StoreError::Corrupt`]: a missing text, an unknown status or kind, a step
/// of a shape `step_from` refuses, or steps that are not the plan its kind
/// and outcomes give (a step missing, moved or added; `check_plan`). A paired
/// run's outcomes are its appraisal's confirmed ones, so a damaged appraisal
/// makes its run Corrupt too. The steps' results are read by
/// [`crate::evidence_rows`].
pub fn run(db: &mut dyn Db, run_id: i64) -> Result<Option<StoredEvidenceRun>, StoreError> {
    let Some(r) = fetch_one(
        db,
        &format!(
            "SELECT paper_id, canonical_id, model, status, kind, created_at, first_shown_at,
                    too_long, appraisal_id, {VERSION_COLUMNS}
             FROM evidence_runs WHERE id = ?"
        ),
        &[Value::from(run_id)],
    )?
    else {
        return Ok(None);
    };
    let bad = |why: String| StoreError::Corrupt(format!("evidence run {run_id}: {why}"));
    let paper = PaperId(r.get_str("paper_id")?.to_string());
    let canonical_id = r.get_i64("canonical_id")?;
    let text = canonical_by_id(db, &paper, canonical_id)?
        .ok_or_else(|| bad("its canonical text is not stored".into()))?
        .text;
    let status = r.get_str("status")?;
    let status =
        RunStatus::parse(status).ok_or_else(|| bad(format!("unknown status {status:?}")))?;
    let kind = optional(&r, "kind", text_of)
        .map_err(bad)?
        .map(|k| RunKind::parse(&k).ok_or_else(|| bad(format!("unknown kind {k:?}"))))
        .transpose()?;
    let versions = versions_from(&r, run_id)?;
    let appraisal = optional(&r, "appraisal_id", Value::as_i64).map_err(bad)?;
    let steps = steps(db, run_id)?;
    let outcomes = if appraisal.is_some() {
        rob2_outcomes(db, run_id)?.len()
    } else {
        let n = fetch_one(
            db,
            "SELECT count(*) AS n FROM evidence_outcomes o
             JOIN evidence_steps s ON s.id = o.step_id WHERE s.run_id = ?",
            &[Value::from(run_id)],
        )?
        .map(|r| r.get_i64("n"))
        .transpose()?
        .unwrap_or(0);
        usize::try_from(n).map_err(|_| bad("a negative count".into()))?
    };
    check_plan(kind, status, &steps, outcomes, appraisal.is_some()).map_err(bad)?;
    Ok(Some(StoredEvidenceRun {
        id: run_id,
        canonical_id,
        model: r.get_str("model")?.to_string(),
        created_at: r.get_str("created_at")?.to_string(),
        first_shown_at: optional(&r, "first_shown_at", text_of).map_err(bad)?,
        status,
        kind,
        earlier_rules: versions != RunVersions::current(),
        too_long: r.get_i64("too_long")? != 0,
        versions,
        steps,
        text,
        appraisal,
    }))
}

/// The paper and canonical text run `run_id` is on, or `None` if there is no
/// such run.
pub fn run_text(db: &mut dyn Db, run_id: i64) -> Result<Option<(PaperId, i64)>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT paper_id, canonical_id FROM evidence_runs WHERE id = ?",
        &[Value::from(run_id)],
    )?;
    Ok(match row {
        Some(r) => Some((
            PaperId(r.get_str("paper_id")?.to_string()),
            r.get_i64("canonical_id")?,
        )),
        None => None,
    })
}

/// The SQL condition, on a row of `evidence_runs`, that the run is not
/// hidden: unpaired, or paired with an appraisal already revealed.
const NOT_HIDDEN: &str = "(evidence_runs.appraisal_id IS NULL OR EXISTS (
    SELECT 1 FROM evidence_appraisals a
    WHERE a.id = evidence_runs.appraisal_id AND a.revealed_at IS NOT NULL))";

/// Whether run `run_id` is hidden: paired with an appraisal not yet revealed
/// (set aside or not), so nothing it judged about RoB 2 may be shown. No such
/// run is [`StoreError::NotFound`].
pub fn is_hidden(db: &mut dyn Db, run_id: i64) -> Result<bool, StoreError> {
    let r = fetch_one(
        db,
        &format!("SELECT {NOT_HIDDEN} AS shown FROM evidence_runs WHERE id = ?"),
        &[Value::from(run_id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("evidence run {run_id}")))?;
    Ok(r.get_i64("shown")? == 0)
}

/// The newest run on canonical text `canonical_id` of `paper`, complete or
/// not, hidden runs ([`is_hidden`]) left out.
pub fn latest_run(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<Option<i64>, StoreError> {
    let row = fetch_one(
        db,
        &format!(
            "SELECT max(id) AS id FROM evidence_runs
             WHERE paper_id = ? AND canonical_id = ? AND {NOT_HIDDEN}"
        ),
        &[Value::from(paper.as_str()), Value::from(canonical_id)],
    )?;
    Ok(match row {
        Some(r) => r.get("id")?.as_i64(),
        None => None,
    })
}

/// Whether any run of `paper` was shown with a RoB 2 result
/// (`first_shown_at`, [`mark_shown`]): an appraisal started then is not blind.
pub fn any_shown(db: &mut dyn Db, paper: &PaperId) -> Result<bool, StoreError> {
    Ok(fetch_one(
        db,
        "SELECT 1 AS one FROM evidence_runs
         WHERE paper_id = ? AND first_shown_at IS NOT NULL LIMIT 1",
        &[Value::from(paper.as_str())],
    )?
    .is_some())
}

/// The newest run paired with appraisal `appraisal_id`, hidden or not: the
/// one compare and *Continue* use.
pub fn latest_paired_run(db: &mut dyn Db, appraisal_id: i64) -> Result<Option<i64>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT max(id) AS id FROM evidence_runs WHERE appraisal_id = ?",
        &[Value::from(appraisal_id)],
    )?;
    Ok(match row {
        Some(r) => r.get("id")?.as_i64(),
        None => None,
    })
}

/// Every run on `paper`, newest first, hidden runs ([`is_hidden`]) left out.
pub fn runs(db: &mut dyn Db, paper: &PaperId) -> Result<Vec<EvidenceRunSummary>, StoreError> {
    let rows = fetch_all(
        db,
        &format!(
            "SELECT id, canonical_id, model, created_at, status FROM evidence_runs
             WHERE paper_id = ? AND {NOT_HIDDEN} ORDER BY id DESC"
        ),
        &[Value::from(paper.as_str())],
    )?;
    rows.iter()
        .map(|r| {
            let status = r.get_str("status")?;
            Ok(EvidenceRunSummary {
                id: r.get_i64("id")?,
                canonical_id: r.get_i64("canonical_id")?,
                model: r.get_str("model")?.to_string(),
                created_at: r.get_str("created_at")?.to_string(),
                status: RunStatus::parse(status).ok_or_else(|| {
                    StoreError::Corrupt(format!("unknown evidence run status {status:?}"))
                })?,
            })
        })
        .collect()
}

/// Whether an evidence run on canonical text `canonical_id` holds a citation:
/// whether spans point into that text, so it must not be re-derived or
/// replaced. A run with nothing cited is no dependent.
pub fn has_citations(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError> {
    Ok(fetch_one(
        db,
        "SELECT 1 AS one FROM evidence_citations c JOIN evidence_steps s ON s.id = c.step_id
         JOIN evidence_runs r ON r.id = s.run_id
         WHERE r.canonical_id = ? LIMIT 1",
        &[Value::from(canonical_id)],
    )?
    .is_some())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::evidence_rows::{stored_domain, stored_outcome_result};
    use crate::pico_store::tests::{cite, count, paper_with_text};
    use crate::store::open_memory_store;
    use bmlib::citations::PassageContradiction;
    use bmlib::quality::rob2_agent::d5_assessment;
    use bmlib::quality::rob2_d5_checks::{fact_verdict, D5Fact, D5_FACTS};
    use bmlib::quality::rob2_d5_facts::{AnalysisCheck, D5Facts, FactReport, OtherMeasurement};
    use bmlib::quality::{
        applicable_questions, build_ladder, domain_judgement, Answer, Answers, DomainAssessment,
        EffectScale, Escalation, Judgement, Level, MachineAnswer, OutcomeRob2, PicoCitation,
        PicoContradictedQuote, ProposedOutcome, RctLayout, ReportedResult, StudyDesign,
    };
    use rusqlite::Connection;

    /// A proposed `design` with `layout`, backed by "Para one.", every quote
    /// list filled so a round trip shows each one.
    pub(crate) fn design(
        c: &mut Connection,
        p: &PaperId,
        cid: i64,
        design: StudyDesign,
        layout: Option<RctLayout>,
    ) -> DesignProposal {
        DesignProposal {
            design,
            rct_layout: layout,
            citations: vec![cite(c, p, cid, "Para one.")],
            quotes_not_found: vec!["Nowhere.".into()],
            contradicted: vec![PicoContradictedQuote {
                quote: "Para three.".into(),
                contradiction: PassageContradiction::Number,
            }],
            quotes_dropped: 1,
            reasoning: "Randomised.".into(),
            condensed: false,
            prompt_version: DESIGN_PROMPT_VERSION,
        }
    }

    /// A parallel-group randomised trial: the design RoB 2 covers.
    pub(crate) fn trial(c: &mut Connection, p: &PaperId, cid: i64) -> DesignProposal {
        design(c, p, cid, StudyDesign::Rct, Some(RctLayout::Parallel))
    }

    /// The name of the outcome at `index`.
    pub(crate) fn outcome_name(index: usize) -> String {
        format!("Outcome {index}")
    }

    /// The outcome at `index`, with a full result and backed by "Para two.".
    pub(crate) fn outcome(
        c: &mut Connection,
        p: &PaperId,
        cid: i64,
        index: usize,
    ) -> ProposedOutcome {
        ProposedOutcome {
            name: outcome_name(index),
            timepoint: Some("12 months".into()),
            result: Some(ReportedResult {
                measure: "mean difference".into(),
                scale: EffectScale::Difference,
                estimate: -0.4,
                ci_low: Some(-0.7),
                ci_high: Some(-0.1),
                ci_level: Some(0.95),
                n_analysed: Some(312),
            }),
            result_error: None,
            citations: vec![cite(c, p, cid, "Para two.")],
            quotes_not_found: vec!["Elsewhere.".into()],
            contradicted: vec![],
            quotes_dropped: 2,
        }
    }

    /// `n` outcomes, as one condensed proposal.
    pub(crate) fn outcomes(c: &mut Connection, p: &PaperId, cid: i64, n: usize) -> OutcomeProposal {
        let mut outcomes = Vec::new();
        for i in 0..n {
            outcomes.push(outcome(c, p, cid, i));
        }
        OutcomeProposal {
            outcomes,
            condensed: true,
            prompt_version: OUTCOME_PROMPT_VERSION,
        }
    }

    /// `domain` assessed with every question that applies answered `answer`
    /// (asked until none is left), the first answer backed by `citation`.
    pub(crate) fn assessed(
        domain: Domain,
        answer: Answer,
        citation: Option<PicoCitation>,
    ) -> DomainResult {
        let mut given = Answers::new();
        loop {
            let next: Vec<&str> = applicable_questions(domain, &given)
                .into_iter()
                .filter(|q| !given.contains_key(*q))
                .collect();
            if next.is_empty() {
                break;
            }
            for q in next {
                given.insert(q.to_string(), answer);
            }
        }
        let judgement = domain_judgement(domain, &given).unwrap();
        let answers = given
            .iter()
            .enumerate()
            .map(|(i, (q, a))| {
                let machine = MachineAnswer {
                    answer: *a,
                    citations: citation.iter().filter(|_| i == 0).cloned().collect(),
                    quotes_not_found: vec![format!("{q} not found")],
                    contradicted: vec![],
                    quotes_dropped: i,
                    reasoning: format!("{q} because"),
                };
                (q.clone(), machine)
            })
            .collect();
        DomainResult::Assessed(DomainAssessment {
            domain,
            answers,
            judgement,
            condensed: false,
            ignored: vec![],
            facts: None,
        })
    }

    /// D5 for outcome `outcome`, from its facts: measure-fixed claimed with
    /// `fact_quote` (none claimed without one), and with `failed_look` one
    /// other measurement named by it whose second look failed, plus one whose
    /// second look found nothing.
    pub(crate) fn d5(
        outcome: &str,
        fact_quote: Option<PicoCitation>,
        failed_look: Option<PicoCitation>,
    ) -> DomainResult {
        let facts = D5_FACTS
            .iter()
            .map(|&fact| {
                let citations: Vec<PicoCitation> = fact_quote
                    .iter()
                    .filter(|_| fact == D5Fact::MeasureFixed)
                    .cloned()
                    .collect();
                let claimed = !citations.is_empty();
                let found: Vec<&str> = citations.iter().map(|c| c.text.as_str()).collect();
                let verdict = fact_verdict(fact, claimed, &found, outcome);
                let report = FactReport {
                    claimed,
                    citations: citations.clone(),
                    quotes_not_found: vec![],
                    contradicted: vec![],
                    quotes_dropped: 0,
                    verdict,
                };
                (fact, report)
            })
            .collect();
        let measurements = failed_look
            .into_iter()
            .flat_map(|named| {
                let failed = OtherMeasurement {
                    label: "at 6 months".into(),
                    named: vec![named.clone()],
                    result: vec![],
                    second_look: None,
                    second_look_error: Some("the server is busy".into()),
                    second_look_retryable: true,
                    second_look_status: Some(503),
                    quotes_not_found: vec!["Gone.".into()],
                    contradicted: vec![],
                    quotes_dropped: 1,
                };
                let looked = OtherMeasurement {
                    label: "at 3 months".into(),
                    second_look: Some(vec![]),
                    second_look_error: None,
                    second_look_retryable: false,
                    second_look_status: None,
                    ..failed.clone()
                };
                [failed, looked]
            })
            .collect();
        DomainResult::Assessed(d5_assessment(D5Facts {
            outcome: outcome.into(),
            facts,
            measurements,
            analysis: AnalysisCheck::default(),
        }))
    }

    /// `outcome` taken through RoB 2 with `escalation`: some concerns and its
    /// ladder, or not assessed with a ladder error when the escalation failed.
    pub(crate) fn weighed(
        outcome: &ProposedOutcome,
        escalation: &Rob2Escalation,
    ) -> AssessedOutcome {
        let some = Some(Judgement::SomeConcerns);
        // As bmlib's `combine_outcome` lays the escalation out.
        let (asked, error, retryable, status, overall) = match escalation {
            Rob2Escalation::NotNeeded => (None, None, false, None, some),
            Rob2Escalation::Asked(e) => (Some(e.clone()), None, false, None, some),
            Rob2Escalation::Failed {
                reason,
                retryable,
                status,
            } => (None, Some(reason.clone()), *retryable, *status, None),
        };
        let ladder = overall.map(|j| {
            build_ladder(
                Level::High,
                j,
                outcome.result.as_ref(),
                &LadderSettings::default(),
            )
            .unwrap()
        });
        AssessedOutcome {
            outcome: outcome.clone(),
            rob2: OutcomeRob2 {
                outcome: outcome.name.clone(),
                domains: vec![],
                escalation: asked,
                escalation_error: error,
                escalation_retryable: retryable,
                escalation_status: status,
                overall,
            },
            ladder_error: overall
                .is_none()
                .then(|| "the escalation failed".to_string()),
            ladder,
        }
    }

    /// An escalation the model answered: not serious enough.
    pub(crate) fn asked() -> Rob2Escalation {
        Rob2Escalation::Asked(Escalation {
            escalate: false,
            reasoning: "Not serious.".into(),
        })
    }

    /// Saves a successful result for pending `step`, as the run's other
    /// helpers make them (D5 with nothing claimed and no failed look).
    pub(crate) fn save_step(c: &mut Connection, p: &PaperId, cid: i64, step: &PendingStep) {
        match step.step {
            EvidenceStep::Design => {
                let d = trial(c, p, cid);
                save_design(c, step.id, &d).unwrap();
            }
            EvidenceStep::Outcomes => {
                let o = outcomes(c, p, cid, 2);
                save_outcomes(c, step.id, &o).unwrap();
            }
            EvidenceStep::Rob2(Rob2Step::Domain {
                domain: Domain::D5,
                outcome,
            }) => {
                let name = outcome_name(outcome.unwrap());
                save_domain(c, step.id, &d5(&name, None, None)).unwrap();
            }
            EvidenceStep::Rob2(Rob2Step::Domain { domain, .. }) => {
                let quote = cite(c, p, cid, "Para one.");
                save_domain(c, step.id, &assessed(domain, Answer::Y, Some(quote))).unwrap();
            }
            EvidenceStep::Rob2(Rob2Step::Escalation { outcome: index }) => {
                let o = outcome(c, p, cid, index);
                save_escalation(c, step.id, &asked(), &weighed(&o, &asked())).unwrap();
            }
        }
    }

    /// Saves every pending step of `run_id` but those in `skip`, the ones each
    /// save adds included.
    pub(crate) fn save_pending(
        c: &mut Connection,
        p: &PaperId,
        cid: i64,
        run_id: i64,
        skip: &[EvidenceStep],
    ) {
        while let Some(step) = pending_steps(c, run_id)
            .unwrap()
            .into_iter()
            .find(|s| !skip.contains(&s.step))
        {
            save_step(c, p, cid, &step);
        }
    }

    /// A paper, its text, and a run on it with its design and two outcomes
    /// saved, so RoB 2's plan is pending.
    pub(crate) fn assessed_run(c: &mut Connection) -> (PaperId, i64, i64) {
        let (p, cid) = paper_with_text(c);
        let run_id = create_run(c, &p, cid, "m", None).unwrap();
        let rob2: Vec<EvidenceStep> = plan_rob2(2).into_iter().map(EvidenceStep::Rob2).collect();
        save_pending(c, &p, cid, run_id, &rob2);
        (p, cid, run_id)
    }

    /// Drives run `run_id`'s design (a parallel trial) and two outcomes to
    /// done and takes RoB 2's plan back off, leaving it as a paired run waits
    /// for its appraisal's confirmation: outcomes done, no kind, no plan.
    pub(crate) fn proposals_only(c: &mut Connection, p: &PaperId, cid: i64, run_id: i64) {
        let rob2: Vec<EvidenceStep> = plan_rob2(2).into_iter().map(EvidenceStep::Rob2).collect();
        save_pending(c, p, cid, run_id, &rob2);
        execute(
            c,
            "DELETE FROM evidence_steps WHERE run_id = ? AND position >= ?",
            &[Value::from(run_id), Value::from(FIRST_ROB2_POSITION)],
        )
        .unwrap();
        execute(
            c,
            "UPDATE evidence_runs SET kind = NULL WHERE id = ?",
            &[Value::from(run_id)],
        )
        .unwrap();
    }

    /// Run `run_id`'s stored kind and status.
    pub(crate) fn kind_and_status(c: &mut Connection, run_id: i64) -> (Option<String>, String) {
        let r = fetch_one(
            c,
            "SELECT kind, status FROM evidence_runs WHERE id = ?",
            &[Value::from(run_id)],
        )
        .unwrap()
        .unwrap();
        (
            r.get("kind").unwrap().as_str().map(str::to_string),
            r.get_str("status").unwrap().to_string(),
        )
    }

    /// Only a parallel-group randomised trial is covered: not another trial
    /// layout, a trial with no layout, or a design with no ladder start.
    #[test]
    fn only_a_parallel_trial_is_covered() {
        assert!(covered(StudyDesign::Rct, Some(RctLayout::Parallel)));
        for layout in [RctLayout::Cluster, RctLayout::Crossover, RctLayout::Other] {
            assert!(!covered(StudyDesign::Rct, Some(layout)), "{layout:?}");
        }
        assert!(!covered(StudyDesign::Rct, None));
        assert!(!covered(StudyDesign::CohortProspective, None));
        assert!(!covered(
            StudyDesign::CohortProspective,
            Some(RctLayout::Parallel)
        ));
    }

    /// A paired run readied for a covered design gets RoB 2's plan for the
    /// confirmed outcomes from the first RoB 2 position; for a design not
    /// covered it ends not covered and complete, with no step added.
    #[test]
    fn plan_paired_plans_rob2_or_ends_not_covered() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let covered = create_run(&mut c, &p, cid, "m", None).unwrap();
        proposals_only(&mut c, &p, cid, covered);
        transaction_with(&mut c, |tx| plan_paired(tx, covered, true, 1)).unwrap();
        let pending: Vec<EvidenceStep> = pending_steps(&mut c, covered)
            .unwrap()
            .into_iter()
            .map(|s| s.step)
            .collect();
        let want: Vec<EvidenceStep> = plan_rob2(1).into_iter().map(EvidenceStep::Rob2).collect();
        assert_eq!(pending, want);
        let first = fetch_one(
            &mut c,
            "SELECT min(position) AS p FROM evidence_steps WHERE run_id = ? AND kind = ?",
            &[Value::from(covered), Value::from(STEP_DOMAIN)],
        )
        .unwrap()
        .unwrap()
        .get_i64("p")
        .unwrap();
        assert_eq!(first, FIRST_ROB2_POSITION as i64);
        assert_eq!(
            kind_and_status(&mut c, covered),
            (Some("assessed".into()), "incomplete".into())
        );
        let other = create_run(&mut c, &p, cid, "m", None).unwrap();
        proposals_only(&mut c, &p, cid, other);
        transaction_with(&mut c, |tx| plan_paired(tx, other, false, 2)).unwrap();
        assert!(pending_steps(&mut c, other).unwrap().is_empty());
        assert_eq!(
            kind_and_status(&mut c, other),
            (Some("not_covered".into()), "complete".into())
        );
        let refused = transaction_with(&mut c, |tx| plan_paired(tx, other, true, 0));
        assert!(
            matches!(refused, Err(StoreError::Corrupt(_))),
            "{refused:?}"
        );
    }

    /// The id of `step` in run `run_id`.
    pub(crate) fn step_id(c: &mut Connection, run_id: i64, step: EvidenceStep) -> i64 {
        stored(c, run_id)
            .steps
            .iter()
            .find(|s| s.step == step)
            .unwrap()
            .id
    }

    /// Domain `domain` of `outcome` (`None` for D1).
    pub(crate) fn domain_step(domain: Domain, outcome: Option<usize>) -> EvidenceStep {
        EvidenceStep::Rob2(Rob2Step::Domain { domain, outcome })
    }

    /// The escalation of `outcome`.
    pub(crate) fn escalation_step(outcome: usize) -> EvidenceStep {
        EvidenceStep::Rob2(Rob2Step::Escalation { outcome })
    }

    /// The stored run `run_id`.
    pub(crate) fn stored(c: &mut Connection, run_id: i64) -> StoredEvidenceRun {
        run(c, run_id).unwrap().unwrap()
    }

    /// The status of `step` in run `run_id`.
    fn status_of(c: &mut Connection, run_id: i64, step: EvidenceStep) -> StepStatus {
        stored(c, run_id)
            .steps
            .iter()
            .find(|s| s.step == step)
            .unwrap()
            .status
    }

    /// A design that is not a parallel-group trial ends the run as not
    /// covered: a crossover trial, then a cohort study.
    #[test]
    fn a_design_that_is_not_covered_completes_the_run() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        for (d, layout) in [
            (StudyDesign::Rct, Some(RctLayout::Crossover)),
            (StudyDesign::CohortProspective, None),
        ] {
            let run_id = create_run(&mut c, &p, cid, "m", None).unwrap();
            let pending = pending_steps(&mut c, run_id).unwrap();
            let steps: Vec<EvidenceStep> = pending.iter().map(|s| s.step).collect();
            assert_eq!(steps, vec![EvidenceStep::Design]);
            let proposal = design(&mut c, &p, cid, d, layout);
            save_design(&mut c, pending[0].id, &proposal).unwrap();
            assert!(pending_steps(&mut c, run_id).unwrap().is_empty());
            let got = stored(&mut c, run_id);
            assert_eq!(got.status, RunStatus::Complete);
            assert_eq!(got.kind, Some(RunKind::NotCovered));
            assert_eq!(got.steps.len(), 1);
            assert_eq!(got.steps[0].status, StepStatus::Done);
            assert_eq!(got.steps[0].condensed, Some(false));
            assert!(!got.earlier_rules);
        }
    }

    /// A trial whose paper names no primary outcome ends as such, complete.
    #[test]
    fn no_outcome_completes_the_run() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let run_id = create_run(&mut c, &p, cid, "m", None).unwrap();
        save_pending(&mut c, &p, cid, run_id, &[EvidenceStep::Outcomes]);
        let outcomes_step = pending_steps(&mut c, run_id).unwrap();
        assert_eq!(outcomes_step[0].step, EvidenceStep::Outcomes);
        let none = outcomes(&mut c, &p, cid, 0);
        save_outcomes(&mut c, outcomes_step[0].id, &none).unwrap();
        let got = stored(&mut c, run_id);
        assert_eq!(got.status, RunStatus::Complete);
        assert_eq!(got.kind, Some(RunKind::NoPrimaryOutcome));
        assert_eq!(got.steps.len(), 2);
        assert_eq!(got.steps[1].condensed, Some(true));
    }

    /// Two outcomes add RoB 2's plan for two, pending in its order; a step
    /// already done is not saved again.
    #[test]
    fn the_outcomes_add_the_plan() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let pending: Vec<EvidenceStep> = pending_steps(&mut c, run_id)
            .unwrap()
            .iter()
            .map(|s| s.step)
            .collect();
        let plan: Vec<EvidenceStep> = plan_rob2(2).into_iter().map(EvidenceStep::Rob2).collect();
        assert_eq!(pending, plan);
        let got = stored(&mut c, run_id);
        assert_eq!(got.kind, Some(RunKind::Assessed));
        assert_eq!(got.status, RunStatus::Incomplete);
        let done = step_id(&mut c, run_id, EvidenceStep::Outcomes);
        let again = outcomes(&mut c, &p, cid, 2);
        assert!(matches!(
            save_outcomes(&mut c, done, &again),
            Err(StoreError::Corrupt(_))
        ));
        let d = trial(&mut c, &p, cid);
        assert!(matches!(
            save_design(&mut c, done + 1000, &d),
            Err(StoreError::NotFound(_))
        ));
    }

    /// A failed domain keeps its reason, whether it may be retried, and its
    /// status, and holds no result; a failed step is not failed again.
    #[test]
    fn a_failed_domain_keeps_its_failure() {
        let mut c = open_memory_store().unwrap();
        let (_, _, run_id) = assessed_run(&mut c);
        let d3 = step_id(&mut c, run_id, domain_step(Domain::D3, Some(0)));
        let failed = DomainResult::Failed {
            domain: Domain::D3,
            reason: "the server is busy".into(),
            retryable: true,
            status: Some(500),
        };
        save_domain(&mut c, d3, &failed).unwrap();
        let got = stored(&mut c, run_id);
        let step = got.steps.iter().find(|s| s.id == d3).unwrap();
        assert_eq!(step.status, StepStatus::Failed);
        assert_eq!(
            step.failure,
            Some(StepFailure {
                message: "the server is busy".into(),
                retryable: true,
                status: Some(500),
            })
        );
        assert_eq!(step.condensed, None);
        assert_eq!(stored_domain(&mut c, d3).unwrap(), None);
        let failure = StepFailure {
            message: "again".into(),
            retryable: false,
            status: None,
        };
        assert!(matches!(
            fail_step(&mut c, d3, &failure),
            Err(StoreError::Corrupt(_))
        ));
        let d4 = step_id(&mut c, run_id, domain_step(Domain::D4, Some(0)));
        fail_step(&mut c, d4, &failure).unwrap();
        let step = stored(&mut c, run_id)
            .steps
            .into_iter()
            .find(|s| s.id == d4)
            .unwrap();
        assert_eq!(step.failure, Some(failure));
        let wrong = assessed(Domain::D2, Answer::Y, None);
        let d5 = step_id(&mut c, run_id, domain_step(Domain::D5, Some(0)));
        assert!(matches!(
            save_domain(&mut c, d5, &wrong),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// Continue sets back a failed domain and its own outcome's escalation,
    /// with that escalation's result gone; the other outcome's stays.
    #[test]
    fn reset_sets_back_a_domain_and_its_escalation() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d3 = domain_step(Domain::D3, Some(0));
        save_pending(&mut c, &p, cid, run_id, &[d3]);
        let d3_id = step_id(&mut c, run_id, d3);
        let failure = StepFailure {
            message: "unreadable".into(),
            retryable: false,
            status: None,
        };
        fail_step(&mut c, d3_id, &failure).unwrap();
        let e0 = step_id(&mut c, run_id, escalation_step(0));
        assert!(stored_outcome_result(&mut c, e0).unwrap().is_some());
        assert_eq!(reset_for_continue(&mut c, run_id).unwrap(), 2);
        assert_eq!(status_of(&mut c, run_id, d3), StepStatus::Pending);
        assert_eq!(
            status_of(&mut c, run_id, escalation_step(0)),
            StepStatus::Pending
        );
        assert_eq!(
            status_of(&mut c, run_id, escalation_step(1)),
            StepStatus::Done
        );
        assert_eq!(stored_outcome_result(&mut c, e0).unwrap(), None);
        let e1 = step_id(&mut c, run_id, escalation_step(1));
        assert!(stored_outcome_result(&mut c, e1).unwrap().is_some());
        let kept = stored(&mut c, run_id);
        let d3_row = kept.steps.iter().find(|s| s.id == d3_id).unwrap();
        assert_eq!(d3_row.failure, Some(failure), "kept until it is saved");
        assert_eq!(reset_for_continue(&mut c, run_id).unwrap(), 0);
    }

    /// D1 set back sets back every outcome's escalation, failed or done.
    #[test]
    fn reset_of_d1_sets_back_every_escalation() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d1 = domain_step(Domain::D1, None);
        save_pending(&mut c, &p, cid, run_id, &[d1, escalation_step(1)]);
        let d1_id = step_id(&mut c, run_id, d1);
        let failed = DomainResult::Failed {
            domain: Domain::D1,
            reason: "no reply".into(),
            retryable: false,
            status: None,
        };
        save_domain(&mut c, d1_id, &failed).unwrap();
        let e1 = step_id(&mut c, run_id, escalation_step(1));
        let o = outcome(&mut c, &p, cid, 1);
        let refused = Rob2Escalation::Failed {
            reason: "unreadable".into(),
            retryable: false,
            status: None,
        };
        save_escalation(&mut c, e1, &refused, &weighed(&o, &refused)).unwrap();
        assert_eq!(
            status_of(&mut c, run_id, escalation_step(1)),
            StepStatus::Failed
        );
        assert!(stored_outcome_result(&mut c, e1).unwrap().is_some(), "kept");
        assert_eq!(reset_for_continue(&mut c, run_id).unwrap(), 3);
        for step in [d1, escalation_step(0), escalation_step(1)] {
            assert_eq!(status_of(&mut c, run_id, step), StepStatus::Pending);
        }
        assert_eq!(count(&mut c, "evidence_outcome_results"), 0);
        assert_eq!(count(&mut c, "evidence_ladder_steps"), 0);
    }

    /// An escalation that is not the one its outcome's RoB 2 was combined
    /// with is refused, and nothing is written.
    #[test]
    fn a_mismatched_escalation_is_refused() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        save_pending(&mut c, &p, cid, run_id, &[escalation_step(0)]);
        let e0 = step_id(&mut c, run_id, escalation_step(0));
        let o = outcome(&mut c, &p, cid, 0);
        let refused = Rob2Escalation::Failed {
            reason: "unreadable".into(),
            retryable: false,
            status: None,
        };
        let e = save_escalation(&mut c, e0, &asked(), &weighed(&o, &refused)).unwrap_err();
        assert!(
            matches!(&e, StoreError::Corrupt(m) if m.contains("combined with")),
            "{e}"
        );
        assert_eq!(
            status_of(&mut c, run_id, escalation_step(0)),
            StepStatus::Pending
        );
        assert!(stored_outcome_result(&mut c, e0).unwrap().is_none());
    }

    /// A D5 whose second look failed goes back to pending with its rows gone,
    /// and so does its outcome's escalation.
    #[test]
    fn reset_sets_back_a_d5_with_a_failed_look() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d5_0 = domain_step(Domain::D5, Some(0));
        save_pending(&mut c, &p, cid, run_id, &[d5_0]);
        let id = step_id(&mut c, run_id, d5_0);
        let named = cite(&mut c, &p, cid, "Para two.");
        save_domain(&mut c, id, &d5(&outcome_name(0), None, Some(named))).unwrap();
        let measurements = count(&mut c, "evidence_d5_measurements");
        assert_eq!(measurements, 2);
        assert_eq!(reset_for_continue(&mut c, run_id).unwrap(), 2);
        assert_eq!(status_of(&mut c, run_id, d5_0), StepStatus::Pending);
        assert_eq!(
            status_of(&mut c, run_id, escalation_step(0)),
            StepStatus::Pending
        );
        assert_eq!(
            status_of(&mut c, run_id, escalation_step(1)),
            StepStatus::Done
        );
        assert_eq!(stored_domain(&mut c, id).unwrap(), None);
        assert_eq!(count(&mut c, "evidence_d5_measurements"), 0);
        assert_eq!(count(&mut c, "evidence_d5_facts"), 8, "outcome 1's D5 only");
    }

    /// A run completes only once its kind is known, no step is pending or
    /// failed, and no D5 holds a failed second look.
    #[test]
    fn finish_needs_every_step_done_and_no_failed_look() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let fresh = create_run(&mut c, &p, cid, "m", None).unwrap();
        assert!(!finish_run(&mut c, fresh).unwrap());
        assert!(matches!(
            finish_run(&mut c, fresh + 1000),
            Err(StoreError::NotFound(_))
        ));
        let d5_1 = domain_step(Domain::D5, Some(1));
        save_pending(&mut c, &p, cid, run_id, &[d5_1]);
        assert!(!finish_run(&mut c, run_id).unwrap(), "D5 pending");
        let id = step_id(&mut c, run_id, d5_1);
        let named = cite(&mut c, &p, cid, "Para two.");
        save_domain(&mut c, id, &d5(&outcome_name(1), None, Some(named))).unwrap();
        assert!(!finish_run(&mut c, run_id).unwrap(), "a failed look");
        assert_eq!(stored(&mut c, run_id).status, RunStatus::Incomplete);
        reset_for_continue(&mut c, run_id).unwrap();
        save_pending(&mut c, &p, cid, run_id, &[]);
        assert!(finish_run(&mut c, run_id).unwrap());
        assert_eq!(stored(&mut c, run_id).status, RunStatus::Complete);
    }

    /// `first_shown_at` is set the first time a run with a done domain is
    /// shown, never before and never again.
    #[test]
    fn first_shown_is_set_once_and_only_after_a_domain() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let run_id = create_run(&mut c, &p, cid, "m", None).unwrap();
        assert!(!mark_shown(&mut c, run_id).unwrap());
        let rob2: Vec<EvidenceStep> = plan_rob2(2).into_iter().map(EvidenceStep::Rob2).collect();
        save_pending(&mut c, &p, cid, run_id, &rob2);
        assert!(!mark_shown(&mut c, run_id).unwrap(), "design and outcomes");
        assert_eq!(stored(&mut c, run_id).first_shown_at, None);
        let d1 = pending_steps(&mut c, run_id).unwrap().remove(0);
        save_step(&mut c, &p, cid, &d1);
        assert!(mark_shown(&mut c, run_id).unwrap());
        let first = stored(&mut c, run_id).first_shown_at;
        assert!(first.is_some());
        assert!(!mark_shown(&mut c, run_id).unwrap());
        assert_eq!(stored(&mut c, run_id).first_shown_at, first);
    }

    /// A database at version 16 gains the appraisal tables and
    /// `evidence_runs.appraisal_id`: a run stored before migration 17 reads
    /// back unpaired (listed, latest, its shown time kept), and an appraisal
    /// started on the upgraded database is not blind.
    #[test]
    fn migration_17_on_a_v16_database() {
        let mut c = bmlib::db::open_memory().unwrap();
        bmlib::db::run_migrations(
            &mut c,
            crate::schema::migrations().into_iter().take(16).collect(),
        )
        .unwrap();
        assert!(execute(&mut c, "SELECT appraisal_id FROM evidence_runs", &[]).is_err());
        let (p, cid) = paper_with_text(&mut c);
        // As version 16's code stored it: no appraisal_id column yet.
        execute(
            &mut c,
            "INSERT INTO evidence_runs (paper_id, canonical_id, model, design_prompt_version,
               outcome_prompt_version, rob2_prompt_version, algorithm_version, ladder_version,
               ladder_settings_json, status, first_shown_at)
             VALUES (?, ?, 'm', 1, 1, 1, 'a', 'l', '{}', 'incomplete', datetime('now'))",
            &[Value::from(p.as_str()), Value::from(cid)],
        )
        .unwrap();
        let run_id = fetch_one(&mut c, "SELECT max(id) AS id FROM evidence_runs", &[])
            .unwrap()
            .unwrap()
            .get_i64("id")
            .unwrap();
        bmlib::db::run_migrations(&mut c, crate::schema::migrations()).unwrap();
        assert_eq!(latest_run(&mut c, &p, cid).unwrap(), Some(run_id));
        assert!(any_shown(&mut c, &p).unwrap());
        assert_eq!(runs(&mut c, &p).unwrap().len(), 1);
        let a = crate::appraisal_store::start_appraisal(&mut c, &p).unwrap();
        let row = fetch_one(
            &mut c,
            "SELECT blind FROM evidence_appraisals WHERE id = ?",
            &[Value::from(a)],
        )
        .unwrap()
        .unwrap();
        assert_eq!(row.get_i64("blind").unwrap(), 0);
    }

    /// An unpaired run shown while a blind appraisal is open and not yet
    /// revealed leaves that appraisal not blind; a run with no RoB 2 result
    /// yet leaves it as it was.
    #[test]
    fn showing_an_unpaired_run_unblinds_the_open_appraisal() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let run_id = create_run(&mut c, &p, cid, "m", None).unwrap();
        let a = crate::appraisal_store::start_appraisal(&mut c, &p).unwrap();
        let blind = |c: &mut dyn Db| {
            fetch_one(
                c,
                "SELECT blind FROM evidence_appraisals WHERE id = ?",
                &[Value::from(a)],
            )
            .unwrap()
            .unwrap()
            .get_i64("blind")
            .unwrap()
        };
        assert_eq!(blind(&mut c), 1);
        assert!(!mark_shown(&mut c, run_id).unwrap());
        assert_eq!(blind(&mut c), 1, "nothing on show yet");
        let rob2: Vec<EvidenceStep> = plan_rob2(2).into_iter().map(EvidenceStep::Rob2).collect();
        save_pending(&mut c, &p, cid, run_id, &rob2);
        let d1 = pending_steps(&mut c, run_id).unwrap().remove(0);
        save_step(&mut c, &p, cid, &d1);
        assert!(mark_shown(&mut c, run_id).unwrap());
        assert_eq!(blind(&mut c), 0);
    }

    /// `too_long` reads false on a new run, true once marked and false again
    /// once cleared, and a run not stored is `NotFound`.
    #[test]
    fn too_long_is_marked() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let run_id = create_run(&mut c, &p, cid, "m", None).unwrap();
        assert!(!stored(&mut c, run_id).too_long);
        mark_too_long(&mut c, run_id).unwrap();
        assert!(stored(&mut c, run_id).too_long);
        assert!(matches!(
            mark_too_long(&mut c, run_id + 1),
            Err(StoreError::NotFound(_))
        ));
        clear_too_long(&mut c).unwrap();
        assert!(!stored(&mut c, run_id).too_long);
    }

    /// A run made under other rules says so; the list, the latest run and
    /// the run's text are read as PRISMA's are.
    #[test]
    fn runs_are_listed_and_earlier_rules_flagged() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        assert_eq!(latest_run(&mut c, &p, cid).unwrap(), None);
        let first = create_run(&mut c, &p, cid, "m", None).unwrap();
        let second = create_run(&mut c, &p, cid, "n", None).unwrap();
        assert_eq!(latest_run(&mut c, &p, cid).unwrap(), Some(second));
        let listed: Vec<(i64, String)> = runs(&mut c, &p)
            .unwrap()
            .into_iter()
            .map(|r| (r.id, r.model))
            .collect();
        assert_eq!(listed, vec![(second, "n".into()), (first, "m".into())]);
        assert_eq!(run_text(&mut c, first).unwrap(), Some((p.clone(), cid)));
        assert_eq!(run_text(&mut c, second + 1).unwrap(), None);
        assert_eq!(run(&mut c, second + 1).unwrap(), None);
        let got = stored(&mut c, first);
        assert_eq!(got.versions, RunVersions::current());
        assert_eq!(
            got.text,
            canonical_by_id(&mut c, &p, cid).unwrap().unwrap().text
        );
        for column in ["algorithm_version", "ladder_settings_json"] {
            execute(
                &mut c,
                &format!("UPDATE evidence_runs SET {column} = 'earlier' WHERE id = ?"),
                &[Value::from(first)],
            )
            .unwrap();
            assert!(stored(&mut c, first).earlier_rules, "{column}");
        }
        assert!(!stored(&mut c, second).earlier_rules);
        assert!(matches!(
            create_run(&mut c, &p, cid + 1, "m", None),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// Deleting the paper deletes its runs and every row they hold.
    #[test]
    fn deleting_paper_cascades() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d5_0 = domain_step(Domain::D5, Some(0));
        save_pending(&mut c, &p, cid, run_id, &[d5_0]);
        let id = step_id(&mut c, run_id, d5_0);
        let quote = cite(&mut c, &p, cid, "Para one.");
        let named = cite(&mut c, &p, cid, "Para two.");
        save_domain(&mut c, id, &d5(&outcome_name(0), Some(quote), Some(named))).unwrap();
        for table in TABLES {
            assert!(count(&mut c, table) > 0, "{table}");
        }
        execute(
            &mut c,
            "DELETE FROM papers WHERE id = ?",
            &[Value::from(p.as_str())],
        )
        .unwrap();
        for table in TABLES {
            assert_eq!(count(&mut c, table), 0, "{table}");
        }
    }

    /// A refused save writes nothing: a design or outcome citing outside
    /// the run's text, a design of another prompt version, a D5 without its
    /// facts. Each step stays pending and no row of its result (nor a step
    /// it would add) is stored.
    #[test]
    fn save_refusals_write_nothing() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let len = stored_text_len(&mut c, &p, cid);
        let outside = |mut citation: PicoCitation| {
            citation.start = len + 10;
            citation.end = len + 20;
            citation
        };
        let run_id = create_run(&mut c, &p, cid, "m", None).unwrap();
        let design_id = pending_steps(&mut c, run_id).unwrap()[0].id;
        let mut bad = trial(&mut c, &p, cid);
        bad.citations[0] = outside(bad.citations[0].clone());
        let mut other_version = trial(&mut c, &p, cid);
        other_version.prompt_version = DESIGN_PROMPT_VERSION + 1;
        for proposal in [bad, other_version] {
            assert!(matches!(
                save_design(&mut c, design_id, &proposal),
                Err(StoreError::Corrupt(_))
            ));
            let got = stored(&mut c, run_id);
            assert_eq!(got.steps.len(), 1, "no outcomes step added");
            assert_eq!(got.steps[0].status, StepStatus::Pending);
            assert_eq!(count(&mut c, "evidence_designs"), 0);
            assert_eq!(count(&mut c, "evidence_citations"), 0);
        }

        let d = trial(&mut c, &p, cid);
        save_design(&mut c, design_id, &d).unwrap();
        let citations = count(&mut c, "evidence_citations");
        let outcomes_id = pending_steps(&mut c, run_id).unwrap()[0].id;
        let mut bad = outcomes(&mut c, &p, cid, 2);
        bad.outcomes[1].citations[0] = outside(bad.outcomes[1].citations[0].clone());
        assert!(matches!(
            save_outcomes(&mut c, outcomes_id, &bad),
            Err(StoreError::Corrupt(_))
        ));
        let got = stored(&mut c, run_id);
        assert_eq!((got.kind, got.steps.len()), (None, 2), "no RoB 2 steps");
        assert_eq!(
            status_of(&mut c, run_id, EvidenceStep::Outcomes),
            StepStatus::Pending
        );
        assert_eq!(count(&mut c, "evidence_outcomes"), 0, "outcome 0 not kept");
        assert_eq!(count(&mut c, "evidence_citations"), citations);

        let mut c = open_memory_store().unwrap();
        let (_, _, run_id) = assessed_run(&mut c);
        let d5_0 = domain_step(Domain::D5, Some(0));
        let id = step_id(&mut c, run_id, d5_0);
        let domains = count(&mut c, "evidence_domains");
        assert!(matches!(
            save_domain(&mut c, id, &assessed(Domain::D5, Answer::Y, None)),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(status_of(&mut c, run_id, d5_0), StepStatus::Pending);
        assert_eq!(count(&mut c, "evidence_domains"), domains);
    }

    /// The length, in code points, of paper `p`'s text `cid`.
    fn stored_text_len(c: &mut Connection, p: &PaperId, cid: i64) -> usize {
        let text = crate::store::canonical_by_id(c, p, cid)
            .unwrap()
            .unwrap()
            .text;
        text.text.chars().count()
    }

    /// Continue's reset leaves no row of a step it sets back, in any result
    /// table (citations included, which a rewritten row could otherwise
    /// pick up by its reused id); `RESULT_TABLES` is every result table.
    #[test]
    fn reset_leaves_no_row_of_the_step() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d5_0 = domain_step(Domain::D5, Some(0));
        save_pending(&mut c, &p, cid, run_id, &[d5_0]);
        let id = step_id(&mut c, run_id, d5_0);
        let quote = cite(&mut c, &p, cid, "Para one.");
        let named = cite(&mut c, &p, cid, "Para two.");
        save_domain(&mut c, id, &d5(&outcome_name(0), Some(quote), Some(named))).unwrap();
        let e0 = step_id(&mut c, run_id, escalation_step(0));
        assert_eq!(reset_for_continue(&mut c, run_id).unwrap(), 2);
        let mut results: Vec<&str> = crate::evidence_rows::RESULT_TABLES.to_vec();
        let mut expected: Vec<&str> = TABLES[2..].to_vec();
        results.sort_unstable();
        expected.sort_unstable();
        assert_eq!(results, expected);
        for table in TABLES[2..].iter() {
            for step in [id, e0] {
                let n = fetch_one(
                    &mut c,
                    &format!("SELECT count(*) AS n FROM {table} WHERE step_id = ?"),
                    &[Value::from(step)],
                )
                .unwrap()
                .unwrap()
                .get_i64("n")
                .unwrap();
                assert_eq!(n, 0, "{table}, step {step}");
            }
        }
    }

    /// Every table of migration 15.
    const TABLES: [&str; 12] = [
        "evidence_runs",
        "evidence_steps",
        "evidence_designs",
        "evidence_outcomes",
        "evidence_domains",
        "evidence_answers",
        "evidence_d5_facts",
        "evidence_d5_measurements",
        "evidence_d5_analyses",
        "evidence_outcome_results",
        "evidence_ladder_steps",
        "evidence_citations",
    ];

    /// Only a stored citation makes a text a dependent.
    #[test]
    fn has_citations_counts_only_citations() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let run_id = create_run(&mut c, &p, cid, "m", None).unwrap();
        let mut uncited = trial(&mut c, &p, cid);
        uncited.citations.clear();
        let design_step = pending_steps(&mut c, run_id).unwrap()[0].id;
        save_design(&mut c, design_step, &uncited).unwrap();
        assert!(!has_citations(&mut c, cid).unwrap());
        save_pending(&mut c, &p, cid, run_id, &[]);
        assert!(has_citations(&mut c, cid).unwrap());
        assert!(!has_citations(&mut c, cid + 1).unwrap());
    }
}
