//! Running PRISMA 2020 on a paper (design 2026-11-18, §3, §6, §7): the
//! suitability check, then the checklist a part at a time, each part stored
//! the moment it finishes ([`crate::prisma_store`]). bmlib's
//! [`Prisma2020Agent`] asks, checks the quotes and condenses a long paper;
//! this sizes its budget, halves a part whose reply was cut off, retries,
//! honours a stop, and runs the stored parts until none is pending. The
//! store adds the follow-ups (missing items, re-checks) as it saves a part,
//! so this loop only ever runs what is pending. Continue
//! ([`continue_prisma`]) runs what an earlier call left.

use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use bmlib::context_processor::llm_processor::ContextModel;
use bmlib::context_processor::ProcessingConfig;
use bmlib::db::Db;
use bmlib::fulltext::canonical::CanonicalText;
use bmlib::quality::prisma2020_agent::{SUITABILITY_RESPONSE_FORMAT, SUITABILITY_TASK_PROMPT};
use bmlib::quality::prisma2020_checklist::item;
use bmlib::quality::{
    part_prompt_chars, plan_parts, JsonChat, PartPlan, PartsError, PicoError, Prisma2020Agent,
    PrismaCondenser, PrismaError, CHECKLIST, PRISMA_CONDENSE_CONSOLIDATION_PROMPT,
    PRISMA_CONDENSE_EXTRACTION_PROMPT, PRISMA_SYSTEM_PROMPT, SUITABILITY_CONDENSE_QUERY,
};
use serde::Serialize;

use crate::config::{
    FIT_CHARS_PER_TOKEN, MIN_QUOTE_SIMILARITY, PRISMA_MIN_BUDGET_CHARS, PRISMA_TEMPERATURE,
    RETRIEVAL_ATTEMPTS,
};
use crate::interrogate::{full_text, AskError};
use crate::pico::{abstract_text, is_set, paper_budget_chars, PicoEvent, PicoRunError};
use crate::prisma_store::{
    create_run, fail_part, finish_run, pending_parts, reset_failed, run as stored_run,
    save_not_suitable, save_part, split_part, PrismaCheck, KIND_RECHECK, PART_FAILED, PART_PENDING,
    RUN_COMPLETE,
};
use crate::retry::with_backoff;
use crate::store::{PaperId, StoreError};

/// What a PRISMA run works with: [`crate::pico::PicoContext`]'s fields, with
/// PRISMA's events.
pub struct PrismaContext<'a> {
    /// The store.
    pub db: &'a mut dyn Db,
    /// The model, for the check and the parts.
    pub chat: &'a mut dyn JsonChat,
    /// The model, for condensing a long paper.
    pub condenser_model: &'a dyn ContextModel,
    /// The chosen `provider:model`, stored with a new run. Continue passes
    /// the run's own.
    pub model: String,
    /// The model's window in tokens, `None` when unknown.
    pub window: Option<u64>,
    /// The room for each reply in tokens: every call's `max_tokens`, what
    /// the paper's budget keeps free, and what the checklist is split to
    /// fit (`prompt::reply_tokens`).
    pub reply_tokens: u64,
    /// The first backoff wait between attempts (the command passes
    /// `RETRY_BASE_DELAY`, tests zero).
    pub retry_delay: Duration,
    /// Told which part is running and how far a condensation has got.
    pub on_event: Arc<dyn Fn(PrismaEvent) + Send + Sync>,
}

/// What the user sees while a PRISMA run works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrismaEvent {
    /// A part has begun.
    Part {
        /// The parts begun in this call so far, this one included, from 1.
        index: usize,
        /// The parts this call runs as known now: those pending when it
        /// started, plus those added since (halves, missing items,
        /// re-checks). It can grow, never below `index`.
        of: usize,
        /// The part's items, as `"Methods 5–13c"`.
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
    /// The part that follows checks key item `item` again on a condensation
    /// of its own (§6.5).
    Rechecking {
        /// The checklist id, for example `"7"`.
        item: String,
    },
    /// A condensation stage this version does not know, shown as bmlib
    /// words it.
    Other {
        /// bmlib's name for the stage.
        stage: String,
        /// bmlib's description of it.
        message: String,
    },
}

impl From<PicoEvent> for PrismaEvent {
    /// The same condensation progress, as PRISMA's event: PICO's mapping of
    /// bmlib's stages ([`PicoEvent::from_progress`]) serves both tabs.
    fn from(event: PicoEvent) -> Self {
        match event {
            PicoEvent::Condensing {
                batch,
                batches,
                level,
            } => PrismaEvent::Condensing {
                batch,
                batches,
                level,
            },
            PicoEvent::Condensed => PrismaEvent::Condensed,
            PicoEvent::Other { stage, message } => PrismaEvent::Other { stage, message },
        }
    }
}

/// Why a call stopped short. A run already started keeps every part it
/// finished, whatever the error.
#[derive(Debug)]
pub enum PrismaRunError {
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
    /// The reply room cannot hold checklist item `item` even alone (§6.2).
    ReplyTooSmall {
        /// The first item that does not fit.
        item: String,
    },
    /// The user stopped the run.
    Cancelled,
    /// bmlib's agent failed, after its retries.
    Prisma(PrismaError),
    /// The store failed.
    Store(StoreError),
    /// A defect in this code or the stored run: a stored item no longer on
    /// the checklist, or Continue asked of a complete run.
    Internal(String),
}

impl From<StoreError> for PrismaRunError {
    /// A store failure as the run's error.
    fn from(e: StoreError) -> Self {
        PrismaRunError::Store(e)
    }
}

