//! Running PICO on a paper: the suitability check, then the extraction, within
//! the model's window. bmlib's [`PicoAgent`] asks, checks the quotes and
//! condenses a long paper; this sizes its budget, retries, honours a stop, and
//! stores the run ([`crate::pico_store`]).

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bmlib::context_processor::llm_processor::ContextModel;
use bmlib::context_processor::{ProcessingConfig, ProgressInfo};
use bmlib::db::Db;
use bmlib::fulltext::canonical::jats::ABSTRACT_HEADING;
use bmlib::fulltext::canonical::CanonicalText;
use bmlib::llm::LlmClient;
use bmlib::quality::{
    JsonChat, LlmChat, LlmContextModel, PicoAgent, PicoError, EXTRACTION_RESPONSE_FORMAT,
    EXTRACTION_TASK_PROMPT, PICO_CONDENSE_CONSOLIDATION_PROMPT, PICO_CONDENSE_EXTRACTION_PROMPT,
    PICO_CONDENSE_QUERY, PICO_SYSTEM_PROMPT, SUITABILITY_RESPONSE_FORMAT, SUITABILITY_TASK_PROMPT,
};

use crate::config::{
    FIT_CHARS_PER_TOKEN, MIN_QUOTE_SIMILARITY, PICO_MIN_BUDGET_CHARS, PICO_TEMPERATURE,
    RETRIEVAL_ATTEMPTS, UNKNOWN_WINDOW_FIT_TOKENS,
};
use crate::interrogate::{full_text, num_ctx, AskError};
use crate::pico_store::{save_run, NewPicoRun, PicoCheck, PicoOutcome};
use crate::prompt::estimated_tokens;
use crate::retry::with_backoff;
use crate::store::{PaperId, StoreError};
use serde::Serialize;

/// What a PICO run works with.
pub struct PicoContext<'a> {
    /// The store.
    pub db: &'a mut dyn Db,
    /// The model, for the check and the extraction.
    pub chat: &'a mut dyn JsonChat,
    /// The model, for condensing a long paper.
    pub condenser_model: &'a dyn ContextModel,
    /// The chosen `provider:model`, stored with the run.
    pub model: String,
    /// The model's window in tokens, `None` when unknown.
    pub window: Option<u64>,
    /// The room for each reply in tokens: every call's `max_tokens`, and what
    /// the paper's budget keeps free (`prompt::reply_tokens`).
    pub reply_tokens: u64,
    /// The first backoff wait between attempts (the command passes
    /// `RETRY_BASE_DELAY`, tests zero).
    pub retry_delay: Duration,
    /// Told how far the condensation of a long paper has got (bmlib #609).
    pub on_event: Arc<dyn Fn(PicoEvent) + Send + Sync>,
}

/// What the user sees while a PICO run condenses a long paper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PicoEvent {
    /// Part `batch` of `batches` is being condensed. At `level` 0 the parts
    /// are the paper's; above, they are the condensed parts, condensed again
    /// because together they still did not fit.
    Condensing {
        /// The part being condensed, from 1.
        batch: usize,
        /// The parts at this level.
        batches: usize,
        /// How many times the parts have been condensed already.
        level: usize,
    },
    /// The condensation has ended. bmlib reports this whether it succeeded or
    /// failed; the run's result says which.
    Condensed,
    /// A stage this version does not know (bmlib added it later), shown as
    /// bmlib words it so the mismatch is seen, not hidden.
    Other {
        /// bmlib's name for the stage.
        stage: String,
        /// bmlib's description of it.
        message: String,
    },
}

impl PicoEvent {
    /// The event a progress report of bmlib's condenser shows: one per part
    /// sent, and one when it is done. The other known stages say nothing new
    /// (none for its start, its batching or a new level: that level's first
    /// part, if it has any, follows at once). A stage bmlib adds later is
    /// logged and shown as [`PicoEvent::Other`].
    pub fn from_progress(info: &ProgressInfo) -> Option<Self> {
        match info.stage.as_str() {
            "extracting" => Some(PicoEvent::Condensing {
                batch: info.current_batch,
                batches: info.total_batches,
                level: info.recursion_level,
            }),
            "complete" => Some(PicoEvent::Condensed),
            "starting" | "batching" | "recursing" => None,
            other => {
                log::warn!("PICO condensation reported an unknown stage {other:?}");
                Some(PicoEvent::Other {
                    stage: other.to_string(),
                    message: info.message.clone(),
                })
            }
        }
    }
}

/// Why a run stored nothing.
#[derive(Debug)]
pub enum PicoRunError {
    /// The paper has no canonical text.
    NoFullText,
    /// The text neither passed the full-text check nor was confirmed.
    NotFullText,
    /// The window leaves no room for the paper; `window` is the reported one,
    /// `None` when it was assumed ([`UNKNOWN_WINDOW_FIT_TOKENS`]).
    TooLong {
        /// The reported window, or `None` when assumed.
        window: Option<u64>,
    },
    /// The user stopped the run.
    Cancelled,
    /// bmlib's agent failed.
    Pico(PicoError),
    /// The store failed.
    Store(StoreError),
    /// A defect in this code or bmlib's prompt constants.
    Internal(String),
}

impl fmt::Display for PicoRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PicoRunError::NoFullText => write!(f, "This paper has no full text."),
            PicoRunError::NotFullText => write!(
                f,
                "This text did not pass the full-text check. Confirm it is the whole paper first."
            ),
            PicoRunError::TooLong { window: Some(w) } => write!(
                f,
                "The model's context window ({w} tokens) leaves no room for the paper. \
                 Choose a model with a larger window, or raise the context-window setting \
                 if it is lower than the model's."
            ),
            PicoRunError::TooLong { window: None } => write!(
                f,
                "The model does not report its context window, and the \
                 {UNKNOWN_WINDOW_FIT_TOKENS} tokens assumed leave no room for the paper. \
                 Set the context-window setting to the model's."
            ),
            PicoRunError::Cancelled => write!(f, "The PICO extraction was stopped."),
            PicoRunError::Pico(PicoError::UnusableReply(msg)) => write!(
                f,
                "The model's reply could not be read ({msg}). Try another model."
            ),
            PicoRunError::Pico(PicoError::Truncated(msg)) => write!(
                f,
                "The model's reply reached its length limit and was cut off ({msg}). \
                 Try another model."
            ),
            PicoRunError::Pico(e) => write!(f, "{e}"),
            PicoRunError::Store(e) => write!(f, "{e}"),
            PicoRunError::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PicoRunError {}

