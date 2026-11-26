//! Running the evidence weight on a paper (design 2026-11-24, §5–§6): the
//! study design, the primary outcomes, then RoB 2 a domain at a time and each
//! outcome's overall judgement and ladder, every step stored the moment it
//! finishes ([`crate::evidence_store`]). bmlib's agents ask, check the quotes
//! and condense a long paper; this sizes their budget, retries, honours a
//! stop, and runs the stored steps until none is pending. The store adds the
//! next steps as it saves the design and the outcomes, so this loop only ever
//! runs what is pending. Continue ([`continue_evidence`]) runs what an earlier
//! call left. It follows `assess_unattended`'s order and rules, so a run
//! driven here ends where bmlib's own pipeline would.

use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use bmlib::context_processor::llm_processor::ContextModel;
use bmlib::context_processor::ProcessingConfig;
use bmlib::db::Db;
use bmlib::fulltext::canonical::CanonicalText;
use bmlib::quality::{
    assess_outcome, combine_outcome, design_prompt_chars, domain_prompt_chars,
    outcome_prompt_chars, start_level, DesignAgent, Domain, DomainResult, Escalation, JsonChat,
    Judgement, LadderSettings, LlmQueryCondenser, OutcomeAgent, ProposalError, ProposedOutcome,
    Rob2Agent, Rob2Digests, Rob2Escalation, Rob2RunError, Rob2Step,
};
use serde::Serialize;

use crate::config::{
    EVIDENCE_MIN_BUDGET_CHARS, EVIDENCE_TEMPERATURE, MIN_QUOTE_SIMILARITY, RETRIEVAL_ATTEMPTS,
};
use crate::evidence_rows::{
    proposed_outcome, rob2_inputs, stored_design, stored_outcome_result, stored_outcomes,
    study_design, ESCALATION_ASKED, ESCALATION_NOT_NEEDED,
};
use crate::evidence_store::{
    create_run, fail_step, finish_run, mark_too_long, pending_steps, reset_for_continue,
    run as stored_run, save_design, save_domain, save_escalation, save_outcomes, EvidenceStep,
    RunStatus, StepFailure, StepStatus, StoredEvidenceRun,
};
use crate::interrogate::{full_text, AskError};
use crate::pico::{is_set, paper_budget_chars, PicoEvent, PicoRunError};
use crate::prisma::title_of;
use crate::retry::with_backoff;
use crate::store::{PaperId, StoreError};

/// The design step's progress label.
const DESIGN_LABEL: &str = "Study design";

/// The outcomes step's progress label.
const OUTCOMES_LABEL: &str = "Primary outcomes";

/// What a run made under earlier rules says, and why Continue refuses it.
pub const EARLIER_RULES: &str = "Made under earlier rules; Re-run for the current ones.";

/// What an evidence weight run works with: [`crate::prisma::PrismaContext`]'s
/// fields, with the evidence weight's events.
pub struct EvidenceContext<'a> {
    /// The store.
    pub db: &'a mut dyn Db,
    /// The model, for every step.
    pub chat: &'a mut dyn JsonChat,
    /// The model, for condensing a long paper.
    pub condenser_model: &'a dyn ContextModel,
    /// The chosen `provider:model`, stored with a new run. Continue passes
    /// the run's own.
    pub model: String,
    /// The model's window in tokens, `None` when unknown.
    pub window: Option<u64>,
    /// The room for each reply in tokens: every call's `max_tokens`, and what
    /// the paper's budget keeps free (`prompt::reply_tokens`).
    pub reply_tokens: u64,
    /// The first backoff wait between attempts (the command passes
    /// `RETRY_BASE_DELAY`, tests zero).
    pub retry_delay: Duration,
    /// Told which step is running and how far a condensation has got.
    pub on_event: Arc<dyn Fn(EvidenceEvent) + Send + Sync>,
}

/// What the user sees while an evidence weight run works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceEvent {
    /// A step has begun; `label` is [`step_label`]'s.
    Step {
        /// What the step assesses, as the tab shows it.
        label: String,
    },
    /// Batch `batch` of `batches` of a condensation is being condensed; as
    /// [`PicoEvent::Condensing`].
    Condensing {
        /// The batch being condensed, from 1.
        batch: usize,
        /// The batches at this level.
        batches: usize,
        /// How many times the batches have been condensed already.
        level: usize,
    },
    /// A condensation has ended, whether it succeeded or failed.
    Condensed,
    /// A condensation stage this version does not know, shown as bmlib
    /// words it.
    Other {
        /// bmlib's name for the stage.
        stage: String,
        /// bmlib's description of it.
        message: String,
    },
}

impl From<PicoEvent> for EvidenceEvent {
    /// The same condensation progress, as the evidence weight's event: PICO's
    /// mapping of bmlib's stages ([`PicoEvent::from_progress`]) serves every
    /// tab.
    fn from(event: PicoEvent) -> Self {
        match event {
            PicoEvent::Condensing {
                batch,
                batches,
                level,
            } => EvidenceEvent::Condensing {
                batch,
                batches,
                level,
            },
            PicoEvent::Condensed => EvidenceEvent::Condensed,
            PicoEvent::Other { stage, message } => EvidenceEvent::Other { stage, message },
        }
    }
}

/// What a RoB 2 domain assesses, as a progress label names it.
fn domain_words(domain: Domain) -> &'static str {
    match domain {
        Domain::D1 => "D1, the randomisation process",
        Domain::D2 => "D2, deviations from the intended interventions",
        Domain::D3 => "D3, missing outcome data",
        Domain::D4 => "D4, measurement of the outcome",
        Domain::D5 => "D5, selection of the reported result",
    }
}

/// The progress label of `step` in a run with `outcomes` primary outcomes:
/// *Study design*, *Primary outcomes*, *The trial: D1, the randomisation
/// process*, then *Outcome {k} of {n}: …* for each outcome's domains and its
/// overall judgement, the outcome counted from 1.
pub fn step_label(step: &EvidenceStep, outcomes: usize) -> String {
    match step {
        EvidenceStep::Design => DESIGN_LABEL.to_string(),
        EvidenceStep::Outcomes => OUTCOMES_LABEL.to_string(),
        EvidenceStep::Rob2(Rob2Step::Domain {
            domain,
            outcome: None,
        }) => format!("The trial: {}", domain_words(*domain)),
        EvidenceStep::Rob2(Rob2Step::Domain {
            domain,
            outcome: Some(o),
        }) => format!("Outcome {} of {outcomes}: {}", o + 1, domain_words(*domain)),
        EvidenceStep::Rob2(Rob2Step::Escalation { outcome }) => {
            format!(
                "Outcome {} of {outcomes}: the overall judgement",
                outcome + 1
            )
        }
    }
}

/// Why a call stopped short. A run already started keeps every step it
/// finished, whatever the error.
#[derive(Debug)]
pub enum EvidenceRunError {
    /// The paper has no canonical text.
    NoFullText,
    /// The text neither passed the full-text check nor was confirmed.
    NotFullText,
    /// The window leaves no room for the paper; `window` is the reported one,
    /// `None` when it was assumed.
    TooLong {
        /// The reported window, or `None` when assumed.
        window: Option<u64>,
    },
    /// The user stopped the run.
    Cancelled,
    /// Continue was asked of a run made under earlier rules ([`EARLIER_RULES`]).
    EarlierRules,
    /// The design or the outcomes could not be had, after the retries; the
    /// step is stored as failed.
    Proposal {
        /// The step's label.
        step: &'static str,
        /// bmlib's error.
        error: ProposalError,
    },
    /// A domain or an outcome's overall judgement failed on a request, after
    /// the retries: the next step would meet the same server. The step is
    /// stored as failed.
    Request {
        /// The step's label ([`step_label`]).
        label: String,
        /// What failed, in the transport's or the server's words.
        message: String,
        /// Whether sending the request again may succeed.
        retryable: bool,
        /// The HTTP status it was answered with, if any.
        status: Option<u16>,
    },
    /// The store failed, or holds a row bmlib refuses.
    Store(StoreError),
    /// A defect in this code or bmlib's prompts: Continue asked of a complete
    /// run, a prompt bmlib cannot measure, or a call bmlib refused.
    Internal(String),
}

impl From<StoreError> for EvidenceRunError {
    /// A store failure as the run's error.
    fn from(e: StoreError) -> Self {
        EvidenceRunError::Store(e)
    }
}