impl fmt::Display for PrismaRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The refusals PICO shares read as PICO's, so both tabs say the same.
        let mut pico = |e: PicoRunError| write!(f, "{e}");
        match self {
            PrismaRunError::NoFullText => pico(PicoRunError::NoFullText),
            PrismaRunError::NotFullText => pico(PicoRunError::NotFullText),
            PrismaRunError::TooLong { window } => pico(PicoRunError::TooLong { window: *window }),
            PrismaRunError::ReplyTooSmall { .. } => write!(
                f,
                "The model's reply room is too small for a PRISMA item; choose another model."
            ),
            PrismaRunError::Cancelled => write!(f, "The PRISMA 2020 assessment was stopped."),
            PrismaRunError::Prisma(PrismaError::UnusableReply(msg)) => {
                pico(PicoRunError::Pico(PicoError::UnusableReply(msg.clone())))
            }
            PrismaRunError::Prisma(PrismaError::Truncated(msg)) => {
                pico(PicoRunError::Pico(PicoError::Truncated(msg.clone())))
            }
            PrismaRunError::Prisma(e) => write!(f, "{e}"),
            PrismaRunError::Store(e) => write!(f, "{e}"),
            PrismaRunError::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PrismaRunError {}

/// The characters of paper a run can send to a model with `window` tokens:
/// [`crate::pico::budget_chars`]'s arithmetic, with the longer of the
/// longest prompt of `plan`'s parts ([`part_prompt_chars`]) and the
/// suitability check's prompt in place of PICO's longest prompt, since the
/// check may send the whole paper too. `None` when that is under
/// [`PRISMA_MIN_BUDGET_CHARS`].
pub fn budget_chars(
    window: Option<u64>,
    reply: u64,
    title: &str,
    plan: &[PartPlan],
) -> Option<usize> {
    let prompt = longest_part_prompt(plan).max(check_prompt_chars());
    paper_budget_chars(window, reply, prompt, title, PRISMA_MIN_BUDGET_CHARS)
}

/// The characters of the suitability check's longest prompt, without the
/// paper and its title, measured as bmlib's [`part_prompt_chars`] measures a
/// part's: the system prompt and the longest of the check's question (as
/// bmlib wraps it) and the two condensation prompts with the check's query.
/// bmlib has no helper for it.
fn check_prompt_chars() -> usize {
    let condense = |template: &str| {
        template
            .replace("{query}", SUITABILITY_CONDENSE_QUERY)
            .replace("{content}", "")
    };
    let longest = [
        // bmlib's `wrapped_prompt` with an empty title and text.
        format!(
            "{SUITABILITY_TASK_PROMPT}\n\nPaper Title: \n\nPaper Text:\n\n\n{SUITABILITY_RESPONSE_FORMAT}"
        ),
        condense(PRISMA_CONDENSE_EXTRACTION_PROMPT),
        condense(PRISMA_CONDENSE_CONSOLIDATION_PROMPT),
    ]
    .iter()
    .map(|p| p.chars().count())
    .max()
    .unwrap_or(0);
    PRISMA_SYSTEM_PROMPT.chars().count() + longest
}

/// [`budget_chars`] for `plan`'s parts alone: what Continue sizes, as it
/// runs no check.
fn parts_budget_chars(
    window: Option<u64>,
    reply: u64,
    title: &str,
    plan: &[PartPlan],
) -> Option<usize> {
    let prompt = longest_part_prompt(plan);
    paper_budget_chars(window, reply, prompt, title, PRISMA_MIN_BUDGET_CHARS)
}

/// The characters of the longest prompt of `plan`'s parts, 0 for none.
fn longest_part_prompt(plan: &[PartPlan]) -> usize {
    plan.iter()
        .map(|part| part_prompt_chars(&part.item_ids))
        .max()
        .unwrap_or(0)
}

/// Starts a PRISMA run on `paper`'s latest text and returns its id. The
/// checklist is split to the reply room and the paper's budget sized before
/// any call, so a model that cannot hold an item, or a window too small,
/// is refused with nothing stored. Then, unless `force`, the suitability
/// check reads the abstract (or, without one, the paper as the parts see
/// it); a paper found not suitable is stored as such and the run ends. A
/// suitable or forced paper gets a run of `pending` parts, run in order.
///
/// Each part is stored as it finishes. A stop or a failure keeps every part
/// finished (a reply that arrives as Stop is pressed included, §7) and
/// returns [`PrismaRunError::Cancelled`] or the failure, the run left
/// `incomplete` for [`continue_prisma`]. A part whose reply, asked again,
/// still left items out fails without ending the call: the rest run, and
/// the run's id comes back with the run `incomplete`.
pub fn run_prisma(
    ctx: &mut PrismaContext,
    paper: &PaperId,
    force: bool,
    stop: &AtomicBool,
) -> Result<i64, PrismaRunError> {
    let result = run_inner(ctx, paper, force, stop);
    log_failure(&result, &format!("paper {}", paper.as_str()));
    result
}

/// Continues run `run_id` on its own canonical text, even if the paper has a
/// newer one: its failed parts are made pending again, then every pending
/// part runs as in [`run_prisma`], with `ctx.model` (the command passes the
/// run's). The suitability check never runs again. A complete run (a
/// finished one, or one stopped at its check) has nothing to continue and
/// is refused as [`PrismaRunError::Internal`]; no such run is
/// [`StoreError::NotFound`].
pub fn continue_prisma(
    ctx: &mut PrismaContext,
    run_id: i64,
    stop: &AtomicBool,
) -> Result<i64, PrismaRunError> {
    let result = continue_inner(ctx, run_id, stop);
    log_failure(&result, &format!("PRISMA run {run_id}"));
    result
}

/// Logs a call's failure as [`crate::pico::run_pico`] does: a stop as info,
/// a store failure or defect as an error, a model failure as a warning.
fn log_failure(result: &Result<i64, PrismaRunError>, what: &str) {
    match result {
        Ok(_) => {}
        Err(PrismaRunError::Cancelled) => log::info!("{what}: PRISMA run stopped"),
        Err(e @ (PrismaRunError::Store(_) | PrismaRunError::Internal(_))) => {
            log::error!("{what}: PRISMA run failed: {e}")
        }
        Err(e) => log::warn!("{what}: PRISMA run failed: {e}"),
    }
}

/// [`run_prisma`] without the logging.
fn run_inner(
    ctx: &mut PrismaContext,
    paper: &PaperId,
    force: bool,
    stop: &AtomicBool,
) -> Result<i64, PrismaRunError> {
    let stored = full_text(ctx.db, paper).map_err(|e| match e {
        AskError::NoFullText => PrismaRunError::NoFullText,
        AskError::NotFullText => PrismaRunError::NotFullText,
        AskError::Store(e) => PrismaRunError::Store(e),
        other => PrismaRunError::Internal(other.to_string()),
    })?;
    let canonical = &stored.text;
    // The reply room in characters, as `budget_chars` converts tokens.
    let reply_chars =
        usize::try_from(ctx.reply_tokens.saturating_mul(FIT_CHARS_PER_TOKEN)).unwrap_or(usize::MAX);
    let rows: Vec<_> = CHECKLIST.iter().collect();
    let plan = plan_parts(&rows, reply_chars).map_err(|e| match e {
        PartsError::ItemTooLarge { id, .. } => PrismaRunError::ReplyTooSmall {
            item: id.to_string(),
        },
    })?;
    let budget = budget_chars(ctx.window, ctx.reply_tokens, title_of(canonical), &plan)
        .ok_or(PrismaRunError::TooLong { window: ctx.window })?;
    let model = ctx.model.clone();
    with_agent(ctx, budget, stop, |db, agent, calls| {
        let check = if force {
            None
        } else {
            Some(check(agent, canonical, calls)?)
        };
        if let Some(check) = check.as_ref().filter(|c| !c.suitability.is_suitable) {
            // Kept even when Stop was pressed as it arrived (§7).
            let id = save_not_suitable(db, paper, stored.id, &model, check)?;
            cancelled(stop)?;
            return Ok(id);
        }
        let run_id = create_run(db, paper, stored.id, &model, check.as_ref(), force, &plan)?;
        drive(db, agent, calls, run_id, canonical)?;
        Ok(run_id)
    })
}

/// [`continue_prisma`] without the logging.
fn continue_inner(
    ctx: &mut PrismaContext,
    run_id: i64,
    stop: &AtomicBool,
) -> Result<i64, PrismaRunError> {
    let run = stored_run(ctx.db, run_id)?
        .ok_or_else(|| StoreError::NotFound(format!("PRISMA run {run_id}")))?;
    if run.status == RUN_COMPLETE {
        return Err(PrismaRunError::Internal(format!(
            "PRISMA run {run_id} is complete: there is nothing to continue"
        )));
    }
    // The budget is sized for the parts left, before anything changes.
    let left = run
        .parts
        .iter()
        .filter(|p| p.status == PART_PENDING || p.status == PART_FAILED)
        .map(|p| {
            Ok(PartPlan {
                item_ids: static_ids(&p.item_ids)?,
            })
        })
        .collect::<Result<Vec<_>, PrismaRunError>>()?;
    let budget = parts_budget_chars(ctx.window, ctx.reply_tokens, title_of(&run.text), &left)
        .ok_or(PrismaRunError::TooLong { window: ctx.window })?;
    reset_failed(ctx.db, run_id)?;
    with_agent(ctx, budget, stop, |db, agent, calls| {
        drive(db, agent, calls, run_id, &run.text)
    })?;
    Ok(run_id)
}

/// What every call of a run shares: where its events go, the first backoff
/// wait, and the stop.
struct Calls<'s> {
    /// The context's listener.
    on_event: Arc<dyn Fn(PrismaEvent) + Send + Sync>,
    /// The first backoff wait.
    delay: Duration,
    /// Set when the user pressed Stop.
    stop: &'s AtomicBool,
}