/// The characters of paper a run can send to a model with `window` tokens
/// (`None`: [`UNKNOWN_WINDOW_FIT_TOKENS`]), after `reply` tokens of room for
/// the reply, the longest PICO prompt and the title; `None` when that is under
/// [`PICO_MIN_BUDGET_CHARS`]. Saturating throughout: a window may be any size.
pub fn budget_chars(window: Option<u64>, reply: u64, title: &str) -> Option<usize> {
    let tokens = window
        .unwrap_or(UNKNOWN_WINDOW_FIT_TOKENS)
        .saturating_sub(reply)
        .saturating_sub(estimated_tokens(longest_prompt_chars()))
        .saturating_sub(estimated_tokens(title.chars().count()));
    let chars = usize::try_from(tokens.saturating_mul(FIT_CHARS_PER_TOKEN)).unwrap_or(usize::MAX);
    (chars >= PICO_MIN_BUDGET_CHARS).then_some(chars)
}

/// The characters of the longest prompt a run sends, without the paper and
/// its title: the system prompt and the longest of the check, the extraction
/// (each as bmlib wraps it) and the two condensation prompts.
fn longest_prompt_chars() -> usize {
    let wrapped =
        |task: &str, format: &str| format!("{task}\n\nPaper Title: \n\nPaper Text:\n\n\n{format}");
    let condense = |template: &str| {
        template
            .replace("{query}", PICO_CONDENSE_QUERY)
            .replace("{content}", "")
    };
    let longest = [
        wrapped(SUITABILITY_TASK_PROMPT, SUITABILITY_RESPONSE_FORMAT),
        wrapped(EXTRACTION_TASK_PROMPT, EXTRACTION_RESPONSE_FORMAT),
        condense(PICO_CONDENSE_EXTRACTION_PROMPT),
        condense(PICO_CONDENSE_CONSOLIDATION_PROMPT),
    ]
    .iter()
    .map(|p| p.chars().count())
    .max()
    .unwrap_or(0);
    PICO_SYSTEM_PROMPT.chars().count() + longest
}

/// The paper's abstract: the blocks of the first section headed `Abstract`
/// (in any case, trimmed) and of every deeper section after it, joined by
/// blank lines. `None` when there is none or it is blank.
pub fn abstract_text(canonical: &CanonicalText) -> Option<String> {
    let heading = |i: usize| {
        canonical.sections[i]
            .heading_span
            .and_then(|h| canonical.span_text(h))
    };
    let start = (0..canonical.sections.len())
        .find(|&i| heading(i).is_some_and(|h| h.trim().eq_ignore_ascii_case(ABSTRACT_HEADING)))?;
    let level = canonical.sections[start].level;
    let blocks: Vec<&str> = std::iter::once(&canonical.sections[start])
        .chain(
            canonical.sections[start + 1..]
                .iter()
                .take_while(|s| s.level > level),
        )
        .flat_map(|s| &s.blocks)
        .filter_map(|b| canonical.span_text(b.span))
        .collect();
    let text = blocks.join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

/// The chat a run asks through: `model` on `client`, sending the window as
/// `num_ctx` when known, and sending nothing once `stop` is set.
pub fn pico_chat<'a>(
    client: &'a LlmClient,
    model: &str,
    window: Option<u64>,
    stop: &'a AtomicBool,
) -> LlmChat<'a> {
    LlmChat::new(client, model)
        .with_num_ctx(num_ctx(window))
        .with_stop(stop)
}

/// The model a run condenses a long paper with, set up as [`pico_chat`].
pub fn pico_condenser_model<'a>(
    client: &'a LlmClient,
    model: &str,
    window: Option<u64>,
    stop: &'a AtomicBool,
) -> LlmContextModel<'a> {
    LlmContextModel::new(client, model)
        .with_num_ctx(num_ctx(window))
        .with_stop(stop)
}

/// Runs PICO on `paper`'s latest text and stores the run; returns its id.
/// `force` skips the suitability check. Nothing is stored on any failure, nor
/// once `stop` is set, even after a call that succeeded.
pub fn run_pico(
    ctx: &mut PicoContext,
    paper: &PaperId,
    force: bool,
    stop: &AtomicBool,
) -> Result<i64, PicoRunError> {
    let result = run_inner(ctx, paper, force, stop);
    if let Err(e) = &result {
        match e {
            PicoRunError::Cancelled => log::info!("paper {}: PICO run stopped", paper.as_str()),
            PicoRunError::Store(_) | PicoRunError::Internal(_) => {
                log::error!("paper {}: PICO run failed: {e}", paper.as_str())
            }
            _ => log::warn!("paper {}: PICO run failed: {e}", paper.as_str()),
        }
    }
    result
}