impl fmt::Display for EvidenceRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The refusals PICO shares read as PICO's, so every tab says the same.
        let mut pico = |e: PicoRunError| write!(f, "{e}");
        match self {
            EvidenceRunError::NoFullText => pico(PicoRunError::NoFullText),
            EvidenceRunError::NotFullText => pico(PicoRunError::NotFullText),
            EvidenceRunError::TooLong { window } => pico(PicoRunError::TooLong { window: *window }),
            EvidenceRunError::Cancelled => {
                write!(f, "The evidence weight assessment was stopped.")
            }
            EvidenceRunError::EarlierRules => write!(f, "{EARLIER_RULES}"),
            EvidenceRunError::Proposal { step, error } => write!(f, "{step}: {error}"),
            EvidenceRunError::Request { label, message, .. } => write!(f, "{label}: {message}"),
            EvidenceRunError::Store(e) => write!(f, "{e}"),
            EvidenceRunError::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EvidenceRunError {}

/// The characters of paper a run can send to a model with `window` tokens,
/// after `reply` tokens of room for the reply, the title, and the longest
/// prompt a step sends: the design's, the outcomes', D1's, and D2–D5's for
/// each of `outcomes` (none before the outcomes are known). `Ok(None)` when
/// that is under [`EVIDENCE_MIN_BUDGET_CHARS`]; a prompt bmlib cannot measure
/// is [`EvidenceRunError::Internal`].
pub fn budget_chars(
    window: Option<u64>,
    reply: u64,
    title: &str,
    outcomes: &[ProposedOutcome],
) -> Result<Option<usize>, EvidenceRunError> {
    let measured = |domain: Domain, outcome: Option<&ProposedOutcome>| {
        domain_prompt_chars(domain, outcome).map_err(EvidenceRunError::Internal)
    };
    let mut longest = design_prompt_chars()
        .max(outcome_prompt_chars())
        .max(measured(Domain::D1, None)?);
    for outcome in outcomes {
        for &domain in &Domain::ALL[1..] {
            longest = longest.max(measured(domain, Some(outcome))?);
        }
    }
    Ok(paper_budget_chars(
        window,
        reply,
        longest,
        title,
        EVIDENCE_MIN_BUDGET_CHARS,
    ))
}

/// Starts an evidence weight run on `paper`'s latest text and returns its id.
/// The paper's budget (with no outcomes known yet) is sized before any call
/// or row, so a window too small is refused as [`EvidenceRunError::TooLong`]
/// with nothing stored, and so is a stop already set. Then the run's steps
/// run in plan order, each stored as it finishes: the design, the outcomes,
/// D1, and each outcome's D2–D5 and overall judgement. Each outcome's D2–D5
/// prompts carry that outcome, so once the outcomes are stored a window too
/// small for them stops the run as [`EvidenceRunError::TooLong`], marked
/// `too_long`.
///
/// A stop or a failure keeps every step finished (a reply that arrives as
/// Stop is pressed included) and returns [`EvidenceRunError::Cancelled`] or
/// the failure; unless that reply finished the run, it is left `incomplete`
/// for [`continue_evidence`]. A domain
/// or overall judgement that failed other than on a request (an unreadable
/// reply, a failed condensation) is stored as failed and the rest run; the
/// run's id comes back with the run `incomplete`.
pub fn run_evidence(
    ctx: &mut EvidenceContext,
    paper: &PaperId,
    stop: &AtomicBool,
) -> Result<i64, EvidenceRunError> {
    let result = run_inner(ctx, paper, stop);
    log_failure(&result, &format!("paper {}", paper.as_str()));
    result
}

/// Continues run `run_id` on its own canonical text, even if the paper has a
/// newer one, with `ctx.model` (the command passes the run's). Refused with
/// nothing changed: a complete run ([`EvidenceRunError::Internal`]), one made
/// under earlier rules ([`EvidenceRunError::EarlierRules`]), a stop already
/// set ([`EvidenceRunError::Cancelled`]), a window too small for the run's
/// stored outcomes ([`EvidenceRunError::TooLong`], the run marked so, as in
/// [`run_evidence`]), and a stored overall
/// judgement that no longer fits its domains ([`StoreError::Corrupt`]).
/// Otherwise its failed steps, and D5s with a failed second look, are set
/// back to pending (`reset_for_continue`, which sets back their escalations
/// too), and every pending step runs as in [`run_evidence`]. No such run is
/// [`StoreError::NotFound`].
pub fn continue_evidence(
    ctx: &mut EvidenceContext,
    run_id: i64,
    stop: &AtomicBool,
) -> Result<i64, EvidenceRunError> {
    let result = continue_inner(ctx, run_id, stop);
    log_failure(&result, &format!("evidence run {run_id}"));
    result
}

/// Logs a call's failure as [`crate::prisma::run_prisma`] does: a stop as
/// info, a store failure or defect as an error, anything else as a warning.
fn log_failure(result: &Result<i64, EvidenceRunError>, what: &str) {
    match result {
        Ok(_) => {}
        Err(EvidenceRunError::Cancelled) => log::info!("{what}: evidence weight run stopped"),
        Err(e @ (EvidenceRunError::Store(_) | EvidenceRunError::Internal(_))) => {
            log::error!("{what}: evidence weight run failed: {e}")
        }
        Err(e) => log::warn!("{what}: evidence weight run failed: {e}"),
    }
}

/// [`run_evidence`] without the logging.
fn run_inner(
    ctx: &mut EvidenceContext,
    paper: &PaperId,
    stop: &AtomicBool,
) -> Result<i64, EvidenceRunError> {
    let stored = full_text(ctx.db, paper).map_err(|e| match e {
        AskError::NoFullText => EvidenceRunError::NoFullText,
        AskError::NotFullText => EvidenceRunError::NotFullText,
        AskError::Store(e) => EvidenceRunError::Store(e),
        other => EvidenceRunError::Internal(other.to_string()),
    })?;
    let canonical = &stored.text;
    budget_chars(ctx.window, ctx.reply_tokens, title_of(canonical), &[])?
        .ok_or(EvidenceRunError::TooLong { window: ctx.window })?;
    cancelled(stop)?;
    let model = ctx.model.clone();
    let run_id = create_run(ctx.db, paper, stored.id, &model)?;
    drive(ctx, run_id, canonical, stop)?;
    Ok(run_id)
}

/// [`continue_evidence`] without the logging.
fn continue_inner(
    ctx: &mut EvidenceContext,
    run_id: i64,
    stop: &AtomicBool,
) -> Result<i64, EvidenceRunError> {
    let run = stored_run(ctx.db, run_id)?
        .ok_or_else(|| StoreError::NotFound(format!("evidence run {run_id}")))?;
    if run.status == RunStatus::Complete {
        return Err(EvidenceRunError::Internal(format!(
            "evidence run {run_id} is complete: there is nothing to continue"
        )));
    }
    if run.earlier_rules {
        return Err(EvidenceRunError::EarlierRules);
    }
    // A stop already set keeps the failed steps' messages.
    cancelled(stop)?;
    // Sized before anything is set back, so a window too small changes
    // nothing but the run's mark.
    let outcomes = run_outcomes(ctx.db, run_id, &run.text)?;
    if budget_chars(ctx.window, ctx.reply_tokens, title_of(&run.text), &outcomes)?.is_none() {
        return Err(too_long(ctx, run_id));
    }
    check_escalations(ctx.db, &run)?;
    reset_for_continue(ctx.db, run_id)?;
    drive(ctx, run_id, &run.text, stop)?;
    Ok(run_id)
}

/// Checks that every done overall judgement of `run` still fits its stored
/// domains: each is combined again (`combine_outcome`) from the stored
/// domains and escalation, which must be accepted and give the stored
/// overall. Anything else is [`StoreError::Corrupt`], naming the run and the
/// outcome. Continue checks this before it changes anything, as it keeps
/// these results.
fn check_escalations(db: &mut dyn Db, run: &StoredEvidenceRun) -> Result<(), EvidenceRunError> {
    for step in &run.steps {
        let EvidenceStep::Rob2(Rob2Step::Escalation { outcome }) = step.step else {
            continue;
        };
        if step.status != StepStatus::Done {
            continue;
        }
        let bad = |why: String| {
            EvidenceRunError::Store(StoreError::Corrupt(format!(
                "evidence run {}: outcome {outcome}'s overall judgement {why}",
                run.id
            )))
        };
        let stored =
            stored_outcome_result(db, step.id)?.ok_or_else(|| bad("is not stored".to_string()))?;
        let escalation = match (stored.escalation.as_str(), stored.escalate) {
            (ESCALATION_NOT_NEEDED, _) => Rob2Escalation::NotNeeded,
            (ESCALATION_ASKED, Some(escalate)) => Rob2Escalation::Asked(Escalation {
                escalate,
                reasoning: stored.escalation_reasoning.clone().unwrap_or_default(),
            }),
            (other, _) => return Err(bad(format!("has escalation {other:?} on a done step"))),
        };
        let (proposed, study, domains) = rob2_inputs(db, run.id, outcome, &run.text)?;
        let combined = combine_outcome(&proposed, &study, domains, escalation)
            .map_err(|e| bad(format!("does not fit its domains: {e}")))?;
        let overall = stored
            .overall
            .as_deref()
            .map(|o| serde_json::from_value::<Judgement>(serde_json::Value::from(o)))
            .transpose()
            .map_err(|e| bad(format!("is not a judgement: {e}")))?;
        if overall != combined.overall {
            return Err(bad(format!(
                "is {overall:?}, and its domains give {:?}",
                combined.overall
            )));
        }
    }
    Ok(())
}

/// What every call of a run shares: where its events go, the first backoff
/// wait, the reply room and the stop.
struct Calls<'s> {
    /// The context's listener.
    on_event: Arc<dyn Fn(EvidenceEvent) + Send + Sync>,
    /// The first backoff wait.
    delay: Duration,
    /// Every call's `max_tokens`: the reply room.
    max_tokens: i64,
    /// Set when the user pressed Stop.
    stop: &'s AtomicBool,
}

impl Calls<'_> {
    /// Tells the listener `event`.
    fn emit(&self, event: EvidenceEvent) {
        (self.on_event)(event);
    }

    /// A condenser over `model` whose progress reaches the listener as
    /// events, sampling as the agents do.
    fn condenser<'m>(&self, model: &'m dyn ContextModel) -> LlmQueryCondenser<'m> {
        let on_event = Arc::clone(&self.on_event);
        LlmQueryCondenser::new(model, EVIDENCE_TEMPERATURE, self.max_tokens).with_progress(
            move |info| {
                if let Some(event) = PicoEvent::from_progress(info) {
                    on_event(event.into());
                }
            },
        )
    }

    /// `e` from the design or outcomes step as the run's error:
    /// [`EvidenceRunError::Cancelled`] once stopped (a failure under way
    /// still reaches the log), else [`EvidenceRunError::Proposal`] for step
    /// `label`.
    fn proposal_failed(&self, label: &'static str, e: ProposalError) -> EvidenceRunError {
        if is_set(self.stop) || matches!(e, ProposalError::Stopped) {
            if !matches!(e, ProposalError::Stopped) {
                log::warn!("{label} failed as it stopped: {e}");
            }
            return EvidenceRunError::Cancelled;
        }
        EvidenceRunError::Proposal {
            step: label,
            error: e,
        }
    }

    /// `e` from a RoB 2 step as the run's error: [`StoreError::Corrupt`] for
    /// domains bmlib refuses (rows read back from the store), even once
    /// stopped; [`EvidenceRunError::Cancelled`] once stopped (a failure under
    /// way still reaches the log); else [`EvidenceRunError::Internal`].
    fn rob2_failed(&self, e: Rob2RunError) -> EvidenceRunError {
        match e {
            Rob2RunError::InvalidDomains(why) => EvidenceRunError::Store(StoreError::Corrupt(why)),
            Rob2RunError::Stopped => EvidenceRunError::Cancelled,
            other if is_set(self.stop) => {
                log::warn!("a RoB 2 step failed as it stopped: {other}");
                EvidenceRunError::Cancelled
            }
            other => EvidenceRunError::Internal(other.to_string()),
        }
    }

    /// Whether a retry may cure `e`: a request that failed but may succeed
    /// again (never a 401 or 404), and only while not stopped.
    fn proposal_retryable(&self, e: &ProposalError) -> bool {
        matches!(
            e,
            ProposalError::RequestFailed {
                retryable: true,
                ..
            }
        ) && !is_set(self.stop)
    }
}

/// Configures one of bmlib's step agents as every step of a run asks: the
/// condenser, the evidence weight's temperature, the reply room as
/// `max_tokens`, `budget` characters of paper and the stop.
macro_rules! configured {
    ($agent:expr, $condenser:expr, $calls:expr, $budget:expr) => {
        $agent
            .with_condenser($condenser)
            .with_temperature(EVIDENCE_TEMPERATURE)
            .with_max_tokens($calls.max_tokens)
            .with_condense_config(ProcessingConfig::default().with_max_context_chars($budget))
            .with_stop($calls.stop)
    };
}

/// One attempt of a RoB 2 call, as [`with_backoff`] sees it.
enum Attempt<T> {
    /// bmlib refused or stopped the call: never retried.
    Refused(Rob2RunError),
    /// A failure value marked retryable (`why`): tried again, and kept if it
    /// is the last attempt's.
    Again {
        /// The failure value.
        value: T,
        /// Its reason, for the retry's log line.
        why: String,
    },
}

impl<T> fmt::Display for Attempt<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Attempt::Refused(e) => write!(f, "{e}"),
            Attempt::Again { why, .. } => write!(f, "{why}"),
        }
    }
}