impl Calls<'_> {
    /// Whether a retry may cure `e`: a request that failed but may succeed
    /// again (never a 401 or 404, bmlib #659), and only while not stopped. A
    /// condensation's failed request is a [`PrismaError::RequestFailed`] too
    /// (#616), so it is retried the same way.
    fn retryable(&self, e: &PrismaError) -> bool {
        matches!(
            e,
            PrismaError::RequestFailed {
                retryable: true,
                ..
            }
        ) && !is_set(self.stop)
    }

    /// `e` as the run's error: [`PrismaRunError::Cancelled`] once stopped
    /// (a failure under way still reaches the log), else
    /// [`PrismaRunError::Prisma`].
    fn failed(&self, e: PrismaError) -> PrismaRunError {
        if is_set(self.stop) || matches!(e, PrismaError::Stopped(_)) {
            if !matches!(e, PrismaError::Stopped(None)) {
                log::warn!("PRISMA call failed as it stopped: {e}");
            }
            PrismaRunError::Cancelled
        } else {
            PrismaRunError::Prisma(e)
        }
    }

    /// Tells the listener `event`.
    fn emit(&self, event: PrismaEvent) {
        (self.on_event)(event);
    }
}

/// Builds the agent for `ctx` (its chat, a [`PrismaCondenser`] over its
/// condenser model reporting progress as events, the reply room as every
/// call's `max_tokens`, `budget` characters of paper, and `stop`) and runs
/// `body` with the store and it.
fn with_agent<T>(
    ctx: &mut PrismaContext,
    budget: usize,
    stop: &AtomicBool,
    body: impl FnOnce(&mut dyn Db, &mut Prisma2020Agent, &Calls) -> Result<T, PrismaRunError>,
) -> Result<T, PrismaRunError> {
    let max_tokens = i64::try_from(ctx.reply_tokens).unwrap_or(i64::MAX);
    let calls = Calls {
        on_event: Arc::clone(&ctx.on_event),
        delay: ctx.retry_delay,
        stop,
    };
    let on_event = Arc::clone(&ctx.on_event);
    let mut condenser = PrismaCondenser::new(ctx.condenser_model, PRISMA_TEMPERATURE, max_tokens)
        .with_progress(move |info| {
            if let Some(event) = PicoEvent::from_progress(info) {
                on_event(event.into());
            }
        });
    let mut agent = Prisma2020Agent::new(&mut *ctx.chat)
        .with_condenser(&mut condenser)
        .with_temperature(PRISMA_TEMPERATURE)
        .with_max_tokens(max_tokens)
        .with_condense_config(ProcessingConfig::default().with_max_context_chars(budget))
        .with_stop(stop);
    body(&mut *ctx.db, &mut agent, &calls)
}

/// The suitability check (§3): on the abstract when the paper has one, else
/// on the whole text, which the agent condenses to the budget as it does
/// for a part.
fn check(
    agent: &mut Prisma2020Agent,
    canonical: &CanonicalText,
    calls: &Calls,
) -> Result<PrismaCheck, PrismaRunError> {
    let abstract_ = abstract_text(canonical);
    let text = abstract_.as_deref().unwrap_or(&canonical.text);
    let suitability = with_backoff(
        RETRIEVAL_ATTEMPTS,
        calls.delay,
        |e| calls.retryable(e),
        || agent.check_suitability(title_of(canonical), text),
    )
    .map_err(|e| calls.failed(e))?;
    Ok(PrismaCheck {
        suitability,
        from_abstract: abstract_.is_some(),
    })
}

/// Runs run `run_id`'s pending parts on `canonical` in id order until none
/// is left, then marks the run complete if nothing failed. Each part is
/// prepared once (whole, or condensed for its items) and assessed, both
/// retried; its assessment is stored at once, and the store adds any
/// follow-up part. A part cut off with more than one item is halved and the
/// loop goes on. A stop (checked before each part, and after a reply, which
/// is kept) ends the call as [`PrismaRunError::Cancelled`]; any other
/// failure fails the part and ends the call with it.
fn drive(
    db: &mut dyn Db,
    agent: &mut Prisma2020Agent,
    calls: &Calls,
    run_id: i64,
    canonical: &CanonicalText,
) -> Result<(), PrismaRunError> {
    let title = title_of(canonical);
    let mut begun = 0;
    loop {
        let pending = pending_parts(db, run_id)?;
        let Some(part) = pending.first() else {
            break;
        };
        cancelled(calls.stop)?;
        let ids = static_ids(&part.item_ids)?;
        begun += 1;
        if part.kind == KIND_RECHECK {
            calls.emit(PrismaEvent::Rechecking {
                item: part.item_ids.join(", "),
            });
        }
        calls.emit(PrismaEvent::Part {
            index: begun,
            of: begun - 1 + pending.len(),
            label: PartPlan {
                item_ids: ids.clone(),
            }
            .label(),
        });
        let assessed = with_backoff(
            RETRIEVAL_ATTEMPTS,
            calls.delay,
            |e| calls.retryable(e),
            || agent.prepare_part(title, &canonical.text, &ids),
        )
        .and_then(|prepared| {
            let condensed = prepared.condensed_from_chars().is_some();
            with_backoff(
                RETRIEVAL_ATTEMPTS,
                calls.delay,
                |e| calls.retryable(e),
                || agent.assess_part(&prepared, &ids, MIN_QUOTE_SIMILARITY),
            )
            .map(|assessment| (assessment, condensed))
        });
        match assessed {
            Ok((assessment, condensed)) => {
                save_part(db, part.id, &assessment, condensed)?;
                if is_set(calls.stop) {
                    // The reply is kept; a run it finished is marked so first.
                    finish_run(db, run_id)?;
                    return Err(PrismaRunError::Cancelled);
                }
            }
            Err(PrismaError::Truncated(why)) if ids.len() > 1 => {
                log::info!(
                    "PRISMA run {run_id}: part {} was cut off ({why}); asking it in halves",
                    part.id
                );
                let (first, second) = ids.split_at(ids.len().div_ceil(2));
                split_part(db, part.id, [first, second])?;
            }
            Err(e) => {
                let e = calls.failed(e);
                if let PrismaRunError::Prisma(p) = &e {
                    if let Err(store) = fail_part(db, part.id, p.kind(), &e.to_string()) {
                        // The store's failure is returned; the model's must not be lost.
                        log::warn!("PRISMA run {run_id}: part {} failed: {e}", part.id);
                        return Err(store.into());
                    }
                }
                return Err(e);
            }
        }
    }
    if !finish_run(db, run_id)? {
        log::warn!("PRISMA run {run_id} ended with failed parts; Continue asks them again");
    }
    Ok(())
}

/// Stored item ids as the checklist's own `&'static str`s. An id no longer
/// on the checklist is [`PrismaRunError::Internal`], naming it.
fn static_ids(ids: &[String]) -> Result<Vec<&'static str>, PrismaRunError> {
    ids.iter()
        .map(|id| {
            item(id).map(|row| row.id).ok_or_else(|| {
                PrismaRunError::Internal(format!(
                    "stored item {id:?} is not on the PRISMA 2020 checklist"
                ))
            })
        })
        .collect()
}

/// The paper's title, or `""` when it has none.
fn title_of(canonical: &CanonicalText) -> &str {
    canonical
        .title_span
        .and_then(|s| canonical.span_text(s))
        .unwrap_or("")
}