/// [`run_pico`] without the logging.
fn run_inner(
    ctx: &mut PicoContext,
    paper: &PaperId,
    force: bool,
    stop: &AtomicBool,
) -> Result<i64, PicoRunError> {
    let stored = full_text(ctx.db, paper).map_err(|e| match e {
        AskError::NoFullText => PicoRunError::NoFullText,
        AskError::NotFullText => PicoRunError::NotFullText,
        AskError::Store(e) => PicoRunError::Store(e),
        other => PicoRunError::Internal(other.to_string()),
    })?;
    let canonical = &stored.text;
    let title = canonical
        .title_span
        .and_then(|s| canonical.span_text(s))
        .unwrap_or("");
    let budget = budget_chars(ctx.window, ctx.reply_tokens, title)
        .ok_or(PicoRunError::TooLong { window: ctx.window })?;
    let max_tokens = i64::try_from(ctx.reply_tokens).unwrap_or(i64::MAX);
    let on_event = Arc::clone(&ctx.on_event);
    let mut condenser =
        PicoAgent::pico_condenser(ctx.condenser_model, PICO_TEMPERATURE, max_tokens)
            .map_err(PicoRunError::Internal)?
            .with_progress(move |info| {
                if let Some(event) = PicoEvent::from_progress(info) {
                    on_event(event);
                }
            });
    let mut agent = PicoAgent::new(&mut *ctx.chat)
        .with_condenser(&mut condenser)
        .with_temperature(PICO_TEMPERATURE)
        .with_max_tokens(max_tokens)
        // A failed batch fails the condensation: never a run from a partial digest.
        .with_condense_config(
            ProcessingConfig::default()
                .with_max_context_chars(budget)
                .with_continue_on_error(false),
        )
        .with_stop(stop);
    let delay = ctx.retry_delay;
    // Only a request a retry may cure (never a 401 or 404, bmlib #659), and
    // never once stopped.
    let retryable = |e: &PicoError| {
        matches!(
            e,
            PicoError::RequestFailed {
                retryable: true,
                ..
            }
        ) && !is_set(stop)
    };
    let failed = |e: PicoError| {
        if is_set(stop) || matches!(e, PicoError::Stopped(_)) {
            // The run reports only "stopped"; a failure under way still reaches the log.
            if !matches!(e, PicoError::Stopped(None)) {
                log::warn!(
                    "paper {}: PICO call failed as it stopped: {e}",
                    paper.as_str()
                );
            }
            PicoRunError::Cancelled
        } else {
            PicoRunError::Pico(e)
        }
    };
    let mut prepared = None;
    let check = if force {
        None
    } else {
        let (suitability, from_abstract) = match abstract_text(canonical) {
            Some(abstract_) => (
                with_backoff(RETRIEVAL_ATTEMPTS, delay, retryable, || {
                    agent.check_suitability(title, &abstract_)
                })
                .map_err(failed)?,
                true,
            ),
            None => {
                let ready = agent.prepare(title, &canonical.text).map_err(failed)?;
                let suitability = with_backoff(RETRIEVAL_ATTEMPTS, delay, retryable, || {
                    agent.check_suitability_prepared(&ready)
                })
                .map_err(failed)?;
                prepared = Some(ready);
                (suitability, false)
            }
        };
        cancelled(stop)?;
        Some(PicoCheck {
            suitability,
            from_abstract,
        })
    };
    let outcome = match check {
        Some(check) if !check.suitability.is_suitable => PicoOutcome::NotSuitable(check),
        check => {
            let ready = match prepared {
                Some(ready) => ready,
                None => agent.prepare(title, &canonical.text).map_err(failed)?,
            };
            cancelled(stop)?;
            let extraction = with_backoff(RETRIEVAL_ATTEMPTS, delay, retryable, || {
                agent.extract_prepared(&ready, MIN_QUOTE_SIMILARITY)
            })
            .map_err(failed)?;
            PicoOutcome::Extracted { check, extraction }
        }
    };
    // A stop pressed during a call that then succeeded still stores nothing.
    cancelled(stop)?;
    let run = NewPicoRun {
        model: ctx.model.clone(),
        outcome,
    };
    save_run(ctx.db, paper, stored.id, &run).map_err(PicoRunError::Store)
}

/// Whether `stop` is set.
fn is_set(stop: &AtomicBool) -> bool {
    stop.load(Ordering::SeqCst)
}