/// Runs RoB 2 call `call` with the run's retries: a value `again` gives a
/// reason for (one marked retryable) is asked again, while not stopped, up to
/// [`RETRIEVAL_ATTEMPTS`] times, and the last one is returned. bmlib's own
/// refusal is returned at once.
fn retried<T>(
    calls: &Calls,
    again: impl Fn(&T) -> Option<String>,
    mut call: impl FnMut() -> Result<T, Rob2RunError>,
) -> Result<T, Rob2RunError> {
    let result = with_backoff(
        RETRIEVAL_ATTEMPTS,
        calls.delay,
        |e: &Attempt<T>| matches!(e, Attempt::Again { .. }) && !is_set(calls.stop),
        || {
            let value = call().map_err(Attempt::Refused)?;
            match again(&value) {
                Some(why) => Err(Attempt::Again { value, why }),
                None => Ok(value),
            }
        },
    );
    match result {
        Ok(value) | Err(Attempt::Again { value, .. }) => Ok(value),
        Err(Attempt::Refused(e)) => Err(e),
    }
}

/// Whether a failed domain or overall judgement came from a request (the
/// failure may be cured by sending it again, or the server answered with an
/// HTTP status): then the next step would meet the same server, so the call
/// ends. Any other failure (an unreadable reply, a failed condensation) is
/// stored and the run goes on.
fn from_a_request(retryable: bool, status: Option<u16>) -> bool {
    retryable || status.is_some()
}

/// Why `result` is worth asking again: a domain that failed with a retryable
/// request, or a D5 holding an other measurement whose second look did.
fn domain_retry_reason(result: &DomainResult) -> Option<String> {
    match result {
        DomainResult::Failed {
            reason,
            retryable: true,
            ..
        } => Some(reason.clone()),
        DomainResult::Failed { .. } => None,
        DomainResult::Assessed(a) => a
            .facts
            .iter()
            .flat_map(|f| &f.measurements)
            .find(|m| m.second_look_retryable && m.second_look_error.is_some())
            .and_then(|m| m.second_look_error.as_ref())
            .map(|e| format!("a second look failed: {e}")),
    }
}

/// Why `escalation` is worth asking again: it failed with a retryable request.
fn escalation_retry_reason(escalation: &Rob2Escalation) -> Option<String> {
    match escalation {
        Rob2Escalation::Failed {
            reason,
            retryable: true,
            ..
        } => Some(reason.clone()),
        _ => None,
    }
}

/// Run `run_id`'s stored outcomes as bmlib's, in order (none before the
/// outcomes step is done).
fn run_outcomes(
    db: &mut dyn Db,
    run_id: i64,
    canonical: &CanonicalText,
) -> Result<Vec<ProposedOutcome>, StoreError> {
    stored_outcomes(db, run_id)?
        .iter()
        .map(|o| proposed_outcome(o, canonical))
        .collect()
}

/// Runs run `run_id`'s pending steps on `canonical` in plan order until none
/// is left, then marks the run complete if nothing failed. Each step is
/// announced ([`EvidenceEvent::Step`]), sized with the run's stored outcomes
/// ([`budget_chars`]; a window too small is [`EvidenceRunError::TooLong`],
/// the run marked `too_long`),
/// asked with retries and stored at once; the store adds the steps that
/// follow. One set of digests serves every RoB 2 step of this call. A stop
/// (checked before each step, and after a reply, which is kept) ends the
/// call as [`EvidenceRunError::Cancelled`].
fn drive(
    ctx: &mut EvidenceContext,
    run_id: i64,
    canonical: &CanonicalText,
    stop: &AtomicBool,
) -> Result<(), EvidenceRunError> {
    let title = title_of(canonical);
    let calls = Calls {
        on_event: Arc::clone(&ctx.on_event),
        delay: ctx.retry_delay,
        max_tokens: i64::try_from(ctx.reply_tokens).unwrap_or(i64::MAX),
        stop,
    };
    let mut digests = Rob2Digests::default();
    loop {
        let pending = pending_steps(ctx.db, run_id)?;
        let Some(next) = pending.first() else {
            break;
        };
        cancelled(stop)?;
        let outcomes = run_outcomes(ctx.db, run_id, canonical)?;
        let Some(budget) = budget_chars(ctx.window, ctx.reply_tokens, title, &outcomes)? else {
            return Err(too_long(ctx, run_id));
        };
        let step = NextStep {
            id: next.id,
            run_id,
            label: step_label(&next.step, outcomes.len()),
        };
        calls.emit(EvidenceEvent::Step {
            label: step.label.clone(),
        });
        let paper = Paper {
            title,
            canonical,
            budget,
        };
        match next.step {
            EvidenceStep::Design => design_step(ctx, &calls, next.id, &paper)?,
            EvidenceStep::Outcomes => outcomes_step(ctx, &calls, next.id, &paper)?,
            EvidenceStep::Rob2(Rob2Step::Domain { domain, outcome }) => {
                let outcome = match outcome {
                    None => None,
                    Some(i) => Some(outcomes.get(i).ok_or_else(|| {
                        StoreError::Corrupt(format!("evidence run {run_id} has no outcome {i}"))
                    })?),
                };
                domain_step(ctx, &calls, step, domain, outcome, &paper, &mut digests)?;
            }
            EvidenceStep::Rob2(Rob2Step::Escalation { outcome }) => {
                escalation_step(ctx, &calls, step, outcome, &paper)?;
            }
        }
        if is_set(stop) {
            // The reply is kept; a run it finished is marked so first.
            finish_run(ctx.db, run_id)?;
            return Err(EvidenceRunError::Cancelled);
        }
    }
    if !finish_run(ctx.db, run_id)? {
        log::warn!("evidence run {run_id} ended with failed steps; Continue asks them again");
    }
    Ok(())
}

/// Run `run_id`'s stop for a window too small for its stored outcomes:
/// [`EvidenceRunError::TooLong`], with the run marked (`mark_too_long`) so
/// the tab no longer offers a Continue on the same window (#166). A failure
/// to mark it is logged and the window is still the error: it is what the user
/// can act on, and the Continue left offered sizes again and marks again.
fn too_long(ctx: &mut EvidenceContext, run_id: i64) -> EvidenceRunError {
    if let Err(e) = mark_too_long(ctx.db, run_id) {
        log::error!("evidence run {run_id}: too long for its window, but not marked: {e}");
    }
    EvidenceRunError::TooLong { window: ctx.window }
}

/// The paper a step reads: its title, its canonical text, and the characters
/// of it the step may send before it is condensed.
struct Paper<'p> {
    /// The paper's title, or `""`.
    title: &'p str,
    /// The run's canonical text.
    canonical: &'p CanonicalText,
    /// The step's budget ([`budget_chars`]).
    budget: usize,
}

/// Marks step `step_id` failed with `e`'s message and request details, and
/// returns `e` as the run's error ([`Calls::proposal_failed`]). A stop is not
/// a failure: the step stays pending. A store failure is returned, with the
/// model's failure logged so it is not lost.
fn fail_proposal(
    db: &mut dyn Db,
    calls: &Calls,
    step_id: i64,
    label: &'static str,
    e: ProposalError,
) -> EvidenceRunError {
    let (retryable, status) = match &e {
        ProposalError::RequestFailed {
            retryable, status, ..
        } => (*retryable, *status),
        _ => (false, None),
    };
    let message = e.to_string();
    let e = calls.proposal_failed(label, e);
    if !matches!(e, EvidenceRunError::Proposal { .. }) {
        return e;
    }
    let failure = StepFailure {
        message,
        retryable,
        status,
    };
    if let Err(store) = fail_step(db, step_id, &failure) {
        log::warn!("evidence step {step_id} failed: {e}");
        return store.into();
    }
    e
}

/// Proposes and stores the study design (design step `step_id`), retried.
fn design_step(
    ctx: &mut EvidenceContext,
    calls: &Calls,
    step_id: i64,
    paper: &Paper,
) -> Result<(), EvidenceRunError> {
    let mut condenser = calls.condenser(ctx.condenser_model);
    let mut agent = configured!(
        DesignAgent::new(&mut *ctx.chat),
        &mut condenser,
        calls,
        paper.budget
    );
    let proposed = with_backoff(
        RETRIEVAL_ATTEMPTS,
        calls.delay,
        |e| calls.proposal_retryable(e),
        || agent.propose(paper.title, &paper.canonical.text, MIN_QUOTE_SIMILARITY),
    );
    match proposed {
        Ok(design) => Ok(save_design(ctx.db, step_id, &design)?),
        Err(e) => Err(fail_proposal(ctx.db, calls, step_id, DESIGN_LABEL, e)),
    }
}

/// Proposes and stores the primary outcomes (outcomes step `step_id`),
/// retried.
fn outcomes_step(
    ctx: &mut EvidenceContext,
    calls: &Calls,
    step_id: i64,
    paper: &Paper,
) -> Result<(), EvidenceRunError> {
    let mut condenser = calls.condenser(ctx.condenser_model);
    let mut agent = configured!(
        OutcomeAgent::new(&mut *ctx.chat),
        &mut condenser,
        calls,
        paper.budget
    );
    let proposed = with_backoff(
        RETRIEVAL_ATTEMPTS,
        calls.delay,
        |e| calls.proposal_retryable(e),
        || agent.propose(paper.title, &paper.canonical.text, MIN_QUOTE_SIMILARITY),
    );
    match proposed {
        Ok(outcomes) => Ok(save_outcomes(ctx.db, step_id, &outcomes)?),
        Err(e) => Err(fail_proposal(ctx.db, calls, step_id, OUTCOMES_LABEL, e)),
    }
}

/// The step [`drive`] runs next: its row id, the run's id and its progress
/// label ([`step_label`]).
struct NextStep {
    /// The step's row id.
    id: i64,
    /// The run it belongs to.
    run_id: i64,
    /// What the status line and the logs call it.
    label: String,
}

/// Assesses and stores domain step `step` of `domain` for `outcome` (`None`
/// for D1). A failure once stopped is
/// [`EvidenceRunError::Cancelled`], the step left pending; a failure from a
/// request is stored and ends the call as [`EvidenceRunError::Request`]; any
/// other is stored and the run goes on.
fn domain_step(
    ctx: &mut EvidenceContext,
    calls: &Calls,
    step: NextStep,
    domain: Domain,
    outcome: Option<&ProposedOutcome>,
    paper: &Paper,
    digests: &mut Rob2Digests,
) -> Result<(), EvidenceRunError> {
    let NextStep {
        id: step_id, label, ..
    } = step;
    let mut condenser = calls.condenser(ctx.condenser_model);
    let mut agent = configured!(
        Rob2Agent::new(&mut *ctx.chat),
        &mut condenser,
        calls,
        paper.budget
    );
    let result = retried(calls, domain_retry_reason, || {
        agent.assess_domain(
            domain,
            paper.title,
            &paper.canonical.text,
            outcome,
            MIN_QUOTE_SIMILARITY,
            digests,
        )
    })
    .map_err(|e| calls.rob2_failed(e))?;
    if let DomainResult::Failed { reason, .. } = &result {
        if is_set(calls.stop) {
            log::warn!("{label} failed as it stopped: {reason}");
            return Err(EvidenceRunError::Cancelled);
        }
    }
    if let Err(store) = save_domain(ctx.db, step_id, &result) {
        // The model's answer (or its failure's reason) would be lost otherwise.
        log::warn!("{label}: its result could not be stored ({store}): {result:?}");
        return Err(store.into());
    }
    match result {
        DomainResult::Failed {
            reason,
            retryable,
            status,
            ..
        } if from_a_request(retryable, status) => Err(EvidenceRunError::Request {
            label,
            message: reason,
            retryable,
            status,
        }),
        DomainResult::Failed { reason, .. } => {
            log::warn!("{label} failed: {reason}; the run goes on");
            Ok(())
        }
        DomainResult::Assessed(_) => Ok(()),
    }
}