/// [`PrismaRunError::Cancelled`] once `stop` is set.
fn cancelled(stop: &AtomicBool) -> Result<(), PrismaRunError> {
    if is_set(stop) {
        Err(PrismaRunError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MAX_REPLY_TOKENS, MIN_REPLY_TOKENS};
    use crate::identifiers::Identifiers;
    use crate::intake::SourceKind;
    use crate::prisma_store::{
        latest_run, StoredPrismaRun, ERROR_MISSING_ITEMS, KIND_HALF, KIND_MISSING, KIND_PLAN,
        PART_DONE, PART_SPLIT, RUN_INCOMPLETE,
    };
    use crate::store::{
        confirm_full_text, create_paper, open_memory_store, save_canonical, save_source, NewSource,
        SourcePlace,
    };
    use bmlib::agents::{ChatJsonError, ChatJsonOutcome};
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_markdown};
    use bmlib::llm::LLMMessage;
    use bmlib::quality::part_query;
    use bmlib::quality::prisma2020_models::Judgement;
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;

    /// The review's title.
    const TITLE: &str = "Statins review";

    /// A small systematic review: an abstract, methods and results.
    const REVIEW: &str = "# Statins review\n\n## Abstract\n\nWe systematically reviewed trials of statins.\n\n## Methods\n\nWe searched MEDLINE and Embase to March 2024.\n\n## Results\n\nTwelve trials were included.\n";

    /// The passage every answered item quotes, as the paper words it.
    const SEARCHED: &str = "We searched MEDLINE and Embase to March 2024.";

    /// What the fake condenser model returns for any call.
    const DIGEST: &str = "Passages on the search.";

    /// A window the whole review fits in.
    const WIDE: Option<u64> = Some(1_000_000);

    /// Words only the suitability check's prompt holds.
    const CHECK_PROMPT: &str = "SYSTEMATIC REVIEW or META-ANALYSIS";

    /// Stores `md` as a new confirmed text of `p`; returns its id.
    fn add_text(c: &mut Connection, p: &PaperId, md: &str) -> i64 {
        let canonical = canonical_from_markdown(md);
        let sid = save_source(
            c,
            p,
            NewSource {
                kind: SourceKind::Markdown,
                origin: "paste".into(),
                file_name: None,
                bytes: md.as_bytes().to_vec(),
                decoded: None,
            },
            SourcePlace::App,
        )
        .unwrap();
        let a = assess_full_text(&canonical).unwrap();
        let cid = save_canonical(c, p, sid, &canonical, &a).unwrap();
        confirm_full_text(c, p, cid).unwrap();
        cid
    }

    /// A store holding one paper with `md` as its text, and that text's id.
    fn store_with(md: &str) -> (Connection, PaperId, i64) {
        let mut c = open_memory_store().unwrap();
        let ids = Identifiers {
            pmid: Some("1".into()),
            ..Default::default()
        };
        let p = create_paper(&mut c, &ids, Some("Review")).unwrap();
        let cid = add_text(&mut c, &p, md);
        (c, p, cid)
    }

    /// The review with `n` sentences of filler, too long for a tight window.
    fn long_review(n: usize) -> String {
        let filler: Vec<String> = (0..n)
            .map(|i| format!("Filler sentence number {i} says nothing."))
            .collect();
        format!("{REVIEW}\n{}\n", filler.join(" "))
    }

    /// One scripted reply of the fake chat.
    type Reply = Result<Value, ChatJsonError>;

    /// How the fake chat answers call `n` (0-based) asking `ids` (none for
    /// the check).
    type Respond<'s> = Box<dyn FnMut(usize, &[&'static str]) -> Reply + 's>;

    /// A chat that answers through a closure, records each user message and
    /// the ids it asked, and can set `stop` during a given call.
    struct FakeChat<'s> {
        respond: Respond<'s>,
        sent: Vec<String>,
        asked: Vec<Vec<&'static str>>,
        stop_on: Option<(usize, &'s AtomicBool)>,
    }

    impl<'s> FakeChat<'s> {
        /// A chat answering through `respond`.
        fn new(respond: impl FnMut(usize, &[&'static str]) -> Reply + 's) -> Self {
            FakeChat {
                respond: Box::new(respond),
                sent: Vec::new(),
                asked: Vec::new(),
                stop_on: None,
            }
        }

        /// A chat that passes the check and reports every item.
        fn answering() -> Self {
            Self::new(|_, ids| {
                if ids.is_empty() {
                    suitable(true)
                } else {
                    answer(ids, |_| "reported")
                }
            })
        }

        /// Sets `stop` during call `at`.
        fn stopping(mut self, at: usize, stop: &'s AtomicBool) -> Self {
            self.stop_on = Some((at, stop));
            self
        }

        /// The ids each part asked, in order; the check left out.
        fn parts_asked(&self) -> Vec<Vec<&'static str>> {
            self.asked
                .iter()
                .filter(|a| !a.is_empty())
                .cloned()
                .collect()
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
            let n = self.sent.len();
            let prompt = messages.last().unwrap().content.clone();
            let ids = asked_ids(&prompt);
            self.sent.push(prompt);
            self.asked.push(ids.clone());
            if let Some((at, stop)) = self.stop_on {
                if at == n {
                    stop.store(true, Ordering::SeqCst);
                }
            }
            (self.respond)(n, &ids).map(|value| ChatJsonOutcome {
                value,
                attempts: 1,
                repaired: false,
            })
        }
    }

    /// The checklist ids a part's prompt lists (`Item 7: Search strategy`).
    fn asked_ids(prompt: &str) -> Vec<&'static str> {
        prompt
            .lines()
            .filter_map(|l| l.strip_prefix("Item ")?.split_once(':'))
            .filter_map(|(id, _)| item(id))
            .map(|row| row.id)
            .collect()
    }

    /// A part reply judging each of `ids` as `judge` says, quoting
    /// [`SEARCHED`] for every judgement but `not_reported`.
    fn answer(ids: &[&str], judge: impl Fn(&str) -> &'static str) -> Reply {
        let items: Vec<Value> = ids
            .iter()
            .map(|id| {
                let j = judge(id);
                let quotes: Vec<&str> = if j == "not_reported" {
                    vec![]
                } else {
                    vec![SEARCHED]
                };
                json!({"id": id, "judgement": j, "explanation": format!("{id} judged"), "quotes": quotes})
            })
            .collect();
        Ok(json!({ "items": items }))
    }

    /// `ids` without `gone`.
    fn without(ids: &[&'static str], gone: &str) -> Vec<&'static str> {
        ids.iter().copied().filter(|id| *id != gone).collect()
    }

    /// A suitability reply.
    fn suitable(yes: bool) -> Reply {
        Ok(json!({
            "is_systematic_review": yes,
            "is_meta_analysis": false,
            "document_type": if yes { "systematic review" } else { "narrative review" },
            "variant": null,
            "rationale": "Because.",
            "confidence": 0.9
        }))
    }

    /// A transport failure a retry may cure.
    fn transport() -> Reply {
        Err(ChatJsonError::Transport {
            message: "connection reset".into(),
            attempts: 1,
            retryable: true,
            status: None,
        })
    }

    /// A refusal no retry cures, such as a 401.
    fn refused() -> Reply {
        Err(ChatJsonError::Transport {
            message: "HTTP 401".into(),
            attempts: 1,
            retryable: false,
            status: Some(401),
        })
    }

    /// A reply cut off at its length limit.
    fn truncated() -> Reply {
        Err(ChatJsonError::Truncated {
            stop_reason: "length".into(),
            budget: 4096,
            attempts: 1,
        })
    }

    /// A condenser model returning [`DIGEST`] and recording each prompt.
    #[derive(Default)]
    struct FakeModel {
        prompts: RefCell<Vec<String>>,
    }

    impl ContextModel for FakeModel {
        fn complete(&self, prompt: &str, _t: f64, _m: i64) -> Result<String, String> {
            self.prompts.borrow_mut().push(prompt.to_string());
            Ok(DIGEST.to_string())
        }

        fn complete_json(&self, _prompt: &str, _t: f64, _m: i64) -> Result<Value, String> {
            Err("not used by the PRISMA condenser".to_string())
        }
    }

    /// A context over `c`, `chat` and `model`, with zero retry delay and no
    /// event listener.
    fn context<'a, 's: 'a>(
        c: &'a mut Connection,
        chat: &'a mut FakeChat<'s>,
        model: &'a FakeModel,
        window: Option<u64>,
        reply_tokens: u64,
    ) -> PrismaContext<'a> {
        PrismaContext {
            db: c,
            chat,
            condenser_model: model,
            model: "ollama:m".into(),
            window,
            reply_tokens,
            retry_delay: Duration::ZERO,
            on_event: Arc::new(|_| {}),
        }
    }

    /// The checklist split for `reply_tokens` of reply room.
    fn plan_for(reply_tokens: u64) -> Vec<PartPlan> {
        let rows: Vec<_> = CHECKLIST.iter().collect();
        plan_parts(&rows, (reply_tokens * FIT_CHARS_PER_TOKEN) as usize).unwrap()
    }

    /// The ids of each part of `plan`.
    fn ids_of(plan: &[PartPlan]) -> Vec<Vec<&'static str>> {
        plan.iter().map(|p| p.item_ids.clone()).collect()
    }

    /// A reply room that splits the checklist into `n` parts.
    fn reply_for_parts(n: usize) -> u64 {
        (MIN_REPLY_TOKENS..=MAX_REPLY_TOKENS)
            .step_by(16)
            .find(|&r| plan_for(r).len() == n)
            .unwrap()
    }

    /// The smallest window (in steps of 64 tokens) that leaves the review a
    /// budget: [`long_review`] is condensed in it.
    fn tight_window(reply_tokens: u64) -> Option<u64> {
        let plan = plan_for(reply_tokens);
        (0..)
            .map(|k| k * 64)
            .find(|&w| budget_chars(Some(w), reply_tokens, TITLE, &plan).is_some())
    }

    /// The stored run `id`.
    fn stored(c: &mut Connection, id: i64) -> StoredPrismaRun {
        stored_run(c, id).unwrap().unwrap()
    }

    /// The newest run on text `cid` of `p`.
    fn latest(c: &mut Connection, p: &PaperId, cid: i64) -> i64 {
        latest_run(c, p, cid).unwrap().unwrap()
    }

    /// Each part's status, in id order.
    fn statuses(run: &StoredPrismaRun) -> Vec<&str> {
        run.parts.iter().map(|p| p.status.as_str()).collect()
    }

    /// The number of stored runs.
    fn runs_stored(c: &mut Connection) -> i64 {
        bmlib::db::fetch_one(c, "SELECT count(*) AS n FROM prisma_runs", &[])
            .unwrap()
            .unwrap()
            .get_i64("n")
            .unwrap()
    }

    /// The shown row of `id` in `run`.
    fn shown<'r>(run: &'r StoredPrismaRun, id: &str) -> &'r crate::prisma_store::StoredPrismaItem {
        run.items.iter().find(|i| i.assessment.id == id).unwrap()
    }

    /// A failed check is the whole run: one call, its check stored, no
    /// parts, complete.
    #[test]
    fn not_suitable_stores_check_only() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|_, _| suitable(false));
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, false, &stop).unwrap();
        assert_eq!(chat.sent.len(), 1);
        let run = stored(&mut c, id);
        assert!(!run.check.unwrap().suitability.is_suitable);
        assert!(!run.forced);
        assert!(run.parts.is_empty());
        assert_eq!(run.status, RUN_COMPLETE);
    }

    /// A forced run asks no check, stores none, and assesses every item.
    #[test]
    fn force_skips_check() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::answering();
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        assert!(!chat.sent.iter().any(|s| s.contains(CHECK_PROMPT)));
        assert_eq!(chat.sent.len(), plan_for(MIN_REPLY_TOKENS).len());
        let run = stored(&mut c, id);
        assert_eq!(run.check, None);
        assert!(run.forced);
        assert_eq!(run.status, RUN_COMPLETE);
        assert_eq!(run.items.len(), CHECKLIST.len());
    }

    /// The check reads the abstract, never the results, and says so; without
    /// an abstract it reads the whole text.
    #[test]
    fn check_reads_abstract_when_present() {
        for (md, from_abstract) in [
            (REVIEW.to_string(), true),
            (REVIEW.replace("## Abstract", "## Summary"), false),
        ] {
            let (mut c, p, _) = store_with(&md);
            let stop = AtomicBool::new(false);
            let model = FakeModel::default();
            let mut chat = FakeChat::new(|_, _| suitable(false));
            let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
            let id = run_prisma(&mut ctx, &p, false, &stop).unwrap();
            assert!(chat.sent[0].contains(CHECK_PROMPT));
            assert!(chat.sent[0].contains("We systematically reviewed"));
            assert_eq!(
                chat.sent[0].contains("Twelve trials"),
                !from_abstract,
                "{md}"
            );
            let check = stored(&mut c, id).check.unwrap();
            assert_eq!(check.from_abstract, from_abstract, "{md}");
        }
    }

    /// The reply room sets the split: the run's parts, and the calls, are
    /// `plan_parts`' for it.
    #[test]
    fn plan_follows_reply_budget() {
        let mut counts = Vec::new();
        for reply in [MIN_REPLY_TOKENS, MAX_REPLY_TOKENS] {
            let (mut c, p, _) = store_with(REVIEW);
            let stop = AtomicBool::new(false);
            let model = FakeModel::default();
            let mut chat = FakeChat::answering();
            let mut ctx = context(&mut c, &mut chat, &model, WIDE, reply);
            let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
            let plan = ids_of(&plan_for(reply));
            assert_eq!(chat.parts_asked(), plan);
            let run = stored(&mut c, id);
            let parts: Vec<Vec<String>> = run.parts.iter().map(|p| p.item_ids.clone()).collect();
            let want: Vec<Vec<String>> = plan
                .iter()
                .map(|ids| ids.iter().map(|s| s.to_string()).collect())
                .collect();
            assert_eq!(parts, want);
            assert!(run.parts.iter().all(|p| p.kind == KIND_PLAN));
            counts.push(plan.len());
        }
        assert!(counts[0] > 1, "{counts:?}");
        assert_eq!(counts[1], 1);
    }

    /// A reply room too small for one item is refused before any call, with
    /// nothing stored.
    #[test]
    fn item_too_large_is_reply_too_small() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::answering();
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, 100);
        let r = run_prisma(&mut ctx, &p, false, &stop);
        assert!(
            matches!(&r, Err(PrismaRunError::ReplyTooSmall { item }) if item == "1"),
            "{r:?}"
        );
        assert!(chat.sent.is_empty());
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// A window too small for the paper is refused before any call.
    #[test]
    fn too_long_window_is_refused() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::answering();
        let window = Some(MIN_REPLY_TOKENS);
        let mut ctx = context(&mut c, &mut chat, &model, window, MIN_REPLY_TOKENS);
        let r = run_prisma(&mut ctx, &p, false, &stop);
        assert!(
            matches!(r, Err(PrismaRunError::TooLong { window: w }) if w == window),
            "{r:?}"
        );
        assert!(chat.sent.is_empty());
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// Stopped during part 3 of 4 (its call fails as it stops): parts 1–2
    /// are done, 3–4 pending, the run incomplete, the result `Cancelled`.
    #[test]
    fn stop_keeps_finished_parts() {
        assert_eq!(plan_for(MIN_REPLY_TOKENS).len(), 4);
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|n, ids| {
            if n == 2 {
                transport()
            } else {
                answer(ids, |_| "reported")
            }
        })
        .stopping(2, &stop);
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let r = run_prisma(&mut ctx, &p, true, &stop);
        assert!(matches!(r, Err(PrismaRunError::Cancelled)), "{r:?}");
        assert_eq!(chat.sent.len(), 3, "not retried once stopped");
        let id = latest(&mut c, &p, cid);
        let run = stored(&mut c, id);
        assert_eq!(
            statuses(&run),
            [PART_DONE, PART_DONE, PART_PENDING, PART_PENDING]
        );
        assert_eq!(run.status, RUN_INCOMPLETE);
    }

    /// A reply that arrives as Stop is pressed is kept, then the run stops
    /// (spec §7).
    #[test]
    fn reply_during_stop_is_kept() {
        let plan = plan_for(MIN_REPLY_TOKENS);
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::answering().stopping(1, &stop);
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let r = run_prisma(&mut ctx, &p, true, &stop);
        assert!(matches!(r, Err(PrismaRunError::Cancelled)), "{r:?}");
        assert_eq!(chat.sent.len(), 2);
        let id = latest(&mut c, &p, cid);
        let run = stored(&mut c, id);
        assert_eq!(
            statuses(&run),
            [PART_DONE, PART_DONE, PART_PENDING, PART_PENDING]
        );
        assert_eq!(
            run.items.len(),
            plan[0].item_ids.len() + plan[1].item_ids.len()
        );
    }

    /// A failure that outlives the retries fails its part, keeps the earlier
    /// ones, and leaves the run incomplete.
    #[test]
    fn failure_keeps_finished_parts() {
        let plan = plan_for(MIN_REPLY_TOKENS);
        let second = plan[1].item_ids.clone();
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(move |_, ids| {
            if ids == second.as_slice() {
                transport()
            } else {
                answer(ids, |_| "reported")
            }
        });
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let r = run_prisma(&mut ctx, &p, true, &stop);
        assert!(
            matches!(
                r,
                Err(PrismaRunError::Prisma(PrismaError::RequestFailed { .. }))
            ),
            "{r:?}"
        );
        assert_eq!(chat.sent.len(), 1 + RETRIEVAL_ATTEMPTS as usize);
        let id = latest(&mut c, &p, cid);
        let run = stored(&mut c, id);
        assert_eq!(
            statuses(&run),
            [PART_DONE, PART_FAILED, PART_PENDING, PART_PENDING]
        );
        assert_eq!(run.parts[1].error_kind.as_deref(), Some("request_failed"));
        assert_eq!(run.status, RUN_INCOMPLETE);
    }

    /// A failed request is retried, and the run completes.
    #[test]
    fn request_failed_is_retried() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|n, ids| {
            if n == 0 {
                transport()
            } else {
                answer(ids, |_| "reported")
            }
        });
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        assert_eq!(chat.sent.len(), plan_for(MIN_REPLY_TOKENS).len() + 1);
        assert_eq!(chat.asked[0], chat.asked[1]);
        assert_eq!(stored(&mut c, id).status, RUN_COMPLETE);
    }

    /// A call that fails as Stop is pressed is not retried, and its part
    /// stays pending, not failed.
    #[test]
    fn stopped_is_not_retried() {
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|_, _| transport()).stopping(0, &stop);
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let r = run_prisma(&mut ctx, &p, true, &stop);
        assert!(matches!(r, Err(PrismaRunError::Cancelled)), "{r:?}");
        assert_eq!(chat.sent.len(), 1);
        let id = latest(&mut c, &p, cid);
        let run = stored(&mut c, id);
        assert!(run.parts.iter().all(|p| p.status == PART_PENDING));
    }

    /// Continue runs only the parts left, never the check again, and
    /// completes the run; a complete run cannot be continued.
    #[test]
    fn continue_runs_only_pending() {
        let plan = ids_of(&plan_for(MIN_REPLY_TOKENS));
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::answering().stopping(1, &stop);
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let r = run_prisma(&mut ctx, &p, false, &stop);
        assert!(matches!(r, Err(PrismaRunError::Cancelled)), "{r:?}");
        let id = latest(&mut c, &p, cid);

        let go_on = AtomicBool::new(false);
        let mut chat = FakeChat::answering();
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        assert_eq!(continue_prisma(&mut ctx, id, &go_on).unwrap(), id);
        assert_eq!(chat.parts_asked(), plan[1..]);
        assert!(!chat.sent.iter().any(|s| s.contains(CHECK_PROMPT)));
        let run = stored(&mut c, id);
        assert_eq!(run.status, RUN_COMPLETE);
        assert!(run.check.is_some());
        assert_eq!(run.items.len(), CHECKLIST.len());

        let mut chat = FakeChat::answering();
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let r = continue_prisma(&mut ctx, id, &go_on);
        assert!(matches!(r, Err(PrismaRunError::Internal(_))), "{r:?}");
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let r = continue_prisma(&mut ctx, id + 1, &go_on);
        assert!(
            matches!(r, Err(PrismaRunError::Store(StoreError::NotFound(_)))),
            "{r:?}"
        );
        assert!(chat.sent.is_empty());
    }

    /// Continue runs on the run's own text though a newer one exists: the
    /// paper it sends is that text, and every quote is located in it.
    #[test]
    fn continue_uses_run_text() {
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::answering().stopping(0, &stop);
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let r = run_prisma(&mut ctx, &p, true, &stop);
        assert!(matches!(r, Err(PrismaRunError::Cancelled)), "{r:?}");
        let id = latest(&mut c, &p, cid);
        let newer = REVIEW.replace("## Methods\n\n", "## Methods\n\nA newer preface here.\n\n");
        let newer_id = add_text(&mut c, &p, &newer);
        assert_ne!(newer_id, cid);

        let go_on = AtomicBool::new(false);
        let mut chat = FakeChat::answering();
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        continue_prisma(&mut ctx, id, &go_on).unwrap();
        assert!(!chat.sent.is_empty());
        assert!(!chat.sent.iter().any(|s| s.contains("A newer preface")));
        let run = stored(&mut c, id);
        assert_eq!(run.canonical_id, cid);
        assert_eq!(run.status, RUN_COMPLETE);
        for i in &run.items {
            assert_eq!(
                i.assessment.citations[0].text, SEARCHED,
                "{}",
                i.assessment.id
            );
        }
    }

    /// A run that failed at part 2 of 3 runs parts 2 and 3 on Continue and
    /// completes.
    #[test]
    fn continue_retries_failed_part() {
        let reply = reply_for_parts(3);
        let plan = ids_of(&plan_for(reply));
        let second = plan[1].clone();
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(move |_, ids| {
            if ids == second.as_slice() {
                refused()
            } else {
                answer(ids, |_| "reported")
            }
        });
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, reply);
        let r = run_prisma(&mut ctx, &p, true, &stop);
        assert!(matches!(r, Err(PrismaRunError::Prisma(_))), "{r:?}");
        assert_eq!(chat.sent.len(), 2, "a refusal is not retried");
        let id = latest(&mut c, &p, cid);
        assert_eq!(
            statuses(&stored(&mut c, id)),
            [PART_DONE, PART_FAILED, PART_PENDING]
        );

        let mut chat = FakeChat::answering();
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, reply);
        continue_prisma(&mut ctx, id, &stop).unwrap();
        assert_eq!(chat.parts_asked(), plan[1..]);
        let run = stored(&mut c, id);
        assert_eq!(run.status, RUN_COMPLETE);
        assert!(run.parts.iter().all(|p| p.error_kind.is_none()));
    }

    /// A part whose reply is cut off is halved (the first half the larger),
    /// and each half is asked.
    #[test]
    fn truncated_part_is_halved() {
        let all = ids_of(&plan_for(MAX_REPLY_TOKENS)).remove(0);
        assert_eq!(all.len(), CHECKLIST.len());
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|n, ids| {
            if n == 0 {
                truncated()
            } else {
                answer(ids, |_| "reported")
            }
        });
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MAX_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        let half = all.len().div_ceil(2);
        assert_eq!(
            chat.parts_asked(),
            vec![all.clone(), all[..half].to_vec(), all[half..].to_vec()]
        );
        let run = stored(&mut c, id);
        let shape: Vec<(&str, &str)> = run
            .parts
            .iter()
            .map(|p| (p.kind.as_str(), p.status.as_str()))
            .collect();
        assert_eq!(
            shape,
            [
                (KIND_PLAN, PART_SPLIT),
                (KIND_HALF, PART_DONE),
                (KIND_HALF, PART_DONE)
            ]
        );
        assert_eq!(run.status, RUN_COMPLETE);
        assert_eq!(run.items.len(), CHECKLIST.len());
    }

    /// A single item cut off fails its part with PICO's wording, and the call
    /// ends.
    #[test]
    fn single_item_truncated_fails() {
        let reply = 600;
        assert!(plan_for(reply).iter().all(|p| p.item_ids.len() == 1));
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|_, _| truncated());
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, reply);
        let r = run_prisma(&mut ctx, &p, true, &stop);
        assert!(
            matches!(r, Err(PrismaRunError::Prisma(PrismaError::Truncated(_)))),
            "{r:?}"
        );
        assert_eq!(chat.sent.len(), 1);
        let id = latest(&mut c, &p, cid);
        let run = stored(&mut c, id);
        let failed = &run.parts[0];
        assert_eq!(failed.status, PART_FAILED);
        assert_eq!(failed.error_kind.as_deref(), Some("truncated"));
        let message = failed.error_message.as_deref().unwrap();
        assert!(
            message.contains("reply reached its length limit"),
            "{message}"
        );
        assert!(run.parts[1..].iter().all(|p| p.status == PART_PENDING));
    }

    /// An item a reply leaves out is asked again once, as a `missing` part
    /// of just that item, and the run completes.
    #[test]
    fn missing_item_reasked_once() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|_, ids| {
            if ids == ["3"] {
                answer(ids, |_| "reported")
            } else {
                answer(&without(ids, "3"), |_| "reported")
            }
        });
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        assert_eq!(chat.parts_asked().last().unwrap(), &["3"]);
        assert_eq!(chat.sent.len(), plan_for(MIN_REPLY_TOKENS).len() + 1);
        let run = stored(&mut c, id);
        let last = run.parts.last().unwrap();
        assert_eq!(
            (last.kind.as_str(), last.status.as_str()),
            (KIND_MISSING, PART_DONE)
        );
        assert_eq!(last.item_ids, ["3"]);
        assert_eq!(run.status, RUN_COMPLETE);
        assert_eq!(run.items.len(), CHECKLIST.len());
    }

    /// An item still left out fails its follow-up, naming it, without ending
    /// the call: the rest run, and the run comes back incomplete.
    #[test]
    fn still_missing_fails_part() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|_, ids| answer(&without(ids, "3"), |_| "reported"));
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        assert_eq!(chat.sent.len(), plan_for(MIN_REPLY_TOKENS).len() + 1);
        let run = stored(&mut c, id);
        assert_eq!(run.status, RUN_INCOMPLETE);
        let last = run.parts.last().unwrap();
        assert_eq!(
            (last.kind.as_str(), last.status.as_str()),
            (KIND_MISSING, PART_FAILED)
        );
        assert_eq!(last.item_ids, ["3"]);
        assert_eq!(last.error_kind.as_deref(), Some(ERROR_MISSING_ITEMS));
        let message = last.error_message.as_deref().unwrap();
        assert!(message.contains("item(s) 3."), "{message}");
        assert_eq!(run.items.len(), CHECKLIST.len() - 1);
        let before_last = &run.parts[..run.parts.len() - 1];
        assert!(before_last.iter().all(|p| p.status == PART_DONE));
    }

    /// Runs [`long_review`] condensed and forced, item 7 judged as
    /// `first` in its part and as `recheck` when asked alone; returns the
    /// store, the run, the chat's asks and the condenser's prompts.
    fn condensed_run(
        first: &'static str,
        recheck: &'static str,
        events: Option<Arc<Mutex<Vec<PrismaEvent>>>>,
    ) -> (StoredPrismaRun, Vec<Vec<&'static str>>, Vec<String>) {
        let (mut c, p, _) = store_with(&long_review(600));
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(move |_, ids| {
            if ids == ["7"] {
                answer(ids, |_| recheck)
            } else {
                answer(ids, |id| if id == "7" { first } else { "reported" })
            }
        });
        let window = tight_window(MIN_REPLY_TOKENS);
        let mut ctx = context(&mut c, &mut chat, &model, window, MIN_REPLY_TOKENS);
        if let Some(events) = events {
            ctx.on_event = Arc::new(move |e| events.lock().unwrap().push(e));
        }
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        assert!(chat.sent.iter().all(|s| s.contains(DIGEST)));
        let asked = chat.parts_asked();
        (stored(&mut c, id), asked, model.prompts.take())
    }

    /// A condensed part that judges key item 7 `not_reported` is followed
    /// by one `recheck` part for 7, condensed for 7 alone; a gap it confirms
    /// stands, its first judgement kept.
    #[test]
    fn condensed_key_gap_is_rechecked() {
        let (run, asked, prompts) = condensed_run("not_reported", "not_reported", None);
        let rechecks: Vec<_> = run
            .parts
            .iter()
            .filter(|p| p.kind == KIND_RECHECK)
            .collect();
        assert_eq!(rechecks.len(), 1);
        assert_eq!(rechecks[0].item_ids, ["7"]);
        assert_eq!(rechecks[0].status, PART_DONE);
        assert_eq!(asked.last().unwrap(), &["7"]);
        let query = part_query(&[item("7").unwrap()]);
        assert!(prompts
            .iter()
            .any(|p| p.contains(&format!("Needed: {query}\n"))));
        let seven = shown(&run, "7");
        assert_eq!(seven.assessment.judgement, Judgement::NotReported);
        assert_eq!(seven.first_judgement, Some(Judgement::NotReported));
        assert_eq!(seven.recheck_part_id, Some(rechecks[0].id));
        assert_eq!(run.status, RUN_COMPLETE);
    }

    /// A re-check that finds the item shows its answer, and the first
    /// judgement is kept beside it.
    #[test]
    fn recheck_keeps_first_judgement() {
        let (run, _, _) = condensed_run("not_reported", "reported", None);
        let seven = shown(&run, "7");
        assert_eq!(seven.assessment.judgement, Judgement::Reported);
        assert_eq!(seven.assessment.citations[0].text, SEARCHED);
        assert_eq!(seven.first_judgement, Some(Judgement::NotReported));
        assert!(seven.condensed);
    }

    /// Items judged from a condensation are stored as such; a non-key gap is
    /// not re-checked.
    #[test]
    fn non_key_condensed_is_noted() {
        let (run, asked, _) = condensed_run("reported", "reported", None);
        assert!(run.parts.iter().all(|p| p.kind == KIND_PLAN));
        assert_eq!(asked.len(), plan_for(MIN_REPLY_TOKENS).len());
        assert_eq!(run.items.len(), CHECKLIST.len());
        assert!(run.items.iter().all(|i| i.condensed));
        assert!(run.parts.iter().all(|p| p.condensed_from_chars.is_some()));
        // A non-key item, judged not reported from a condensation, stands.
        let (mut c, p, _) = store_with(&long_review(600));
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|_, ids| {
            answer(ids, |id| {
                if id == "8" {
                    "not_reported"
                } else {
                    "reported"
                }
            })
        });
        let window = tight_window(MIN_REPLY_TOKENS);
        let mut ctx = context(&mut c, &mut chat, &model, window, MIN_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        let run = stored(&mut c, id);
        assert!(run.parts.iter().all(|p| p.kind == KIND_PLAN));
        let eight = shown(&run, "8");
        assert_eq!(eight.assessment.judgement, Judgement::NotReported);
        assert!(eight.condensed && eight.first_judgement.is_none());
    }

    /// A paper sent whole is never condensed, and a key gap is not
    /// re-checked.
    #[test]
    fn whole_paper_needs_no_recheck() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::new(|_, ids| {
            answer(ids, |id| {
                if id == "7" {
                    "not_reported"
                } else {
                    "reported"
                }
            })
        });
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        assert!(model.prompts.borrow().is_empty());
        let run = stored(&mut c, id);
        assert!(run.parts.iter().all(|p| p.kind == KIND_PLAN));
        assert!(run.items.iter().all(|i| !i.condensed));
        assert_eq!(
            shown(&run, "7").assessment.judgement,
            Judgement::NotReported
        );
        assert_eq!(run.status, RUN_COMPLETE);
    }

    /// Each part is announced in order, with its label and the parts known
    /// so far, the re-check's count added once it exists; `Rechecking`
    /// comes just before the re-check's part, and condensation progress is
    /// reported.
    #[test]
    fn events_name_each_part() {
        let events = Arc::new(Mutex::new(Vec::new()));
        condensed_run("not_reported", "reported", Some(Arc::clone(&events)));
        let events = events.lock().unwrap().clone();
        let plan = plan_for(MIN_REPLY_TOKENS);
        let n = plan.len();
        let k = plan.iter().position(|p| p.item_ids.contains(&"7")).unwrap();
        let mut want: Vec<PrismaEvent> = plan
            .iter()
            .enumerate()
            .map(|(i, part)| PrismaEvent::Part {
                index: i + 1,
                of: if i <= k { n } else { n + 1 },
                label: part.label(),
            })
            .collect();
        want.push(PrismaEvent::Part {
            index: n + 1,
            of: n + 1,
            label: "Methods 7".into(),
        });
        let parts: Vec<PrismaEvent> = events
            .iter()
            .filter(|e| matches!(e, PrismaEvent::Part { .. }))
            .cloned()
            .collect();
        assert_eq!(parts, want);
        let at = events
            .iter()
            .position(|e| matches!(e, PrismaEvent::Rechecking { .. }))
            .unwrap();
        assert_eq!(events[at], PrismaEvent::Rechecking { item: "7".into() });
        assert_eq!(events[at + 1], want[n]);
        assert!(events
            .iter()
            .any(|e| matches!(e, PrismaEvent::Condensing { .. })));
        assert!(events.contains(&PrismaEvent::Condensed));
    }

    /// The budget is PICO's arithmetic with the plan's longest part prompt:
    /// longer parts leave less paper; too small a window gives none.
    #[test]
    fn budget_uses_longest_part_prompt() {
        let plan = plan_for(MIN_REPLY_TOKENS);
        let longest = plan
            .iter()
            .map(|p| part_prompt_chars(&p.item_ids))
            .max()
            .unwrap();
        let window = Some(100_000);
        assert_eq!(
            budget_chars(window, MIN_REPLY_TOKENS, TITLE, &plan),
            paper_budget_chars(
                window,
                MIN_REPLY_TOKENS,
                longest,
                TITLE,
                PRISMA_MIN_BUDGET_CHARS
            )
        );
        let whole = plan_for(MAX_REPLY_TOKENS);
        assert!(
            budget_chars(window, MIN_REPLY_TOKENS, TITLE, &whole).unwrap()
                < budget_chars(window, MIN_REPLY_TOKENS, TITLE, &plan).unwrap()
        );
        assert_eq!(
            budget_chars(Some(MIN_REPLY_TOKENS), MIN_REPLY_TOKENS, TITLE, &plan),
            None
        );
        let w = tight_window(MIN_REPLY_TOKENS).unwrap();
        let budget = budget_chars(Some(w), MIN_REPLY_TOKENS, TITLE, &plan).unwrap();
        assert!(budget >= PRISMA_MIN_BUDGET_CHARS);
        assert_eq!(
            budget_chars(Some(w - 64), MIN_REPLY_TOKENS, TITLE, &plan),
            None
        );
    }

    /// When the check's prompt is longer than every part's (here a plan of
    /// item 26 alone, the shortest part prompt), the budget leaves room for
    /// the check's: it is the budget for that prompt, below what the part
    /// alone would allow. Otherwise the part's longest prompt sets it.
    #[test]
    fn budget_covers_the_check() {
        let short = [PartPlan {
            item_ids: vec!["26"],
        }];
        let longest = part_prompt_chars(&["26"]);
        assert!(longest < check_prompt_chars(), "{longest}");
        let window = Some(100_000);
        let budget = budget_chars(window, MIN_REPLY_TOKENS, TITLE, &short).unwrap();
        let for_check = paper_budget_chars(
            window,
            MIN_REPLY_TOKENS,
            check_prompt_chars(),
            TITLE,
            PRISMA_MIN_BUDGET_CHARS,
        );
        assert_eq!(Some(budget), for_check);
        assert!(budget < parts_budget_chars(window, MIN_REPLY_TOKENS, TITLE, &short).unwrap());
        let wrapped = format!(
            "{SUITABILITY_TASK_PROMPT}\n\nPaper Title: \n\nPaper Text:\n\n\n{SUITABILITY_RESPONSE_FORMAT}"
        );
        assert!(
            check_prompt_chars() >= PRISMA_SYSTEM_PROMPT.chars().count() + wrapped.chars().count()
        );
        // A plan with a longer part prompt is sized by it, as before.
        let plan = plan_for(MIN_REPLY_TOKENS);
        assert_eq!(
            budget_chars(window, MIN_REPLY_TOKENS, TITLE, &plan),
            parts_budget_chars(window, MIN_REPLY_TOKENS, TITLE, &plan)
        );
    }

    /// A `missing` part of two items cut off is halved; a half that still
    /// leaves its item out fails at once, never asked a second time.
    #[test]
    fn halved_reask_is_not_reasked() {
        let (mut c, p, _) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        // Items 3 and 4 are left out, their re-ask is cut off, and 3 is left
        // out again by its half.
        let mut chat = FakeChat::new(|_, ids| match ids {
            ["3", "4"] => truncated(),
            ["4"] => answer(ids, |_| "reported"),
            _ => answer(&without(&without(ids, "3"), "4"), |_| "reported"),
        });
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MIN_REPLY_TOKENS);
        let id = run_prisma(&mut ctx, &p, true, &stop).unwrap();
        let n = plan_for(MIN_REPLY_TOKENS).len();
        let asked = chat.parts_asked();
        assert_eq!(asked[n..], [vec!["3", "4"], vec!["3"], vec!["4"]]);
        let run = stored(&mut c, id);
        assert_eq!(run.status, RUN_INCOMPLETE);
        let shape: Vec<(&str, &str, Vec<String>)> = run.parts[n..]
            .iter()
            .map(|p| (p.kind.as_str(), p.status.as_str(), p.item_ids.clone()))
            .collect();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            shape,
            [
                (KIND_MISSING, PART_SPLIT, ids(&["3", "4"])),
                (KIND_MISSING, PART_DONE, ids(&["3"])),
                (KIND_MISSING, PART_DONE, ids(&["4"])),
                (KIND_MISSING, PART_FAILED, ids(&["3"])),
            ]
        );
        assert_eq!(
            run.parts.last().unwrap().error_kind.as_deref(),
            Some(ERROR_MISSING_ITEMS)
        );
    }

    /// A stop during the last part keeps its reply and finishes the run:
    /// complete, though the call reports the stop.
    #[test]
    fn stop_on_last_part_finishes_run() {
        assert_eq!(plan_for(MAX_REPLY_TOKENS).len(), 1);
        let (mut c, p, cid) = store_with(REVIEW);
        let stop = AtomicBool::new(false);
        let model = FakeModel::default();
        let mut chat = FakeChat::answering().stopping(0, &stop);
        let mut ctx = context(&mut c, &mut chat, &model, WIDE, MAX_REPLY_TOKENS);
        let r = run_prisma(&mut ctx, &p, true, &stop);
        assert!(matches!(r, Err(PrismaRunError::Cancelled)), "{r:?}");
        let id = latest(&mut c, &p, cid);
        let run = stored(&mut c, id);
        assert_eq!(run.status, RUN_COMPLETE);
        assert_eq!(run.items.len(), CHECKLIST.len());
    }

    /// The errors read as the tab shows them, and the events serialize as
    /// the frontend reads them.
    #[test]
    fn messages_and_events_read_as_shown() {
        let small = PrismaRunError::ReplyTooSmall { item: "1".into() };
        assert_eq!(
            small.to_string(),
            "The model's reply room is too small for a PRISMA item; choose another model."
        );
        let cut = PrismaRunError::Prisma(PrismaError::Truncated("x".into())).to_string();
        assert!(cut.contains("reply reached its length limit") && cut.contains("(x)"));
        assert!(PrismaRunError::TooLong { window: Some(2048) }
            .to_string()
            .contains("2048"));
        assert_eq!(
            serde_json::to_value(PrismaEvent::Part {
                index: 1,
                of: 2,
                label: "Methods 5–13c".into()
            })
            .unwrap(),
            json!({"kind": "part", "index": 1, "of": 2, "label": "Methods 5–13c"})
        );
        assert_eq!(
            serde_json::to_value(PrismaEvent::Rechecking { item: "7".into() }).unwrap(),
            json!({"kind": "rechecking", "item": "7"})
        );
        assert_eq!(
            PrismaEvent::from(PicoEvent::Condensing {
                batch: 1,
                batches: 2,
                level: 0
            }),
            PrismaEvent::Condensing {
                batch: 1,
                batches: 2,
                level: 0
            }
        );
    }
}