/// [`PicoRunError::Cancelled`] once `stop` is set.
fn cancelled(stop: &AtomicBool) -> Result<(), PicoRunError> {
    if is_set(stop) {
        Err(PicoRunError::Cancelled)
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
    use crate::pico_store::{latest_run, run as stored_run};
    use crate::store::{
        canonical_by_id, confirm_full_text, create_paper, latest_canonical, open_memory_store,
        save_canonical, save_source, NewSource,
    };
    use bmlib::agents::{ChatJsonError, ChatJsonOutcome};
    use bmlib::fulltext::canonical::{
        assess_full_text, canonical_from_jats, canonical_from_markdown,
    };
    use bmlib::fulltext::jats_reader;
    use bmlib::llm::LLMMessage;
    use bmlib::quality::{PicoPartState, STOPPED_MESSAGE};
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::num::NonZeroU32;
    use std::sync::Arc;

    /// A small trial: an abstract, methods and results.
    const TRIAL: &str = "# Asthma trial\n\n## Abstract\n\nInhaler X was tested in adults with asthma.\n\n## Methods\n\nWe enrolled 105 adults with asthma.\n\n## Results\n\nExacerbations fell by a third.\n";

    /// The quote the extraction cites, as the paper words it.
    const ENROLLED: &str = "We enrolled 105 adults with asthma.";

    /// What the fake condenser model returns for any call.
    const DIGEST: &str = "Adults with asthma took inhaler X.";

    /// A store holding one paper with `canonical` as its text, confirmed or not.
    fn store_with(canonical: &CanonicalText, confirmed: bool) -> (Connection, PaperId) {
        let mut c = open_memory_store().unwrap();
        let ids = Identifiers {
            pmid: Some("1".into()),
            ..Default::default()
        };
        let p = create_paper(&mut c, &ids, Some("Trial")).unwrap();
        let sid = save_source(
            &mut c,
            &p,
            NewSource {
                kind: SourceKind::Markdown,
                origin: "paste".into(),
                file_name: None,
                bytes: canonical.text.as_bytes().to_vec(),
                decoded: None,
            },
        )
        .unwrap();
        let a = assess_full_text(canonical).unwrap();
        let cid = save_canonical(&mut c, &p, sid, canonical, &a).unwrap();
        if confirmed {
            confirm_full_text(&mut c, &p, cid).unwrap();
        }
        (c, p)
    }

    /// The trial with `extra` appended to its results.
    fn trial_with(extra: &str) -> CanonicalText {
        canonical_from_markdown(&format!("{TRIAL}\n{extra}\n"))
    }

    /// `n` sentences of filler.
    fn filler(n: usize) -> String {
        (0..n)
            .map(|i| format!("Filler sentence number {i} says nothing."))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A chat that answers from a script, records each user message, and can
    /// set `stop` during a given call (0-based).
    struct FakeChat<'s> {
        replies: VecDeque<Result<Value, ChatJsonError>>,
        sent: Vec<String>,
        /// Each call's `max_tokens`.
        max_tokens: Vec<i64>,
        stop_on: Option<(usize, &'s AtomicBool)>,
    }

    impl<'s> FakeChat<'s> {
        /// A chat answering `replies` in order.
        fn new(replies: Vec<Result<Value, ChatJsonError>>) -> Self {
            FakeChat {
                replies: replies.into(),
                sent: Vec::new(),
                max_tokens: Vec::new(),
                stop_on: None,
            }
        }

        /// Sets `stop` during call `at`.
        fn stopping(mut self, at: usize, stop: &'s AtomicBool) -> Self {
            self.stop_on = Some((at, stop));
            self
        }
    }

    impl JsonChat for FakeChat<'_> {
        fn chat_json(
            &mut self,
            messages: &[LLMMessage],
            _temperature: f64,
            max_tokens: i64,
            _max_retries: usize,
            _require_dict: bool,
        ) -> Result<ChatJsonOutcome, ChatJsonError> {
            let n = self.sent.len();
            self.max_tokens.push(max_tokens);
            self.sent.push(messages.last().unwrap().content.clone());
            if let Some((at, stop)) = self.stop_on {
                if at == n {
                    stop.store(true, Ordering::SeqCst);
                }
            }
            self.replies
                .pop_front()
                .expect("an unexpected chat call")
                .map(|value| ChatJsonOutcome {
                    value,
                    attempts: 1,
                    repaired: false,
                })
        }
    }

    /// A condenser model returning [`DIGEST`], counting its calls; it can fail
    /// or set `stop` on a given call, and refuses once `stop` is set, as
    /// `LlmContextModel` does.
    struct FakeModel<'s> {
        calls: Cell<usize>,
        /// Each call's `max_tokens`.
        max_tokens: RefCell<Vec<i64>>,
        fail_on: Option<usize>,
        stop_on: Option<usize>,
        stop: &'s AtomicBool,
    }

    impl<'s> FakeModel<'s> {
        /// A model that always answers.
        fn new(stop: &'s AtomicBool) -> Self {
            FakeModel {
                calls: Cell::new(0),
                max_tokens: RefCell::new(Vec::new()),
                fail_on: None,
                stop_on: None,
                stop,
            }
        }
    }

    impl ContextModel for FakeModel<'_> {
        fn complete(&self, _prompt: &str, _t: f64, m: i64) -> Result<String, String> {
            self.max_tokens.borrow_mut().push(m);
            if self.stop.load(Ordering::SeqCst) {
                return Err(STOPPED_MESSAGE.to_string());
            }
            let n = self.calls.get();
            self.calls.set(n + 1);
            if self.stop_on == Some(n) {
                self.stop.store(true, Ordering::SeqCst);
            }
            if self.fail_on == Some(n) {
                return Err("the batch failed".to_string());
            }
            Ok(DIGEST.to_string())
        }

        fn complete_json(&self, _prompt: &str, _t: f64, _m: i64) -> Result<Value, String> {
            Err("not used by the PICO condenser".to_string())
        }
    }

    /// One scripted reply of the fake chat.
    type Reply = Result<Value, ChatJsonError>;

    /// A suitability reply.
    fn suitable(yes: bool) -> Result<Value, ChatJsonError> {
        Ok(json!({
            "is_intervention_study": yes,
            "has_comparison": yes,
            "is_suitable": yes,
            "confidence": 0.9,
            "rationale": "Because.",
            "study_type": "randomized controlled trial"
        }))
    }

    /// An extraction reply whose population quotes `quote`; the other parts
    /// are not reported.
    fn extraction_quoting(quote: &str) -> Result<Value, ChatJsonError> {
        let absent =
            |d: &str| json!({"description": d, "reported": false, "confidence": 0.5, "quotes": []});
        Ok(json!({
            "population": {"description": "adults with asthma", "reported": true, "confidence": 0.9, "quotes": [quote]},
            "intervention": absent("inhaler X"),
            "comparison": absent("none"),
            "outcome": absent("exacerbations"),
            "study_type": "RCT",
            "sample_size": "N=105"
        }))
    }

    /// A transport failure.
    fn transport() -> Result<Value, ChatJsonError> {
        Err(ChatJsonError::Transport {
            message: "connection reset".into(),
            attempts: 1,
            retryable: true,
            status: None,
        })
    }

    /// A refusal no retry cures, such as a 401.
    fn refused() -> Result<Value, ChatJsonError> {
        Err(ChatJsonError::Transport {
            message: "HTTP 401".into(),
            attempts: 1,
            retryable: false,
            status: Some(401),
        })
    }

    /// Runs PICO on `p` with zero retry delay.
    fn go(
        c: &mut Connection,
        p: &PaperId,
        chat: &mut FakeChat,
        model: &FakeModel,
        window: Option<u64>,
        force: bool,
        stop: &AtomicBool,
    ) -> Result<i64, PicoRunError> {
        let mut ctx = PicoContext {
            db: c,
            chat,
            condenser_model: model,
            model: "ollama:m".into(),
            window,
            reply_tokens: MIN_REPLY_TOKENS,
            retry_delay: Duration::ZERO,
            on_event: Arc::new(|_| {}),
        };
        run_pico(&mut ctx, p, force, stop)
    }

    /// The number of stored runs.
    fn runs_stored(c: &mut Connection) -> i64 {
        bmlib::db::fetch_one(c, "SELECT count(*) AS n FROM pico_runs", &[])
            .unwrap()
            .unwrap()
            .get_i64("n")
            .unwrap()
    }

    /// The stored run `id`'s outcome.
    fn outcome(c: &mut Connection, id: i64) -> PicoOutcome {
        stored_run(c, id).unwrap().unwrap().outcome
    }

    /// The budget is the window less the reply, the longest prompt and the
    /// title, in characters; too small a window gives none; no overflow.
    #[test]
    fn budget_from_window() {
        let title = "A title";
        let fixed = |w: u64| {
            (w - MIN_REPLY_TOKENS
                - estimated_tokens(longest_prompt_chars())
                - estimated_tokens(title.chars().count()))
                * FIT_CHARS_PER_TOKEN
        };
        assert_eq!(
            budget_chars(Some(100_000), MIN_REPLY_TOKENS, title),
            Some(fixed(100_000) as usize)
        );
        assert_eq!(
            budget_chars(None, MIN_REPLY_TOKENS, title),
            Some(fixed(UNKNOWN_WINDOW_FIT_TOKENS) as usize)
        );
        assert_eq!(budget_chars(Some(2048), MIN_REPLY_TOKENS, title), None);
        assert_eq!(
            budget_chars(Some(MIN_REPLY_TOKENS), MIN_REPLY_TOKENS, title),
            None
        );
        // A larger reply room leaves that much less paper.
        assert_eq!(
            budget_chars(Some(100_000), 32_768, title),
            Some((fixed(100_000) - (32_768 - MIN_REPLY_TOKENS) * FIT_CHARS_PER_TOKEN) as usize)
        );
        let overhead = MIN_REPLY_TOKENS
            + estimated_tokens(longest_prompt_chars())
            + estimated_tokens(title.chars().count());
        let small = overhead + (PICO_MIN_BUDGET_CHARS as u64 / FIT_CHARS_PER_TOKEN) - 10;
        assert!(fixed(small) > 0);
        assert_eq!(budget_chars(Some(small), MIN_REPLY_TOKENS, title), None);
        assert!(budget_chars(Some(u64::MAX), MIN_REPLY_TOKENS, title).is_some());
    }

    /// The check reads the abstract, never the results, and says so; without
    /// an abstract (or with an empty one) it reads the whole text.
    #[test]
    fn check_reads_the_abstract() {
        let cases = [
            (TRIAL.to_string(), true),
            (TRIAL.replace("## Abstract", "## ABSTRACT "), true),
            (TRIAL.replace("## Abstract", "## Summary"), false),
            (
                TRIAL.replace("Inhaler X was tested in adults with asthma.\n\n", ""),
                false,
            ),
        ];
        for (md, from_abstract) in cases {
            let canonical = canonical_from_markdown(&md);
            let (mut c, p) = store_with(&canonical, true);
            let stop = AtomicBool::new(false);
            let model = FakeModel::new(&stop);
            let mut chat = FakeChat::new(vec![suitable(false)]);
            let id = go(&mut c, &p, &mut chat, &model, None, false, &stop).unwrap();
            let sent = &chat.sent[0];
            assert_eq!(sent.contains("Exacerbations fell"), !from_abstract, "{md}");
            let PicoOutcome::NotSuitable(check) = outcome(&mut c, id) else {
                panic!("not a check");
            };
            assert_eq!(check.from_abstract, from_abstract, "{md}");
        }
    }

    /// A JATS abstract's sub-sections are read (its heading has no blocks of
    /// its own), not the body.
    #[test]
    fn check_reads_a_jats_abstract() {
        let xml = include_str!("../tests/fixtures/real_rct.xml");
        let canonical = canonical_from_jats(&jats_reader::parse(xml).unwrap());
        let abstract_ = abstract_text(&canonical).unwrap();
        assert!(
            abstract_.contains("included 60 adult patients undergoing elective three-port VATS")
        );
        let body = canonical
            .sections
            .iter()
            .skip_while(|s| {
                s.heading_span
                    .and_then(|h| canonical.span_text(h))
                    .is_none_or(|h| h != ABSTRACT_HEADING)
            })
            .skip(1)
            .find(|s| s.level == canonical.sections[0].level.min(s.level) && !s.blocks.is_empty())
            .and_then(|s| s.blocks.last())
            .map(|b| canonical.span_text(b.span).unwrap().to_string())
            .unwrap();
        let (mut c, p) = store_with(&canonical, true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![suitable(false)]);
        go(&mut c, &p, &mut chat, &model, None, false, &stop).unwrap();
        assert!(chat.sent[0].contains("included 60 adult patients"));
        assert!(!chat.sent[0].contains(body.as_str()), "{body}");
        assert_eq!(model.calls.get(), 0);
    }

    /// A passed check, then an extraction: one run with both, the population
    /// verified by the paper's own span.
    #[test]
    fn suitable_then_extracted() {
        let canonical = trial_with("");
        let (mut c, p) = store_with(&canonical, true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![suitable(true), extraction_quoting(ENROLLED)]);
        let id = go(&mut c, &p, &mut chat, &model, None, false, &stop).unwrap();
        assert_eq!(chat.sent.len(), 2);
        assert!(chat.sent[1].contains("Exacerbations fell"));
        let PicoOutcome::Extracted {
            check: Some(check),
            extraction,
        } = outcome(&mut c, id)
        else {
            panic!("not extracted after a check");
        };
        assert!(check.suitability.is_suitable);
        assert_eq!(extraction.population.state, PicoPartState::Verified);
        let cite = &extraction.population.citations[0];
        assert_eq!(cite.text, ENROLLED);
        let text = latest_canonical(&mut c, &p).unwrap().unwrap().text;
        assert_eq!(
            text.span_text(bmlib::fulltext::canonical::TextSpan {
                start: cite.start,
                end: cite.end
            }),
            Some(ENROLLED)
        );
    }

    /// A failed check is the whole run: one call, no extraction stored.
    #[test]
    fn not_suitable_stops_at_check() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![suitable(false)]);
        let id = go(&mut c, &p, &mut chat, &model, None, false, &stop).unwrap();
        assert_eq!(chat.sent.len(), 1);
        assert!(matches!(outcome(&mut c, id), PicoOutcome::NotSuitable(_)));
    }

    /// A forced run asks for the extraction only.
    #[test]
    fn forced_skips_check() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![extraction_quoting(ENROLLED)]);
        let id = go(&mut c, &p, &mut chat, &model, None, true, &stop).unwrap();
        assert_eq!(chat.sent.len(), 1);
        assert!(matches!(
            outcome(&mut c, id),
            PicoOutcome::Extracted { check: None, .. }
        ));
    }

    /// The context's reply room is every call's `max_tokens`.
    #[test]
    fn reply_room_is_each_calls_max_tokens() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![suitable(true), extraction_quoting(ENROLLED)]);
        let mut ctx = PicoContext {
            db: &mut c,
            chat: &mut chat,
            condenser_model: &model,
            model: "deepseek:m".into(),
            window: Some(1_000_000),
            reply_tokens: 32_768,
            retry_delay: Duration::ZERO,
            on_event: Arc::new(|_| {}),
        };
        run_pico(&mut ctx, &p, false, &stop).unwrap();
        assert_eq!(chat.max_tokens, [32_768, 32_768]);
    }

    /// A long paper's condenser calls get the context's reply room too, and
    /// the paper's budget keeps it free: a room leaving too little paper
    /// refuses the run before any call.
    #[test]
    fn condenser_gets_the_reply_room() {
        let extra = MAX_REPLY_TOKENS - MIN_REPLY_TOKENS;
        for (window, refused) in [(10_000 + extra, false), (10_000, true)] {
            let (mut c, p) = store_with(&trial_with(&filler(600)), true);
            let stop = AtomicBool::new(false);
            let model = FakeModel::new(&stop);
            let mut chat = FakeChat::new(vec![extraction_quoting(ENROLLED)]);
            let mut ctx = PicoContext {
                db: &mut c,
                chat: &mut chat,
                condenser_model: &model,
                model: "deepseek:m".into(),
                window: Some(window),
                reply_tokens: MAX_REPLY_TOKENS,
                retry_delay: Duration::ZERO,
                on_event: Arc::new(|_| {}),
            };
            let r = run_pico(&mut ctx, &p, true, &stop);
            if refused {
                assert!(matches!(r, Err(PicoRunError::TooLong { .. })), "{r:?}");
                assert_eq!(model.calls.get(), 0);
                assert!(chat.sent.is_empty());
            } else {
                r.unwrap();
                assert!(model.calls.get() > 0);
                let max = model.max_tokens.borrow();
                assert!(max.iter().all(|&m| m == MAX_REPLY_TOKENS as i64), "{max:?}");
                assert_eq!(chat.max_tokens, [MAX_REPLY_TOKENS as i64]);
            }
        }
    }

    /// A text over the budget is condensed: the extraction gets the digest,
    /// and a quote from the paper but not the digest is still verified, its
    /// span in the whole text (Review Focus 1).
    #[test]
    fn long_paper_is_condensed() {
        let canonical = trial_with(&filler(600));
        assert!(
            canonical.text.chars().count()
                > budget_chars(Some(10_000), MIN_REPLY_TOKENS, "Asthma trial").unwrap()
        );
        let (mut c, p) = store_with(&canonical, true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![extraction_quoting(ENROLLED)]);
        let id = go(&mut c, &p, &mut chat, &model, Some(10_000), true, &stop).unwrap();
        assert!(model.calls.get() > 0);
        assert!(chat.sent[0].contains(DIGEST));
        assert!(!chat.sent[0].contains("Filler sentence number 599"));
        let PicoOutcome::Extracted { extraction, .. } = outcome(&mut c, id) else {
            panic!("not extracted");
        };
        assert!(extraction.condensed_from_chars.is_some());
        let cite = &extraction.population.citations[0];
        assert_eq!(cite.text, ENROLLED);
        assert_eq!(
            &canonical
                .text
                .chars()
                .skip(cite.start)
                .take(cite.end - cite.start)
                .collect::<String>(),
            ENROLLED
        );
    }

    /// A long paper's run says which part it condenses, then that it is done;
    /// a short paper's says nothing (bmlib #609).
    #[test]
    fn condensation_reports_its_parts() {
        for (extra, window) in [(filler(600), Some(10_000)), (String::new(), None)] {
            let canonical = trial_with(&extra);
            let (mut c, p) = store_with(&canonical, true);
            let stop = AtomicBool::new(false);
            let model = FakeModel::new(&stop);
            let mut chat = FakeChat::new(vec![extraction_quoting(ENROLLED)]);
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let seen = Arc::clone(&events);
            let mut ctx = PicoContext {
                db: &mut c,
                chat: &mut chat,
                condenser_model: &model,
                model: "ollama:m".into(),
                window,
                reply_tokens: MIN_REPLY_TOKENS,
                retry_delay: Duration::ZERO,
                on_event: Arc::new(move |e| seen.lock().unwrap().push(e)),
            };
            run_pico(&mut ctx, &p, true, &stop).unwrap();
            let events = events.lock().unwrap().clone();
            if window.is_none() {
                assert!(events.is_empty(), "{events:?}");
                continue;
            }
            let parts: Vec<_> = (1..=model.calls.get())
                .map(|batch| PicoEvent::Condensing {
                    batch,
                    batches: model.calls.get(),
                    level: 0,
                })
                .chain([PicoEvent::Condensed])
                .collect();
            assert!(model.calls.get() > 1);
            assert_eq!(events, parts);
        }
    }

    /// Each stage of bmlib's condenser maps to what the tab shows.
    #[test]
    fn progress_stages_map() {
        let info = |stage: &str| {
            let mut info = ProgressInfo::new(stage);
            info.current_batch = 2;
            info.total_batches = 3;
            info.recursion_level = 1;
            info
        };
        assert_eq!(
            PicoEvent::from_progress(&info("extracting")),
            Some(PicoEvent::Condensing {
                batch: 2,
                batches: 3,
                level: 1
            })
        );
        assert_eq!(
            PicoEvent::from_progress(&info("complete")),
            Some(PicoEvent::Condensed)
        );
        for quiet in ["starting", "batching", "recursing"] {
            assert_eq!(PicoEvent::from_progress(&info(quiet)), None, "{quiet}");
        }
        let mut new_stage = info("a new stage");
        new_stage.message = "doing something new".into();
        assert_eq!(
            PicoEvent::from_progress(&new_stage),
            Some(PicoEvent::Other {
                stage: "a new stage".into(),
                message: "doing something new".into()
            })
        );
    }

    /// On a paper with accents and astral characters before the quote, the
    /// stored span is in code points of the whole text, short or condensed,
    /// and the view's UTF-16 span slices to the paper's own wording.
    #[test]
    fn non_ascii_spans_end_to_end() {
        assert!(!DIGEST.contains(ENROLLED));
        let md = TRIAL.replace("## Methods\n\n", "## Methods\n\nCafé 😀 𝛂 naïve.\n\n");
        for (extra, window) in [(String::new(), None), (filler(600), Some(10_000))] {
            let canonical = canonical_from_markdown(&format!("{md}\n{extra}\n"));
            let (mut c, p) = store_with(&canonical, true);
            let stop = AtomicBool::new(false);
            let model = FakeModel::new(&stop);
            let mut chat = FakeChat::new(vec![extraction_quoting(ENROLLED)]);
            let id = go(&mut c, &p, &mut chat, &model, window, true, &stop).unwrap();
            assert_eq!(model.calls.get() > 0, window.is_some());
            let PicoOutcome::Extracted { extraction, .. } = outcome(&mut c, id) else {
                panic!("not extracted");
            };
            assert_eq!(extraction.population.state, PicoPartState::Verified);
            let cite = &extraction.population.citations[0];
            assert_eq!(cite.text, ENROLLED);
            let view = crate::view::pico_run_view(&mut c, id).unwrap();
            let span = view.extraction.unwrap().parts[0].citations[0].span;
            let units: Vec<u16> = canonical.text.encode_utf16().collect();
            assert_eq!(
                String::from_utf16(&units[span.start..span.end]).unwrap(),
                ENROLLED
            );
        }
    }

    /// A long paper with no abstract is condensed once: the check and the
    /// extraction both read the same digest.
    #[test]
    fn no_abstract_condensed_once() {
        let canonical = canonical_from_markdown(&format!(
            "{}\n{}\n",
            TRIAL.replace("## Abstract", "## Summary"),
            filler(600)
        ));
        let stop = AtomicBool::new(false);
        let once = {
            let (mut c, p) = store_with(&canonical, true);
            let model = FakeModel::new(&stop);
            let mut chat = FakeChat::new(vec![extraction_quoting(ENROLLED)]);
            go(&mut c, &p, &mut chat, &model, Some(10_000), true, &stop).unwrap();
            model.calls.get()
        };
        let (mut c, p) = store_with(&canonical, true);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![suitable(true), extraction_quoting(ENROLLED)]);
        let id = go(&mut c, &p, &mut chat, &model, Some(10_000), false, &stop).unwrap();
        assert!(once > 0);
        assert_eq!(model.calls.get(), once);
        assert!(
            chat.sent.iter().all(|m| m.contains(DIGEST)),
            "{:?}",
            chat.sent
        );
        let PicoOutcome::Extracted {
            check: Some(check), ..
        } = outcome(&mut c, id)
        else {
            panic!("not extracted after a check");
        };
        assert!(!check.from_abstract);
    }

    /// A stop during the check's condensation stores nothing.
    #[test]
    fn stop_during_check_condensation() {
        let canonical = canonical_from_markdown(&format!(
            "{}\n{}\n",
            TRIAL.replace("## Abstract", "## Summary"),
            filler(600)
        ));
        let (mut c, p) = store_with(&canonical, true);
        let stop = AtomicBool::new(false);
        let mut model = FakeModel::new(&stop);
        model.stop_on = Some(0);
        let mut chat = FakeChat::new(vec![]);
        let r = go(&mut c, &p, &mut chat, &model, Some(10_000), false, &stop);
        assert!(matches!(r, Err(PicoRunError::Cancelled)), "{r:?}");
        assert!(chat.sent.is_empty());
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// A failed condensation batch fails the run: no extraction call, no row,
    /// never a run from a partial digest (Review Focus 1).
    #[test]
    fn failed_batch_fails_the_run() {
        let (mut c, p) = store_with(&trial_with(&filler(600)), true);
        let stop = AtomicBool::new(false);
        let mut model = FakeModel::new(&stop);
        model.fail_on = Some(0);
        let mut chat = FakeChat::new(vec![]);
        let r = go(&mut c, &p, &mut chat, &model, Some(10_000), true, &stop);
        assert!(
            matches!(r, Err(PicoRunError::Pico(PicoError::CondensationFailed(_)))),
            "{r:?}"
        );
        assert!(chat.sent.is_empty());
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// A quote that changes a number is not cited and is listed as not found
    /// (Review Focus 2).
    #[test]
    fn changed_number_is_not_cited() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let changed = "We enrolled 150 adults with asthma.";
        let mut chat = FakeChat::new(vec![extraction_quoting(changed)]);
        let id = go(&mut c, &p, &mut chat, &model, None, true, &stop).unwrap();
        let PicoOutcome::Extracted { extraction, .. } = outcome(&mut c, id) else {
            panic!("not extracted");
        };
        assert!(extraction.population.citations.is_empty());
        assert_eq!(extraction.population.quotes_not_found, vec![changed]);
    }

    /// A window too small for the paper is refused before any call.
    #[test]
    fn too_small_window_refused() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![]);
        let window = Some(MIN_REPLY_TOKENS);
        let r = go(&mut c, &p, &mut chat, &model, window, false, &stop);
        assert!(matches!(r, Err(PicoRunError::TooLong { window: w }) if w == window));
        assert!(chat.sent.is_empty());
    }

    /// A text that is not known to be the whole paper is refused.
    #[test]
    fn unconfirmed_text_refused() {
        let (mut c, p) = store_with(&trial_with(""), false);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![]);
        let r = go(&mut c, &p, &mut chat, &model, None, false, &stop);
        assert!(matches!(r, Err(PicoRunError::NotFullText)), "{r:?}");
        assert!(chat.sent.is_empty());
    }

    /// A failed request is retried (Review Focus 4).
    #[test]
    fn request_failed_is_retried() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![transport(), extraction_quoting(ENROLLED)]);
        go(&mut c, &p, &mut chat, &model, None, true, &stop).unwrap();
        assert_eq!(chat.sent.len(), 2);
        assert_eq!(runs_stored(&mut c), 1);
    }

    /// Once the attempts run out the failure is reported; the check is never
    /// asked again (Review Focus 4).
    #[test]
    fn retries_run_out() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut replies = vec![suitable(true)];
        replies.extend((0..RETRIEVAL_ATTEMPTS).map(|_| transport()));
        let mut chat = FakeChat::new(replies);
        let r = go(&mut c, &p, &mut chat, &model, None, false, &stop);
        assert!(
            matches!(r, Err(PicoRunError::Pico(PicoError::RequestFailed { .. }))),
            "{r:?}"
        );
        assert_eq!(chat.sent.len(), 1 + RETRIEVAL_ATTEMPTS as usize);
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// A refusal (a bad key, no such model) is reported at once, not retried
    /// as an outage (bmlib #659).
    #[test]
    fn a_refusal_is_not_retried() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![refused(), extraction_quoting(ENROLLED)]);
        let r = go(&mut c, &p, &mut chat, &model, None, true, &stop);
        assert!(
            matches!(
                r,
                Err(PicoRunError::Pico(PicoError::RequestFailed {
                    retryable: false,
                    ..
                }))
            ),
            "{r:?}"
        );
        assert_eq!(chat.sent.len(), 1);
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// A stop during a call that fails ends the run: not retried, nothing
    /// stored (Review Focus 4).
    #[test]
    fn stopped_is_not_retried() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![transport(), suitable(true)]).stopping(0, &stop);
        let r = go(&mut c, &p, &mut chat, &model, None, false, &stop);
        assert!(matches!(r, Err(PicoRunError::Cancelled)), "{r:?}");
        assert_eq!(chat.sent.len(), 1);
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// A stop stores nothing (Review Focus 4).
    #[test]
    fn stop_stores_nothing() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(true);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![suitable(true)]);
        let r = go(&mut c, &p, &mut chat, &model, None, false, &stop);
        assert!(matches!(r, Err(PicoRunError::Cancelled)), "{r:?}");
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// A stop during a call that succeeds discards its reply: (a) a failed
    /// check is not stored; (b) no extraction follows a passed one; (c) an
    /// extraction is not stored (Review Focus 4).
    #[test]
    fn stop_during_successful_call() {
        let cases: [(Vec<Reply>, usize, bool); 3] = [
            (vec![suitable(false)], 0, false),
            (vec![suitable(true)], 0, false),
            (vec![extraction_quoting(ENROLLED)], 0, true),
        ];
        for (replies, at, force) in cases {
            let (mut c, p) = store_with(&trial_with(""), true);
            let stop = AtomicBool::new(false);
            let model = FakeModel::new(&stop);
            let mut chat = FakeChat::new(replies).stopping(at, &stop);
            let r = go(&mut c, &p, &mut chat, &model, None, force, &stop);
            assert!(matches!(r, Err(PicoRunError::Cancelled)), "{r:?}");
            assert_eq!(chat.sent.len(), 1);
            assert_eq!(runs_stored(&mut c), 0);
        }
    }

    /// A stop during the condensation ends the run before the extraction, not
    /// retried, nothing stored (Review Focus 4).
    #[test]
    fn stop_during_condensation() {
        let (mut c, p) = store_with(&trial_with(&filler(600)), true);
        let stop = AtomicBool::new(false);
        let mut model = FakeModel::new(&stop);
        model.stop_on = Some(0);
        let mut chat = FakeChat::new(vec![]);
        let r = go(&mut c, &p, &mut chat, &model, Some(10_000), true, &stop);
        assert!(matches!(r, Err(PicoRunError::Cancelled)), "{r:?}");
        assert!(chat.sent.is_empty());
        assert_eq!(model.calls.get(), 1);
        assert_eq!(runs_stored(&mut c), 0);
    }

    /// A failed run keeps the previous one as the latest (Review Focus 4).
    #[test]
    fn failure_keeps_previous_run() {
        let (mut c, p) = store_with(&trial_with(""), true);
        let stop = AtomicBool::new(false);
        let model = FakeModel::new(&stop);
        let mut chat = FakeChat::new(vec![suitable(false)]);
        let first = go(&mut c, &p, &mut chat, &model, None, false, &stop).unwrap();
        let unusable = Err(ChatJsonError::Unparseable { attempts: 3 });
        let mut chat = FakeChat::new(vec![unusable]);
        let r = go(&mut c, &p, &mut chat, &model, None, true, &stop);
        assert!(
            matches!(r, Err(PicoRunError::Pico(PicoError::UnusableReply(_)))),
            "{r:?}"
        );
        let cid = latest_canonical(&mut c, &p).unwrap().unwrap().id;
        assert_eq!(latest_run(&mut c, &p, cid).unwrap(), Some(first));
        assert!(canonical_by_id(&mut c, &p, cid).unwrap().is_some());
    }

    /// A client with no providers configured; enough to build a chat.
    fn client() -> LlmClient {
        LlmClient::new(Arc::new(bmlib::http::UreqClient::new()))
    }

    /// The chat carries the window and the stop.
    #[test]
    fn pico_chat_carries_window() {
        let client = client();
        let stop = AtomicBool::new(false);
        let chat = pico_chat(&client, "ollama:m", Some(8192), &stop);
        assert_eq!(chat.num_ctx, NonZeroU32::new(8192));
        assert!(std::ptr::eq(chat.stop.unwrap(), &stop));
        assert_eq!(chat.model, "ollama:m");
        assert_eq!(pico_chat(&client, "ollama:m", None, &stop).num_ctx, None);
    }

    /// The condenser's model carries the window and the stop (Review Focus 1).
    #[test]
    fn pico_condenser_model_carries_window() {
        let client = client();
        let stop = AtomicBool::new(false);
        let model = pico_condenser_model(&client, "ollama:m", Some(8192), &stop);
        assert_eq!(model.num_ctx, NonZeroU32::new(8192));
        assert!(std::ptr::eq(model.stop.unwrap(), &stop));
        assert_eq!(model.model, "ollama:m");
        assert_eq!(
            pico_condenser_model(&client, "ollama:m", None, &stop).num_ctx,
            None
        );
    }

    /// Each refusal names its fix.
    #[test]
    fn display_names_the_fix() {
        let known = PicoRunError::TooLong { window: Some(2048) }.to_string();
        assert!(
            known.contains("2048") && known.contains("larger window"),
            "{known}"
        );
        let assumed = PicoRunError::TooLong { window: None }.to_string();
        assert!(assumed.contains("assumed"), "{assumed}");
        assert!(assumed.contains(&UNKNOWN_WINDOW_FIT_TOKENS.to_string()));
        for e in [
            PicoError::UnusableReply("x".into()),
            PicoError::Truncated("x".into()),
        ] {
            let m = PicoRunError::Pico(e).to_string();
            assert!(m.ends_with("Try another model."), "{m}");
            assert!(m.contains("(x)"), "bmlib's detail is kept: {m}");
        }
    }
}