/// Asks (when the domains call for it) and stores one outcome's escalation,
/// with its overall judgement and ladder, at step `step` for outcome
/// `outcome` (its index). Its inputs are read back
/// from the store; a row bmlib refuses is [`StoreError::Corrupt`]. Failures
/// end the call as [`domain_step`]'s do.
fn escalation_step(
    ctx: &mut EvidenceContext,
    calls: &Calls,
    step: NextStep,
    outcome: usize,
    paper: &Paper,
) -> Result<(), EvidenceRunError> {
    let NextStep {
        id: step_id,
        run_id,
        label,
    } = step;
    let corrupt = |why: String| {
        EvidenceRunError::Store(StoreError::Corrupt(format!(
            "evidence run {run_id}, outcome {outcome}: {why}"
        )))
    };
    let (proposed, study, domains) = rob2_inputs(ctx.db, run_id, outcome, paper.canonical)?;
    let design = stored_design(ctx.db, run_id)?
        .ok_or_else(|| corrupt("its design is not stored".to_string()))?;
    let start = start_level(study_design(&design)?)
        .ok_or_else(|| corrupt(format!("design {:?} has no ladder", design.design)))?;
    let mut condenser = calls.condenser(ctx.condenser_model);
    let mut agent = configured!(
        Rob2Agent::new(&mut *ctx.chat),
        &mut condenser,
        calls,
        paper.budget
    );
    let escalation = retried(calls, escalation_retry_reason, || {
        agent.escalate_outcome(paper.title, &proposed, &study, &domains)
    })
    .map_err(|e| calls.rob2_failed(e))?;
    if let Rob2Escalation::Failed { reason, .. } = &escalation {
        if is_set(calls.stop) {
            log::warn!("{label} failed as it stopped: {reason}");
            return Err(EvidenceRunError::Cancelled);
        }
    }
    // On a store failure the model's escalation (or its failure's reason) would be lost otherwise.
    let unstored = |why: &dyn std::fmt::Display| {
        log::warn!("{label}: its escalation could not be stored ({why}): {escalation:?}");
    };
    let rob2 = combine_outcome(&proposed, &study, domains, escalation.clone()).map_err(|why| {
        unstored(&why);
        corrupt(why)
    })?;
    let assessed = assess_outcome(start, &study, &proposed, &rob2, &LadderSettings::default());
    if let Err(store) = save_escalation(ctx.db, step_id, &escalation, &assessed) {
        unstored(&store);
        return Err(store.into());
    }
    match escalation {
        Rob2Escalation::Failed {
            reason,
            retryable,
            status,
        } if from_a_request(retryable, status) => Err(EvidenceRunError::Request {
            label,
            message: reason,
            retryable,
            status,
        }),
        Rob2Escalation::Failed { reason, .. } => {
            log::warn!("{label} failed: {reason}; the run goes on");
            Ok(())
        }
        Rob2Escalation::NotNeeded | Rob2Escalation::Asked(_) => Ok(()),
    }
}

/// [`EvidenceRunError::Cancelled`] once `stop` is set.
fn cancelled(stop: &AtomicBool) -> Result<(), EvidenceRunError> {
    if is_set(stop) {
        Err(EvidenceRunError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MIN_REPLY_TOKENS;
    use crate::evidence_rows::{
        stored_domain, StoredDomain, StoredOutcomeResult, ESCALATION_FAILED,
    };
    use crate::evidence_store::{RunKind, StoredStep};
    use crate::identifiers::Identifiers;
    use crate::intake::SourceKind;
    use crate::store::{
        confirm_full_text, create_paper, open_memory_store, save_canonical, save_source, NewSource,
        SourcePlace,
    };
    use bmlib::agents::{ChatJsonError, ChatJsonOutcome};
    use bmlib::db::execute;
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_markdown};
    use bmlib::llm::LLMMessage;
    use bmlib::quality::rob2_d5_checks::D5_FACTS;
    use bmlib::quality::rob2_d5_facts::{D5_FACTS_SYSTEM_PROMPT, D5_SECOND_LOOK_SYSTEM_PROMPT};
    use bmlib::quality::rob2_questions::domain_title;
    use bmlib::quality::{
        assess_unattended, paper_weight, AgentOptions, EvidenceWeight, DESIGN_TASK_PROMPT,
        OUTCOME_TASK_PROMPT, ROB2_DOMAIN_PROMPT,
    };
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;

    // bmlib's evidence-weight fixture paper (tests/fixtures/evidence_weight.json).

    /// The trial's title.
    const TITLE: &str = "A trial of metformin";
    /// The design's quote.
    const RANDOMISED: &str = "Participants were randomly assigned 1:1 to metformin or placebo.";
    /// Every outcome's quote, and D5's measure and time point.
    const PRIMARY: &str = "The primary outcome was all-cause mortality at 28 days.";
    /// D1's quote.
    const SEQUENCE: &str = "The allocation sequence was generated by computer.";
    /// The passage naming the other measurement of the second-look tests.
    const CONCEALED: &str = "Allocation was concealed centrally.";
    /// D5's analysis, and what a second look finds.
    const ANALYSED: &str = "Mortality was analysed with the prespecified log-binomial regression.";
    /// The first outcome: low throughout, laddered high.
    const MORTALITY: &str = "All-cause mortality";
    /// The second outcome: D5 has some concerns, its interval crosses no effect.
    const STAY: &str = "Length of stay";
    /// A window the whole paper fits in.
    const WIDE: Option<u64> = Some(1_000_000);
    /// The reply room of every test run.
    const REPLY: u64 = MIN_REPLY_TOKENS;
    /// What the fake condenser model returns for any call.
    const DIGEST: &str = "Passages on the trial.";

    /// The trial as markdown, with `extra` after its methods.
    fn trial_md(extra: &str) -> String {
        format!("# {TITLE}\n\n## Methods\n\n{RANDOMISED} {PRIMARY} {SEQUENCE} {CONCEALED} {ANALYSED}\n{extra}")
    }

    /// Stores `md` as a new confirmed text of `p`; returns its id.
    fn add_text(c: &mut Connection, p: &PaperId, md: &str) -> i64 {
        let canonical = canonical_from_markdown(md);
        let source = NewSource {
            kind: SourceKind::Markdown,
            origin: "paste".into(),
            file_name: None,
            bytes: md.as_bytes().to_vec(),
            decoded: None,
        };
        let sid = save_source(c, p, source, SourcePlace::App).unwrap();
        let a = assess_full_text(&canonical).unwrap();
        let cid = save_canonical(c, p, sid, &canonical, &a).unwrap();
        confirm_full_text(c, p, cid).unwrap();
        cid
    }

    /// A store holding one paper with `md` as its text.
    fn store_with(md: &str) -> (Connection, PaperId) {
        let mut c = open_memory_store().unwrap();
        let ids = Identifiers {
            pmid: Some("1".into()),
            ..Default::default()
        };
        let p = create_paper(&mut c, &ids, Some("Trial")).unwrap();
        add_text(&mut c, &p, md);
        (c, p)
    }

    /// What a request asked, read from its prompts.
    #[derive(Debug, Clone, PartialEq)]
    enum Asked {
        Design,
        Outcomes,
        /// A domain's questions, and the outcome named (none for D1).
        Domain(Domain, Option<String>),
        /// D5's facts for the outcome named.
        Facts(String),
        /// A second look for the outcome named.
        Look(String),
        /// The escalation of the outcome named.
        Escalation(String),
    }

    /// What `messages` ask.
    fn asked_by(messages: &[LLMMessage]) -> Asked {
        let system = messages[0].content.as_str();
        let user = messages.last().unwrap().content.as_str();
        let outcome = user
            .lines()
            .find_map(|l| l.strip_prefix("Outcome assessed: "))
            .map(str::to_string);
        if user.starts_with(DESIGN_TASK_PROMPT) {
            Asked::Design
        } else if user.starts_with(OUTCOME_TASK_PROMPT) {
            Asked::Outcomes
        } else if system == D5_FACTS_SYSTEM_PROMPT {
            Asked::Facts(outcome.unwrap())
        } else if system == D5_SECOND_LOOK_SYSTEM_PROMPT {
            Asked::Look(outcome.unwrap())
        } else if user.starts_with(ROB2_DOMAIN_PROMPT) {
            let d = Domain::ALL
                .into_iter()
                .find(|d| user.contains(domain_title(*d)));
            Asked::Domain(d.unwrap(), outcome)
        } else {
            Asked::Escalation(outcome.unwrap())
        }
    }

    /// One scripted reply.
    type Reply = Result<Value, ChatJsonError>;

    /// A chat answering through a closure, recording what each request
    /// asked, and setting a stop during a given call.
    struct FakeChat<'s> {
        respond: Box<dyn FnMut(&Asked) -> Reply + 's>,
        asked: Vec<Asked>,
        stop_on: Option<(usize, &'s AtomicBool)>,
    }

    impl<'s> FakeChat<'s> {
        /// A chat answering through `respond`.
        fn new(respond: impl FnMut(&Asked) -> Reply + 's) -> Self {
            FakeChat {
                respond: Box::new(respond),
                asked: Vec::new(),
                stop_on: None,
            }
        }

        /// A chat answering every step of the trial ([`reply_to`]).
        fn trial() -> Self {
            Self::new(reply_to)
        }

        /// Sets `stop` during call `at` (0-based).
        fn stopping(mut self, at: usize, stop: &'s AtomicBool) -> Self {
            self.stop_on = Some((at, stop));
            self
        }

        /// How many requests asked `a`.
        fn count(&self, a: &Asked) -> usize {
            self.asked.iter().filter(|x| *x == a).count()
        }
    }

    impl JsonChat for FakeChat<'_> {
        fn chat_json(
            &mut self,
            messages: &[LLMMessage],
            _temperature: f64,
            _max_tokens: i64,
            _max_retries: usize,
            _require_dict: bool,
        ) -> Result<ChatJsonOutcome, ChatJsonError> {
            let asked = asked_by(messages);
            if let Some((at, stop)) = self.stop_on {
                if at == self.asked.len() {
                    stop.store(true, Ordering::SeqCst);
                }
            }
            self.asked.push(asked.clone());
            (self.respond)(&asked).map(|value| ChatJsonOutcome {
                value,
                attempts: 1,
                repaired: false,
            })
        }
    }

    /// A design reply: a randomised trial with `layout`.
    fn design_reply(layout: &str) -> Reply {
        Ok(
            json!({"study_design": "rct", "rct_layout": layout, "quotes": [RANDOMISED],
                  "reasoning": "As the methods say."}),
        )
    }

    /// The outcomes reply naming `names` ([`MORTALITY`], [`STAY`]).
    fn outcomes_reply(names: &[&str]) -> Reply {
        let outcomes: Vec<Value> = names
            .iter()
            .map(|&name| {
                let result = if name == MORTALITY {
                    json!({"measure": "risk ratio", "scale": "ratio", "estimate": 0.7,
                           "ci_low": 0.55, "ci_high": 0.88, "ci_level": 95, "n_analysed": 1200})
                } else {
                    json!({"measure": "mean difference", "scale": "difference", "estimate": -1.5,
                           "ci_low": -3.0, "ci_high": 0.2, "ci_level": 95, "n_analysed": 180})
                };
                json!({"name": name, "timepoint": "28 days", "quotes": [PRIMARY], "result": result})
            })
            .collect();
        Ok(json!({ "primary_outcomes": outcomes }))
    }

    /// The answers that make `domain` (D1–D4) low, with `changes`; D1's first
    /// quotes [`SEQUENCE`] (bmlib's `reply`).
    fn domain_reply(domain: Domain, changes: &[(&str, &str)]) -> Reply {
        let low: &[(&str, &str)] = match domain {
            Domain::D1 => &[("1.1", "Y"), ("1.2", "Y"), ("1.3", "N")],
            Domain::D2 => &[("2.1", "N"), ("2.2", "N"), ("2.6", "Y")],
            Domain::D3 => &[("3.1", "Y")],
            _ => &[("4.1", "N"), ("4.2", "N"), ("4.3", "N")],
        };
        let mut answers = serde_json::Map::new();
        for (q, a) in low.iter().chain(changes) {
            let quotes: &[&str] = if *q == "1.1" { &[SEQUENCE] } else { &[] };
            answers.insert(
                q.to_string(),
                json!({"answer": a, "quotes": quotes, "reasoning": "Why."}),
            );
        }
        Ok(json!({ "answers": answers }))
    }

    /// D3's reply that stays unreadable when asked again.
    fn unreadable() -> Reply {
        Ok(json!({"answers": {"3.1": {"answer": "Unclear", "quotes": [], "reasoning": "Why."}}}))
    }

    /// D5's facts as the fixture gives them (measure, time point and analysis
    /// fixed), with `measurements` as its other measurements.
    fn facts_reply(measurements: Value) -> Reply {
        let quotes = |f: &str| match f {
            "measure_fixed" | "timepoint_fixed" => vec![PRIMARY],
            "analysis_fixed" => vec![ANALYSED],
            _ => vec![],
        };
        let facts: serde_json::Map<String, Value> = D5_FACTS
            .iter()
            .map(|f| {
                let q = quotes(f.as_str());
                (
                    f.as_str().to_string(),
                    json!({"present": !q.is_empty(), "quotes": q}),
                )
            })
            .collect();
        Ok(json!({"facts": facts, "other_measurements": measurements,
                  "main_analysis": {"differs": false, "planned": [], "reported": []}}))
    }

    /// One other measurement of mortality naming no result, so it gets a
    /// second look.
    fn unreported() -> Value {
        json!([{"label": "Mortality at 90 days", "named": [CONCEALED], "result": []}])
    }

    /// A second look that finds the result.
    fn found() -> Reply {
        Ok(json!({"found": true, "quotes": [ANALYSED]}))
    }

    /// A server failure a retry may cure.
    fn server_error() -> Reply {
        Err(ChatJsonError::Transport {
            message: "HTTP 500".into(),
            attempts: 1,
            retryable: true,
            status: Some(500),
        })
    }

    /// The trial's reply to `a`: a parallel-group trial with two outcomes,
    /// every domain low but STAY's D5, no other measurement.
    fn reply_to(a: &Asked) -> Reply {
        match a {
            Asked::Design => design_reply("parallel"),
            Asked::Outcomes => outcomes_reply(&[MORTALITY, STAY]),
            Asked::Domain(d, _) => domain_reply(*d, &[]),
            Asked::Facts(_) => facts_reply(json!([])),
            Asked::Look(_) => found(),
            Asked::Escalation(_) => Ok(json!({"escalate": false, "reasoning": "Why."})),
        }
    }

    /// [`reply_to`] with some concerns in D1, so STAY (D1 and D5) needs the
    /// escalation, which says to escalate.
    fn concerns(a: &Asked) -> Reply {
        match a {
            Asked::Domain(Domain::D1, _) => domain_reply(Domain::D1, &[("1.2", "NI")]),
            Asked::Escalation(_) => Ok(json!({"escalate": true, "reasoning": "Two concerns."})),
            other => reply_to(other),
        }
    }

    /// A condenser model returning [`DIGEST`].
    struct FakeModel;

    impl ContextModel for FakeModel {
        fn complete(&self, _prompt: &str, _t: f64, _m: i64) -> Result<String, String> {
            Ok(DIGEST.to_string())
        }

        fn complete_json(&self, _prompt: &str, _t: f64, _m: i64) -> Result<Value, String> {
            Err("not used by the condenser".to_string())
        }
    }

    /// A context over `c` and `chat` with `window`, zero retry delay and no
    /// listener.
    fn context<'a, 's: 'a>(
        c: &'a mut Connection,
        chat: &'a mut FakeChat<'s>,
        window: Option<u64>,
    ) -> EvidenceContext<'a> {
        EvidenceContext {
            db: c,
            chat,
            condenser_model: &FakeModel,
            model: "ollama:m".into(),
            window,
            reply_tokens: REPLY,
            retry_delay: Duration::ZERO,
            on_event: Arc::new(|_| {}),
        }
    }

    /// A new run with `chat` (`stop` as given) and its result.
    fn start(
        c: &mut Connection,
        p: &PaperId,
        chat: &mut FakeChat,
        stop: &AtomicBool,
    ) -> Result<i64, EvidenceRunError> {
        run_evidence(&mut context(c, chat, WIDE), p, stop)
    }

    /// Continues `run_id` with `chat`.
    fn resume(
        c: &mut Connection,
        run_id: i64,
        chat: &mut FakeChat,
        stop: &AtomicBool,
    ) -> Result<i64, EvidenceRunError> {
        continue_evidence(&mut context(c, chat, WIDE), run_id, stop)
    }

    /// The newest run's id.
    fn newest(c: &mut Connection) -> i64 {
        bmlib::db::fetch_one(c, "SELECT max(id) AS id FROM evidence_runs", &[])
            .unwrap()
            .unwrap()
            .get_i64("id")
            .unwrap()
    }

    /// The stored run `id`.
    fn stored(c: &mut Connection, id: i64) -> StoredEvidenceRun {
        stored_run(c, id).unwrap().unwrap()
    }

    /// Domain `domain`'s step for `outcome` (`None`: D1).
    fn dom(domain: Domain, outcome: Option<usize>) -> EvidenceStep {
        EvidenceStep::Rob2(Rob2Step::Domain { domain, outcome })
    }

    /// Outcome `outcome`'s escalation step.
    fn esc(outcome: usize) -> EvidenceStep {
        EvidenceStep::Rob2(Rob2Step::Escalation { outcome })
    }

    /// Step `step` of run `id`.
    fn step_of(c: &mut Connection, id: i64, step: EvidenceStep) -> StoredStep {
        let run = stored(c, id);
        run.steps.into_iter().find(|s| s.step == step).unwrap()
    }

    /// Step `step`'s status in run `id`.
    fn status(c: &mut Connection, id: i64, step: EvidenceStep) -> StepStatus {
        step_of(c, id, step).status
    }

    /// Every step's status in plan order.
    fn statuses(c: &mut Connection, id: i64) -> Vec<(EvidenceStep, StepStatus)> {
        stored(c, id)
            .steps
            .iter()
            .map(|s| (s.step, s.status))
            .collect()
    }

    /// Outcome `outcome`'s stored overall judgement, ladder and its error.
    fn result_of(c: &mut Connection, id: i64, outcome: usize) -> StoredOutcomeResult {
        let step = step_of(c, id, esc(outcome)).id;
        stored_outcome_result(c, step).unwrap().unwrap()
    }

    /// What the trial run asks, in order: the design, the outcomes, D1, then
    /// each of `names`' D2–D4 and D5 facts.
    fn trial_asks(names: &[&str]) -> Vec<Asked> {
        let mut asks = vec![
            Asked::Design,
            Asked::Outcomes,
            Asked::Domain(Domain::D1, None),
        ];
        for name in names {
            for d in &Domain::ALL[1..4] {
                asks.push(Asked::Domain(*d, Some(name.to_string())));
            }
            asks.push(Asked::Facts(name.to_string()));
        }
        asks
    }

    /// `name`'s D3.
    fn d3_of(name: &str) -> Asked {
        Asked::Domain(Domain::D3, Some(name.to_string()))
    }

    /// The labels, in plan order, with the names the tab shows.
    #[test]
    fn step_labels_are_the_tabs() {
        assert_eq!(step_label(&EvidenceStep::Design, 0), "Study design");
        assert_eq!(step_label(&EvidenceStep::Outcomes, 0), "Primary outcomes");
        assert_eq!(
            step_label(&dom(Domain::D1, None), 2),
            "The trial: D1, the randomisation process"
        );
        assert_eq!(
            step_label(&dom(Domain::D4, Some(1)), 2),
            "Outcome 2 of 2: D4, measurement of the outcome"
        );
        assert_eq!(
            step_label(&esc(0), 3),
            "Outcome 1 of 3: the overall judgement"
        );
    }

    /// The budget falls as outcomes add longer prompts, and a small window has
    /// none.
    #[test]
    fn budget_counts_every_prompt() {
        let (mut c, p) = store_with(&trial_md(""));
        let id = start(&mut c, &p, &mut FakeChat::trial(), &AtomicBool::new(false)).unwrap();
        let text = stored(&mut c, id).text;
        let outcomes = run_outcomes(&mut c, id, &text).unwrap();
        let none = budget_chars(WIDE, REPLY, TITLE, &[]).unwrap().unwrap();
        let two = budget_chars(WIDE, REPLY, TITLE, &outcomes)
            .unwrap()
            .unwrap();
        assert!(two < none, "{two} {none}");
        assert_eq!(budget_chars(Some(REPLY), REPLY, TITLE, &[]).unwrap(), None);
    }

    /// The refusals read as PICO's, the rest name their step.
    #[test]
    fn errors_read_as_the_tab_shows_them() {
        assert_eq!(
            EvidenceRunError::Cancelled.to_string(),
            "The evidence weight assessment was stopped."
        );
        assert_eq!(EvidenceRunError::EarlierRules.to_string(), EARLIER_RULES);
        assert_eq!(
            EvidenceRunError::NoFullText.to_string(),
            PicoRunError::NoFullText.to_string()
        );
        let request = EvidenceRunError::Request {
            label: "Outcome 1 of 1: D3, missing outcome data".into(),
            message: "HTTP 500".into(),
            retryable: true,
            status: Some(500),
        };
        assert_eq!(
            request.to_string(),
            "Outcome 1 of 1: D3, missing outcome data: HTTP 500"
        );
        let proposal = EvidenceRunError::Proposal {
            step: DESIGN_LABEL,
            error: ProposalError::Unreadable("bad".into()),
        };
        assert_eq!(proposal.to_string(), "Study design: bad");
    }

    /// A crossover trial is not covered: one request, and the run is
    /// complete.
    #[test]
    fn not_covered_ends_after_the_design() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|_| design_reply("crossover"));
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(chat.asked, [Asked::Design]);
        let run = stored(&mut c, id);
        assert_eq!(run.status, RunStatus::Complete);
        assert_eq!(run.kind, Some(RunKind::NotCovered));
    }

    /// A trial naming no primary outcome ends after the outcomes, complete.
    #[test]
    fn no_primary_outcome_ends_after_the_outcomes() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|a| match a {
            Asked::Outcomes => outcomes_reply(&[]),
            other => reply_to(other),
        });
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(chat.asked, [Asked::Design, Asked::Outcomes]);
        let run = stored(&mut c, id);
        assert_eq!(run.status, RunStatus::Complete);
        assert_eq!(run.kind, Some(RunKind::NoPrimaryOutcome));
    }

    /// A D5 is asked again for the first retryable second look that failed,
    /// even after a retryable one that holds no error.
    #[test]
    fn domain_retry_reason_finds_a_later_failed_look() {
        use bmlib::quality::rob2_d5_facts::{AnalysisCheck, D5Facts, OtherMeasurement};
        use bmlib::quality::DomainAssessment;
        let look = |error: Option<&str>| OtherMeasurement {
            label: "at 6 weeks".into(),
            named: vec![],
            result: vec![],
            second_look: None,
            second_look_error: error.map(str::to_string),
            second_look_retryable: true,
            second_look_status: None,
            quotes_not_found: vec![],
            contradicted: vec![],
            quotes_dropped: 0,
        };
        let d5 = |measurements| {
            DomainResult::Assessed(DomainAssessment {
                domain: Domain::D5,
                answers: Default::default(),
                judgement: Judgement::Low,
                condensed: false,
                ignored: vec![],
                facts: Some(Box::new(D5Facts {
                    outcome: "pain".into(),
                    facts: Default::default(),
                    measurements,
                    analysis: AnalysisCheck::default(),
                })),
            })
        };
        assert_eq!(
            domain_retry_reason(&d5(vec![look(None), look(Some("busy"))])).as_deref(),
            Some("a second look failed: busy")
        );
        assert_eq!(domain_retry_reason(&d5(vec![look(None)])), None);
    }

    /// A window with no room for the paper is refused before any call or row.
    #[test]
    fn too_long_window_is_refused() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::trial();
        let stop = AtomicBool::new(false);
        let e = run_evidence(&mut context(&mut c, &mut chat, Some(REPLY)), &p, &stop);
        assert!(matches!(
            e,
            Err(EvidenceRunError::TooLong {
                window: Some(REPLY)
            })
        ));
        assert!(chat.asked.is_empty());
        let runs = bmlib::db::fetch_one(&mut c, "SELECT count(*) AS n FROM evidence_runs", &[]);
        assert_eq!(runs.unwrap().unwrap().get_i64("n").unwrap(), 0);
    }

    /// A refused design (a 401) is asked once, fails its step and ends the
    /// call as `Proposal`.
    #[test]
    fn a_refused_design_fails_its_step() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|_| {
            Err(ChatJsonError::Transport {
                message: "HTTP 401".into(),
                attempts: 1,
                retryable: false,
                status: Some(401),
            })
        });
        let e = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap_err();
        assert!(
            matches!(
                e,
                EvidenceRunError::Proposal {
                    step: "Study design",
                    ..
                }
            ),
            "{e}"
        );
        assert_eq!(chat.asked, [Asked::Design]);
        let id = newest(&mut c);
        let design = step_of(&mut c, id, EvidenceStep::Design);
        assert_eq!(design.status, StepStatus::Failed);
        assert_eq!(design.failure.unwrap().status, Some(401));
    }

    /// A RoB 2 error: domains bmlib refuses stay `Corrupt` even once stopped
    /// (damage is never reported as a cancel); any other error is a cancel
    /// once stopped, else internal.
    #[test]
    fn a_rob2_error_once_stopped() {
        for stopped in [false, true] {
            let stop = AtomicBool::new(stopped);
            let calls = Calls {
                on_event: Arc::new(|_| {}),
                delay: Duration::ZERO,
                max_tokens: 1,
                stop: &stop,
            };
            let corrupt = calls.rob2_failed(Rob2RunError::InvalidDomains("bad".into()));
            assert!(
                matches!(corrupt, EvidenceRunError::Store(StoreError::Corrupt(_))),
                "{corrupt}"
            );
            let other = calls.rob2_failed(Rob2RunError::WrongOutcome);
            if stopped {
                assert!(matches!(other, EvidenceRunError::Cancelled), "{other}");
            } else {
                assert!(matches!(other, EvidenceRunError::Internal(_)), "{other}");
            }
            let cancel = calls.rob2_failed(Rob2RunError::Stopped);
            assert!(matches!(cancel, EvidenceRunError::Cancelled), "{cancel}");
        }
    }

    /// Continue after a failed design, or failed outcomes, asks again from
    /// that step on (never a step already done) and finishes the run, the
    /// step's failure cleared once it is saved.
    #[test]
    fn continue_reruns_a_failed_design_or_outcomes() {
        for failing in [Asked::Design, Asked::Outcomes] {
            let (mut c, p) = store_with(&trial_md(""));
            let mut chat = FakeChat::new(|a| {
                if *a == failing {
                    return Err(ChatJsonError::Transport {
                        message: "HTTP 401".into(),
                        attempts: 1,
                        retryable: false,
                        status: Some(401),
                    });
                }
                reply_to(a)
            });
            let e = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap_err();
            assert!(matches!(e, EvidenceRunError::Proposal { .. }), "{e}");
            let id = newest(&mut c);
            let step = if failing == Asked::Design {
                EvidenceStep::Design
            } else {
                EvidenceStep::Outcomes
            };
            assert_eq!(status(&mut c, id, step), StepStatus::Failed);

            let mut chat = FakeChat::trial();
            assert_eq!(
                resume(&mut c, id, &mut chat, &AtomicBool::new(false)).unwrap(),
                id
            );
            let mut asks = trial_asks(&[MORTALITY, STAY]);
            if failing == Asked::Outcomes {
                asks.remove(0);
            }
            assert_eq!(chat.asked, asks, "{failing:?}");
            let again = step_of(&mut c, id, step);
            assert_eq!((again.status, again.failure), (StepStatus::Done, None));
            assert_eq!(stored(&mut c, id).status, RunStatus::Complete);
        }
    }

    /// A trial runs every step in plan order (no escalation is needed, so
    /// none is asked) and ends complete; Continue then has nothing to do.
    #[test]
    fn a_trial_runs_every_step_in_plan_order() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::trial();
        let stop = AtomicBool::new(false);
        let id = start(&mut c, &p, &mut chat, &stop).unwrap();
        assert_eq!(chat.asked, trial_asks(&[MORTALITY, STAY]));
        let run = stored(&mut c, id);
        assert_eq!(run.status, RunStatus::Complete);
        assert_eq!(run.kind, Some(RunKind::Assessed));
        assert_eq!(run.steps.len(), 2 + 1 + 2 * 5);
        assert!(run.steps.iter().all(|s| s.status == StepStatus::Done));
        let e = resume(&mut c, id, &mut FakeChat::trial(), &stop).unwrap_err();
        assert!(matches!(e, EvidenceRunError::Internal(_)), "{e}");
    }

    /// The same replies through bmlib's `assess_unattended` give the same
    /// overall judgements, ladders and paper weight as the run stores.
    #[test]
    fn the_steps_match_assess_unattended() {
        let md = trial_md("");
        let (mut c, p) = store_with(&md);
        let mut chat = FakeChat::new(concerns);
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(chat.count(&Asked::Escalation(STAY.into())), 1);
        let text = stored(&mut c, id).text;
        let budget = budget_chars(WIDE, REPLY, TITLE, &[]).unwrap().unwrap();
        let options = AgentOptions {
            temperature: EVIDENCE_TEMPERATURE,
            max_tokens: REPLY as i64,
            condense_config: ProcessingConfig::default().with_max_context_chars(budget),
            ..AgentOptions::default()
        };
        let mut theirs = FakeChat::new(concerns);
        let settings = LadderSettings::default();
        let weight = assess_unattended(
            &mut theirs,
            None,
            None,
            &settings,
            &options,
            title_of(&text),
            &text.text,
            MIN_QUOTE_SIMILARITY,
        )
        .unwrap();
        assert_eq!(theirs.asked, chat.asked);
        let EvidenceWeight::Assessed {
            assessed,
            paper_weight: expected,
            ..
        } = weight
        else {
            panic!("not assessed");
        };
        let name = |v: Value| v.as_str().unwrap().to_string();
        let mut weights = Vec::new();
        for (i, theirs) in assessed.iter().enumerate() {
            let ours = result_of(&mut c, id, i);
            let overall = theirs.rob2.overall.map(|j| name(json!(j)));
            assert_eq!(ours.overall, overall, "outcome {i}");
            let ladder = theirs
                .ladder
                .as_ref()
                .map(|l| (name(json!(l.level)), l.weight));
            let stored = ours.ladder.as_ref().map(|l| (l.level.clone(), l.weight));
            assert_eq!(stored, ladder, "outcome {i}");
            weights.extend(stored.map(|(_, w)| w));
        }
        assert_eq!(result_of(&mut c, id, 1).overall.as_deref(), Some("high"));
        let ours = weights.iter().copied().reduce(f64::max);
        let ladders: Vec<_> = assessed.iter().filter_map(|a| a.ladder.clone()).collect();
        assert_eq!(ours, expected);
        assert_eq!(ours, paper_weight(&ladders));
    }

    /// Each step is announced in plan order, its label before its
    /// condensation's events.
    #[test]
    fn events_name_each_step() {
        let filler = "Filler sentence about nothing at all. ".repeat(4000);
        let (mut c, p) = store_with(&trial_md(&filler));
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&events);
        let window = (0..)
            .map(|k| k * 64)
            .find(|&w| budget_chars(Some(w), REPLY, TITLE, &[]).unwrap().is_some())
            .map(|w| w + MIN_REPLY_TOKENS);
        let budget = budget_chars(window, REPLY, TITLE, &[]).unwrap().unwrap();
        assert!(filler.len() > budget, "the paper must be condensed");
        let mut chat = FakeChat::trial();
        let mut ctx = context(&mut c, &mut chat, window);
        ctx.on_event = Arc::new(move |e| seen.lock().unwrap().push(e));
        run_evidence(&mut ctx, &p, &AtomicBool::new(false)).unwrap();
        let events = events.lock().unwrap().clone();
        let labels: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                EvidenceEvent::Step { label } => Some(label.clone()),
                _ => None,
            })
            .collect();
        let id = newest(&mut c);
        let plan: Vec<String> = stored(&mut c, id)
            .steps
            .iter()
            .map(|s| step_label(&s.step, 2))
            .collect();
        assert_eq!(labels, plan);
        let condensing = events
            .iter()
            .position(|e| matches!(e, EvidenceEvent::Condensing { .. }))
            .unwrap();
        let second = events
            .iter()
            .position(|e| {
                *e == EvidenceEvent::Step {
                    label: OUTCOMES_LABEL.into(),
                }
            })
            .unwrap();
        assert_eq!(
            events[0],
            EvidenceEvent::Step {
                label: DESIGN_LABEL.into()
            }
        );
        assert!(condensing < second);
        assert!(events[1..second].contains(&EvidenceEvent::Condensed));
    }

    /// A D3 that stays unreadable fails and the run goes on: D4 and D5 run,
    /// the outcome has no ladder, and the run is incomplete.
    #[test]
    fn an_unreadable_domain_fails_and_the_run_goes_on() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|a| match a {
            Asked::Domain(Domain::D3, Some(o)) if o == MORTALITY => unreadable(),
            other => reply_to(other),
        });
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            status(&mut c, id, dom(Domain::D3, Some(0))),
            StepStatus::Failed
        );
        assert_eq!(
            status(&mut c, id, dom(Domain::D4, Some(0))),
            StepStatus::Done
        );
        assert_eq!(
            status(&mut c, id, dom(Domain::D5, Some(0))),
            StepStatus::Done
        );
        let result = result_of(&mut c, id, 0);
        assert!(result.ladder.is_none());
        assert!(result.ladder_error.unwrap().contains("D3 failed"));
        assert_eq!(stored(&mut c, id).status, RunStatus::Incomplete);
    }

    /// A D3 answered 500 on every attempt fails and ends the call: D4 is
    /// still pending.
    #[test]
    fn a_request_failure_ends_the_call() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|a| match a {
            Asked::Domain(Domain::D3, _) => server_error(),
            other => reply_to(other),
        });
        let e = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap_err();
        let EvidenceRunError::Request {
            label,
            retryable,
            status: code,
            ..
        } = e
        else {
            panic!("not a request failure: {e}");
        };
        assert_eq!(label, "Outcome 1 of 2: D3, missing outcome data");
        assert_eq!((retryable, code), (true, Some(500)));
        assert_eq!(chat.count(&d3_of(MORTALITY)), RETRIEVAL_ATTEMPTS as usize);
        let id = newest(&mut c);
        assert_eq!(
            status(&mut c, id, dom(Domain::D3, Some(0))),
            StepStatus::Failed
        );
        assert_eq!(
            status(&mut c, id, dom(Domain::D4, Some(0))),
            StepStatus::Pending
        );
    }

    /// [`concerns`] with mortality's D2 some concerns too, so both outcomes
    /// need the escalation; `escalation` answers it for mortality.
    fn both_escalate(escalation: fn() -> Reply) -> impl FnMut(&Asked) -> Reply {
        move |a| match a {
            Asked::Domain(Domain::D2, Some(o)) if o == MORTALITY => {
                domain_reply(Domain::D2, &[("2.6", "N"), ("2.7", "N")])
            }
            Asked::Escalation(o) if o == MORTALITY => escalation(),
            other => concerns(other),
        }
    }

    /// An escalation answered 500 on every attempt fails its step, keeps its
    /// outcome's row (no overall judgement, the ladder error), and ends the
    /// call as `Request`: STAY's steps are still pending.
    #[test]
    fn a_failed_escalation_request_ends_the_call() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(both_escalate(server_error));
        let e = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap_err();
        let EvidenceRunError::Request {
            label,
            retryable,
            status: code,
            ..
        } = e
        else {
            panic!("not a request failure: {e}");
        };
        assert_eq!(label, "Outcome 1 of 2: the overall judgement");
        assert_eq!((retryable, code), (true, Some(500)));
        let mortality = Asked::Escalation(MORTALITY.into());
        assert_eq!(chat.count(&mortality), RETRIEVAL_ATTEMPTS as usize);
        let id = newest(&mut c);
        assert_eq!(status(&mut c, id, esc(0)), StepStatus::Failed);
        let result = result_of(&mut c, id, 0);
        assert_eq!(result.escalation, ESCALATION_FAILED);
        assert!(result.overall.is_none() && result.ladder_error.is_some());
        assert_eq!(
            status(&mut c, id, dom(Domain::D2, Some(1))),
            StepStatus::Pending
        );
    }

    /// An escalation whose reply cannot be read (not a request fault) is
    /// stored as failed and the run goes on: STAY is assessed, and the run
    /// ends incomplete.
    #[test]
    fn an_unreadable_escalation_fails_and_the_run_goes_on() {
        let (mut c, p) = store_with(&trial_md(""));
        let unreadable = || Ok(json!({"escalate": "maybe", "reasoning": "Why."}));
        let mut chat = FakeChat::new(both_escalate(unreadable));
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(status(&mut c, id, esc(0)), StepStatus::Failed);
        let result = result_of(&mut c, id, 0);
        assert_eq!(result.escalation, ESCALATION_FAILED);
        assert!(result.overall.is_none() && result.ladder_error.is_some());
        assert_eq!(chat.count(&Asked::Escalation(STAY.into())), 1);
        assert_eq!(status(&mut c, id, esc(1)), StepStatus::Done);
        assert!(result_of(&mut c, id, 1).ladder.is_some());
        assert_eq!(stored(&mut c, id).status, RunStatus::Incomplete);
    }

    /// A failure a retry may cure is asked again, and the run completes.
    #[test]
    fn a_retryable_failure_is_retried() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut failed = false;
        let mut chat = FakeChat::new(|a| match a {
            Asked::Domain(Domain::D3, _) if !failed => {
                failed = true;
                server_error()
            }
            other => reply_to(other),
        });
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(chat.count(&d3_of(MORTALITY)), 2);
        assert_eq!(stored(&mut c, id).status, RunStatus::Complete);
    }

    /// A failure as Stop is pressed is not retried: the call is cancelled and
    /// the step left pending.
    #[test]
    fn stopped_is_not_retried() {
        let (mut c, p) = store_with(&trial_md(""));
        let stop = AtomicBool::new(false);
        let mut chat = FakeChat::new(|a| match a {
            Asked::Domain(Domain::D3, _) => server_error(),
            other => reply_to(other),
        })
        .stopping(4, &stop);
        let e = start(&mut c, &p, &mut chat, &stop).unwrap_err();
        assert!(matches!(e, EvidenceRunError::Cancelled), "{e}");
        assert_eq!(chat.count(&d3_of(MORTALITY)), 1);
        let id = newest(&mut c);
        assert_eq!(
            status(&mut c, id, dom(Domain::D3, Some(0))),
            StepStatus::Pending
        );
    }

    /// A run whose mortality D5 holds a second look that failed on every
    /// attempt (each attempt asking the facts again).
    fn run_with_a_failed_look(c: &mut Connection, p: &PaperId) -> (i64, FakeChat<'static>) {
        let mut chat = FakeChat::new(|a| match a {
            Asked::Facts(o) if o == MORTALITY => facts_reply(unreported()),
            Asked::Look(_) => server_error(),
            other => reply_to(other),
        });
        let id = start(c, p, &mut chat, &AtomicBool::new(false)).unwrap();
        (id, chat)
    }

    /// A second look that fails on every attempt is kept on a done D5, named,
    /// and leaves the run incomplete after its last step.
    #[test]
    fn a_failed_second_look_is_kept_and_named() {
        let (mut c, p) = store_with(&trial_md(""));
        let (id, chat) = run_with_a_failed_look(&mut c, &p);
        let attempts = RETRIEVAL_ATTEMPTS as usize;
        assert_eq!(chat.count(&Asked::Facts(MORTALITY.into())), attempts);
        assert_eq!(chat.count(&Asked::Look(MORTALITY.into())), attempts);
        let d5 = step_of(&mut c, id, dom(Domain::D5, Some(0)));
        assert_eq!(d5.status, StepStatus::Done);
        let row = stored_domain(&mut c, d5.id).unwrap().unwrap();
        let look = &row.d5.unwrap().measurements[0];
        assert_eq!(look.label, "Mortality at 90 days");
        assert!(look.second_look_error.as_deref().unwrap().contains("500"));
        assert!(statuses(&mut c, id)
            .iter()
            .all(|(_, s)| *s == StepStatus::Done));
        assert_eq!(stored(&mut c, id).status, RunStatus::Incomplete);
    }

    /// Stops a run during STAY's D2 (call 7), so mortality's overall
    /// judgement is done and the rest pending; returns the run's id.
    fn stopped_after_mortality(c: &mut Connection, p: &PaperId) -> i64 {
        let stop = AtomicBool::new(false);
        let mut chat = FakeChat::trial().stopping(7, &stop);
        let e = start(c, p, &mut chat, &stop).unwrap_err();
        assert!(matches!(e, EvidenceRunError::Cancelled), "{e}");
        newest(c)
    }

    /// An overall judgement edited to say the escalation was asked, where the
    /// domains do not call for it, is corrupt on Continue, which changes
    /// nothing.
    #[test]
    fn a_stale_escalation_is_corrupt() {
        let (mut c, p) = store_with(&trial_md(""));
        let id = stopped_after_mortality(&mut c, &p);
        assert_eq!(result_of(&mut c, id, 0).escalation, ESCALATION_NOT_NEEDED);
        let step = step_of(&mut c, id, esc(0)).id;
        execute(
            &mut c,
            "UPDATE evidence_outcome_results SET escalation = 'asked', escalate = 1,
                    escalation_reasoning = 'Serious.' WHERE step_id = ?",
            &[bmlib::db::Value::from(step)],
        )
        .unwrap();
        let before = statuses(&mut c, id);
        let mut chat = FakeChat::trial();
        let e = resume(&mut c, id, &mut chat, &AtomicBool::new(false)).unwrap_err();
        assert!(
            matches!(&e, EvidenceRunError::Store(StoreError::Corrupt(m)) if m.contains("outcome 0")),
            "{e}"
        );
        assert!(chat.asked.is_empty());
        assert_eq!(statuses(&mut c, id), before);
    }

    /// An escalation stored as asked with no answer is corrupt on Continue:
    /// `stored_outcome_result`, inside `check_escalations`, refuses it before
    /// `check_escalations`' own arm would.
    #[test]
    fn an_asked_escalation_without_its_answer_is_corrupt() {
        let (mut c, p) = store_with(&trial_md(""));
        let id = stopped_after_mortality(&mut c, &p);
        let step = step_of(&mut c, id, esc(0)).id;
        execute(
            &mut c,
            "UPDATE evidence_outcome_results SET escalation = 'asked', escalate = NULL
             WHERE step_id = ?",
            &[bmlib::db::Value::from(step)],
        )
        .unwrap();
        let mut chat = FakeChat::trial();
        let e = resume(&mut c, id, &mut chat, &AtomicBool::new(false)).unwrap_err();
        assert!(
            matches!(&e, EvidenceRunError::Store(StoreError::Corrupt(m)) if m.contains("escalation \"asked\" does not fit")),
            "{e}"
        );
        assert!(chat.asked.is_empty());
    }

    /// A stop after outcome 1's D2 keeps D1 and D2, leaves the rest pending,
    /// and the run incomplete.
    #[test]
    fn stop_keeps_finished_steps() {
        let (mut c, p) = store_with(&trial_md(""));
        let stop = AtomicBool::new(false);
        let mut chat = FakeChat::trial().stopping(3, &stop);
        let e = start(&mut c, &p, &mut chat, &stop).unwrap_err();
        assert!(matches!(e, EvidenceRunError::Cancelled), "{e}");
        let id = newest(&mut c);
        let done: Vec<EvidenceStep> = statuses(&mut c, id)
            .into_iter()
            .filter(|(_, s)| *s == StepStatus::Done)
            .map(|(step, _)| step)
            .collect();
        let expected = [
            EvidenceStep::Design,
            EvidenceStep::Outcomes,
            dom(Domain::D1, None),
            dom(Domain::D2, Some(0)),
        ];
        assert_eq!(done, expected);
        let run = stored(&mut c, id);
        assert_eq!(run.status, RunStatus::Incomplete);
        assert!(run.steps[4..]
            .iter()
            .all(|s| s.status == StepStatus::Pending));
    }

    /// The reply that arrives as Stop is pressed is stored.
    #[test]
    fn a_reply_during_stop_is_kept() {
        let (mut c, p) = store_with(&trial_md(""));
        let stop = AtomicBool::new(false);
        let mut chat = FakeChat::trial().stopping(0, &stop);
        let e = start(&mut c, &p, &mut chat, &stop).unwrap_err();
        assert!(matches!(e, EvidenceRunError::Cancelled), "{e}");
        let id = newest(&mut c);
        assert_eq!(status(&mut c, id, EvidenceStep::Design), StepStatus::Done);
        assert_eq!(
            status(&mut c, id, EvidenceStep::Outcomes),
            StepStatus::Pending
        );
    }

    /// Continue runs only the steps a stop left.
    #[test]
    fn continue_runs_only_what_is_left() {
        let (mut c, p) = store_with(&trial_md(""));
        let id = stopped_after_mortality(&mut c, &p);
        let mut chat = FakeChat::trial();
        assert_eq!(
            resume(&mut c, id, &mut chat, &AtomicBool::new(false)).unwrap(),
            id
        );
        // STAY's D2 came back as Stop was pressed, and was kept.
        assert_eq!(chat.asked, trial_asks(&[MORTALITY, STAY])[8..]);
        assert_eq!(stored(&mut c, id).status, RunStatus::Complete);
    }

    /// Continue asks a failed D3 again, and its outcome's overall judgement
    /// is made again from it.
    #[test]
    fn continue_reruns_a_failed_domain_and_its_escalation() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|a| match a {
            Asked::Domain(Domain::D3, Some(o)) if o == MORTALITY => unreadable(),
            other => reply_to(other),
        });
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        let mut chat = FakeChat::trial();
        resume(&mut c, id, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(chat.asked, [d3_of(MORTALITY)]);
        let result = result_of(&mut c, id, 0);
        assert!(result.ladder.is_some() && result.ladder_error.is_none());
        assert_eq!(stored(&mut c, id).status, RunStatus::Complete);
    }

    /// Continue asks a failed D1 again, and every outcome's overall judgement
    /// is made again.
    #[test]
    fn continue_reruns_d1_and_every_escalation() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|a| match a {
            Asked::Domain(Domain::D1, _) => Ok(json!({"answers": {}})),
            other => reply_to(other),
        });
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        for i in 0..2 {
            assert!(result_of(&mut c, id, i)
                .ladder_error
                .unwrap()
                .contains("D1 failed"));
        }
        let mut chat = FakeChat::trial();
        resume(&mut c, id, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(chat.asked, [Asked::Domain(Domain::D1, None)]);
        for i in 0..2 {
            assert!(result_of(&mut c, id, i).ladder.is_some(), "outcome {i}");
        }
        assert_eq!(stored(&mut c, id).status, RunStatus::Complete);
    }

    /// Continue asks a D5 with a failed second look again, look included.
    #[test]
    fn continue_reruns_a_d5_with_a_failed_look() {
        let (mut c, p) = store_with(&trial_md(""));
        let (id, _) = run_with_a_failed_look(&mut c, &p);
        let mut chat = FakeChat::new(|a| match a {
            Asked::Facts(o) if o == MORTALITY => facts_reply(unreported()),
            other => reply_to(other),
        });
        resume(&mut c, id, &mut chat, &AtomicBool::new(false)).unwrap();
        let mortality = MORTALITY.to_string();
        assert_eq!(
            chat.asked,
            [Asked::Facts(mortality.clone()), Asked::Look(mortality)]
        );
        let d5 = step_of(&mut c, id, dom(Domain::D5, Some(0))).id;
        let row = stored_domain(&mut c, d5).unwrap().unwrap();
        assert!(row.d5.unwrap().measurements[0].second_look.is_some());
        assert_eq!(stored(&mut c, id).status, RunStatus::Complete);
    }

    /// Continue reads the run's own text, not the paper's newer one: its
    /// quotes are located there.
    #[test]
    fn continue_uses_run_text() {
        let (mut c, p) = store_with(&trial_md(""));
        let stop = AtomicBool::new(false);
        let mut chat = FakeChat::trial().stopping(0, &stop);
        start(&mut c, &p, &mut chat, &stop).unwrap_err();
        let id = newest(&mut c);
        let newer = format!(
            "# {TITLE}\n\n## Preface\n\nA newer preface.\n\n{}",
            trial_md("")
        );
        let newer_id = add_text(&mut c, &p, &newer);
        resume(&mut c, id, &mut FakeChat::trial(), &AtomicBool::new(false)).unwrap();
        let run = stored(&mut c, id);
        assert_ne!(run.canonical_id, newer_id);
        let at = run.text.text.find(PRIMARY).unwrap();
        let outcomes = stored_outcomes(&mut c, id).unwrap();
        assert_eq!(outcomes[0].quotes.citations[0].start, at);
        assert_eq!(run.status, RunStatus::Complete);
    }

    /// A run made under earlier rules is refused, with no step changed.
    #[test]
    fn continue_refuses_earlier_rules() {
        let (mut c, p) = store_with(&trial_md(""));
        let id = stopped_after_mortality(&mut c, &p);
        execute(
            &mut c,
            "UPDATE evidence_runs SET algorithm_version = 'rob2/0' WHERE id = ?",
            &[bmlib::db::Value::from(id)],
        )
        .unwrap();
        let before = statuses(&mut c, id);
        let mut chat = FakeChat::trial();
        let e = resume(&mut c, id, &mut chat, &AtomicBool::new(false)).unwrap_err();
        assert!(matches!(e, EvidenceRunError::EarlierRules), "{e}");
        assert!(chat.asked.is_empty());
        assert_eq!(statuses(&mut c, id), before);
    }

    /// Every step of run `id` with its stored result: each step (status and
    /// failure included), each domain's row and each overall judgement.
    fn snapshot(
        c: &mut Connection,
        id: i64,
    ) -> Vec<(
        StoredStep,
        Option<StoredDomain>,
        Option<StoredOutcomeResult>,
    )> {
        stored(c, id)
            .steps
            .into_iter()
            .map(|s| match s.step {
                EvidenceStep::Rob2(Rob2Step::Domain { .. }) => {
                    let d = stored_domain(c, s.id).unwrap();
                    (s, d, None)
                }
                EvidenceStep::Rob2(Rob2Step::Escalation { .. }) => {
                    let r = stored_outcome_result(c, s.id).unwrap();
                    (s, None, r)
                }
                _ => (s, None, None),
            })
            .collect()
    }

    /// A Continue whose window is too small for the run's outcomes is
    /// refused as `TooLong` before it sets anything back: the failed D3, the
    /// D5 with a failed second look and their overall judgement are as they
    /// were, and nothing is asked.
    #[test]
    fn continue_too_long_changes_nothing() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|a| match a {
            Asked::Domain(Domain::D3, Some(o)) if o == MORTALITY => unreadable(),
            Asked::Facts(o) if o == MORTALITY => facts_reply(unreported()),
            Asked::Look(_) => server_error(),
            other => reply_to(other),
        });
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            status(&mut c, id, dom(Domain::D3, Some(0))),
            StepStatus::Failed
        );
        let before = snapshot(&mut c, id);
        let d5 = before
            .iter()
            .find(|(s, _, _)| s.step == dom(Domain::D5, Some(0)))
            .unwrap();
        let look = &d5.1.as_ref().unwrap().d5.as_ref().unwrap().measurements[0];
        assert!(look.second_look_error.is_some());
        let mut chat = FakeChat::trial();
        let e = continue_evidence(
            &mut context(&mut c, &mut chat, Some(REPLY)),
            id,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(
            matches!(
                e,
                EvidenceRunError::TooLong {
                    window: Some(REPLY)
                }
            ),
            "{e}"
        );
        assert!(chat.asked.is_empty());
        assert_eq!(snapshot(&mut c, id), before);
        assert!(stored(&mut c, id).too_long, "the run is marked");
    }

    /// A window with room for the design's, the outcomes' and D1's prompts
    /// but not for the outcomes' D2–D5 stops the run as `TooLong` once the
    /// outcomes are stored, and marks it; a Continue on the same window is
    /// refused again with nothing asked (#166).
    #[test]
    fn too_long_after_the_outcomes_marks_the_run() {
        let (mut c, p) = store_with(&trial_md(""));
        let window = (REPLY..)
            .find(|&w| budget_chars(Some(w), REPLY, TITLE, &[]).unwrap().is_some())
            .unwrap();
        let mut chat = FakeChat::trial();
        let stop = AtomicBool::new(false);
        let e = run_evidence(&mut context(&mut c, &mut chat, Some(window)), &p, &stop);
        assert!(
            matches!(e, Err(EvidenceRunError::TooLong { window: Some(w) }) if w == window),
            "{e:?}"
        );
        assert_eq!(chat.asked, [Asked::Design, Asked::Outcomes]);
        let id = bmlib::db::fetch_one(&mut c, "SELECT id FROM evidence_runs", &[])
            .unwrap()
            .unwrap()
            .get_i64("id")
            .unwrap();
        let run = stored(&mut c, id);
        assert_eq!(run.status, RunStatus::Incomplete);
        assert!(run.too_long);
        let mut chat = FakeChat::trial();
        let e = continue_evidence(&mut context(&mut c, &mut chat, Some(window)), id, &stop);
        assert!(matches!(e, Err(EvidenceRunError::TooLong { .. })), "{e:?}");
        assert!(chat.asked.is_empty());
    }

    /// A stop already set keeps the failed steps and their messages.
    #[test]
    fn continue_when_stopped_keeps_failures() {
        let (mut c, p) = store_with(&trial_md(""));
        let mut chat = FakeChat::new(|a| match a {
            Asked::Domain(Domain::D3, Some(o)) if o == MORTALITY => unreadable(),
            other => reply_to(other),
        });
        let id = start(&mut c, &p, &mut chat, &AtomicBool::new(false)).unwrap();
        let mut chat = FakeChat::trial();
        let e = resume(&mut c, id, &mut chat, &AtomicBool::new(true)).unwrap_err();
        assert!(matches!(e, EvidenceRunError::Cancelled), "{e}");
        assert!(chat.asked.is_empty());
        let d3 = step_of(&mut c, id, dom(Domain::D3, Some(0)));
        assert_eq!(d3.status, StepStatus::Failed);
        assert!(d3.failure.unwrap().message.contains("Unclear"));
    }
}
