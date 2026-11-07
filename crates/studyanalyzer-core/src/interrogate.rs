//! Asking a question of one paper: pick whole-paper or retrieval mode, stream
//! the model's answer, locate its quotes in the paper, store the turn.
//!
//! Nothing is answered without the paper's full text: no canonical text, or
//! one that neither passed the full-text check nor was confirmed by the user,
//! is refused before the model is asked.

use std::fmt;
use std::num::NonZeroU32;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};

use bmlib::db::{transaction_with, Db};
use bmlib::embed::store::ChunkEmbedder;
use bmlib::fulltext::canonical::TextSpan;
use bmlib::llm::{
    ChatError, ChatRequest, LLMResponse, LlmClient, StreamDelta, CANCELLED_STOP_REASON,
};
use serde::Serialize;

use crate::answer::{locate, parse_answer};
use crate::config::{
    ANSWER_TEMPERATURE, LENGTH_STOP_REASONS, LLM_MODEL_KEY, MIN_QUOTE_SIMILARITY, NO_FULL_TEXT_YET,
    UNKNOWN_WINDOW_FIT_TOKENS,
};
use crate::conversations::{
    create_conversation, current_conversation, save_turn, turns, Mode, NewTurn, TurnStatus,
};
use crate::embedder::{EmbedderError, ModelSlot};
use crate::passages::{find_passages, PassageError};
use crate::prompt::{build_messages, choose_mode, fits, paper_context, passages_context};
use crate::store::{
    drop_superseded_chunk_set, get_setting, latest_canonical, PaperId, StoreError, StoredCanonical,
};
use crate::view::full_text_status;

/// A model that streams an answer; [`LlmClient`] in the app, a fake in tests.
pub trait ChatStreamer {
    /// Sends `request`, calling `on_delta` for every piece of the answer as it
    /// arrives. Setting `cancel` (even before the first piece, while a local
    /// model still reads the prompt) or `on_delta` returning `Break` stops the
    /// stream, and the response then holds what arrived with `stop_reason`
    /// [`CANCELLED_STOP_REASON`] (or the server's own, when its end marker had
    /// already arrived); a `cancel` already set sends nothing.
    ///
    /// # Errors
    /// The model call's own [`ChatError`].
    fn chat_stream(
        &self,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_delta: &mut dyn FnMut(&StreamDelta) -> ControlFlow<()>,
    ) -> Result<LLMResponse, ChatError>;
}

impl ChatStreamer for LlmClient {
    /// Passes the call to bmlib's [`LlmClient::chat_stream_with_cancel`] (#635).
    fn chat_stream(
        &self,
        request: &ChatRequest,
        cancel: &AtomicBool,
        on_delta: &mut dyn FnMut(&StreamDelta) -> ControlFlow<()>,
    ) -> Result<LLMResponse, ChatError> {
        LlmClient::chat_stream_with_cancel(self, request, cancel, on_delta)
    }
}

/// What the user sees while a question is being answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerEvent {
    /// The search model on disk is being checked and loaded (the first
    /// retrieval of a launch hashes its whole file, which takes a while).
    CheckingModel,
    /// The paper is being indexed for retrieval: `done` of `total` chunks.
    Indexing {
        /// Chunks embedded so far.
        done: usize,
        /// Chunks in all.
        total: usize,
    },
    /// The model has been asked; its first text has not arrived yet.
    Waiting,
    /// A piece of the answer's text.
    Delta {
        /// The new text.
        text: String,
    },
}

/// What [`ask`] works with.
pub struct AskContext<'a> {
    /// The store.
    pub db: &'a mut dyn Db,
    /// The model.
    pub llm: &'a dyn ChatStreamer,
    /// The embedder for retrieval; `None` when it is not loaded.
    pub embedder: Option<&'a mut dyn ChunkEmbedder>,
    /// The `provider:model` to ask.
    pub model: String,
    /// The model's context window in tokens, `None` when unknown.
    pub window: Option<u64>,
    /// The room for the answer in tokens: its `max_tokens`, and what the fit
    /// check keeps free (`prompt::reply_tokens`).
    pub reply_tokens: u64,
}

/// Why a question was not answered. A stopped or broken answer is not an
/// error: it is stored as a `Stopped` or `Failed` turn.
#[derive(Debug)]
pub enum AskError {
    /// No model is chosen in Settings.
    NeedsModel,
    /// The question is blank.
    EmptyQuestion,
    /// The paper has no canonical text.
    NoFullText,
    /// The text neither passed the full-text check nor was confirmed.
    NotFullText,
    /// Retrieval is needed (the paper does not fit, or the window is unknown)
    /// and no embedder is loaded.
    NeedsEmbedder,
    /// The prompt does not fit `window` tokens (the checked limit: the model's
    /// window, or [`UNKNOWN_WINDOW_FIT_TOKENS`] when that is unknown).
    TooLong {
        /// The limit the prompt was checked against.
        window: u64,
        /// Whether earlier complete turns were in the prompt; without them the
        /// passages alone did not fit, and a new conversation would not help.
        has_history: bool,
    },
    /// The user stopped before the model was asked, or before any of its
    /// answer's text arrived; nothing was stored.
    Cancelled,
    /// The model call failed before any text arrived; nothing was stored.
    Chat(ChatError),
    /// Finding the passages failed.
    Passages(PassageError),
    /// The store failed.
    Store(StoreError),
    /// The search model on disk could not be checked or loaded.
    Embedder(EmbedderError),
    /// A bug or damaged data: a stored span outside the paper, or a bad
    /// similarity setting. The message says which.
    Internal(String),
}

impl fmt::Display for AskError {
    /// A sentence for the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AskError::NeedsModel => write!(f, "Choose a model in Settings first."),
            AskError::EmptyQuestion => write!(f, "Type a question first."),
            AskError::NoFullText => write!(f, "{NO_FULL_TEXT_YET}"),
            AskError::NotFullText => write!(
                f,
                "This text did not pass the full-text check. Confirm that it is the whole paper, or add the full text, before asking about it."
            ),
            AskError::NeedsEmbedder => write!(
                f,
                "Answering needs the search model, which is not downloaded yet."
            ),
            AskError::TooLong {
                window,
                has_history: true,
            } => write!(
                f,
                "This conversation no longer fits the model's context window ({window} tokens). Start a new conversation."
            ),
            AskError::TooLong {
                window,
                has_history: false,
            } => write!(
                f,
                "The passages found for this question do not fit the model's context window ({window} tokens). Enter a larger context window in Settings, or choose a model with a larger window."
            ),
            AskError::Cancelled => write!(f, "The question was cancelled."),
            AskError::Chat(e) => write!(f, "The model could not answer: {e}"),
            AskError::Passages(e) => write!(f, "{e}"),
            AskError::Store(e) => write!(f, "Could not read or save the conversation: {e}"),
            AskError::Embedder(e) => write!(f, "{e}"),
            AskError::Internal(m) => write!(f, "Something went wrong preparing the answer: {m}"),
        }
    }
}

impl std::error::Error for AskError {}

/// Logs `e` for `paper` (error for a bug or the store, info for a cancel,
/// warn for the rest) and returns it.
fn fail(paper: &PaperId, e: AskError) -> AskError {
    match &e {
        AskError::Internal(_) | AskError::Store(_) | AskError::Embedder(_) => {
            log::error!("paper {}: asking failed: {e}", paper.as_str())
        }
        AskError::Cancelled => log::info!("paper {}: question cancelled", paper.as_str()),
        _ => log::warn!("paper {}: question refused: {e}", paper.as_str()),
    }
    e
}

/// The window as Ollama's `num_ctx`: saturated into `u32`, none for an unknown
/// (or zero) window.
pub fn num_ctx(window: Option<u64>) -> Option<NonZeroU32> {
    window.and_then(|w| NonZeroU32::new(u32::try_from(w).unwrap_or(u32::MAX)))
}

/// The paper's latest text, if it may be asked about: [`AskError::NoFullText`]
/// without one, [`AskError::NotFullText`] when it neither passed the full-text
/// check nor was confirmed.
pub(crate) fn full_text(db: &mut dyn Db, paper: &PaperId) -> Result<StoredCanonical, AskError> {
    let stored = latest_canonical(db, paper)
        .map_err(AskError::Store)?
        .ok_or(AskError::NoFullText)?;
    let status = full_text_status(&stored.assessment_json, stored.confirmed_full_text)
        .map_err(AskError::Store)?;
    if !status.passed && !status.confirmed {
        return Err(AskError::NotFullText);
    }
    Ok(stored)
}

/// The checks before the model is reached, in order: a model is chosen (its
/// `provider:model` is returned), then the paper's text may be asked about.
/// So a paper without full text is refused as such even offline or keyless.
///
/// # Errors
/// [`AskError::NeedsModel`], [`AskError::NoFullText`],
/// [`AskError::NotFullText`] or [`AskError::Store`], logged here.
pub fn ready_to_ask(db: &mut dyn Db, paper: &PaperId) -> Result<String, AskError> {
    let model = get_setting(db, LLM_MODEL_KEY)
        .map_err(|e| fail(paper, AskError::Store(e)))?
        .ok_or_else(|| fail(paper, AskError::NeedsModel))?;
    full_text(db, paper).map_err(|e| fail(paper, e))?;
    Ok(model)
}

/// What [`ask_loading_embedder`] works with: [`AskContext`] with the app's
/// search model slot in place of a loaded embedder.
pub struct Asker<'a, E> {
    /// The store.
    pub db: &'a mut dyn Db,
    /// The model.
    pub llm: &'a dyn ChatStreamer,
    /// The search model, loaded or not.
    pub embedder: &'a ModelSlot<E>,
    /// The `provider:model` to ask.
    pub model: String,
    /// The model's context window in tokens, `None` when unknown.
    pub window: Option<u64>,
    /// The room for the answer in tokens: its `max_tokens`, and what the fit
    /// check keeps free (`prompt::reply_tokens`).
    pub reply_tokens: u64,
}

/// [`ask`] with the slot's embedder. When retrieval needs it and it is not
/// loaded but its file is on disk (a new launch), it is checked and loaded
/// (announced as [`AnswerEvent::CheckingModel`]), and the question asked once
/// more; nothing was sent or stored before [`AskError::NeedsEmbedder`], so
/// asking again is safe.
///
/// # Errors
/// As [`ask`], plus [`AskError::Embedder`] when the file cannot be checked or
/// loaded.
pub fn ask_loading_embedder<E: ChunkEmbedder>(
    asker: &mut Asker<E>,
    paper: &PaperId,
    question: &str,
    stop: &AtomicBool,
    on_event: &mut dyn FnMut(AnswerEvent),
) -> Result<i64, AskError> {
    let mut loaded = asker.embedder.is_loaded();
    loop {
        let mut shared = asker.embedder.shared();
        let mut ctx = AskContext {
            db: &mut *asker.db,
            llm: asker.llm,
            embedder: loaded.then_some(&mut shared as &mut dyn ChunkEmbedder),
            model: asker.model.clone(),
            window: asker.window,
            reply_tokens: asker.reply_tokens,
        };
        match ask(&mut ctx, paper, question, stop, on_event) {
            Err(AskError::NeedsEmbedder) if !loaded => {
                if asker.embedder.downloaded() {
                    on_event(AnswerEvent::CheckingModel);
                }
                match asker.embedder.ensure_loaded() {
                    Ok(true) => {
                        log::info!(
                            "paper {}: search model loaded; asking again",
                            paper.as_str()
                        );
                        loaded = true;
                    }
                    Ok(false) => return Err(AskError::NeedsEmbedder),
                    Err(e) => return Err(fail(paper, AskError::Embedder(e))),
                }
            }
            result => return result,
        }
    }
}

/// Asks `question` of `paper` and stores the turn; returns the conversation id.
///
/// The whole paper is sent when it fits the window, else the passages closest
/// to the question (indexing the paper first, reported as
/// [`AnswerEvent::Indexing`]). [`AnswerEvent::Waiting`] says the model has
/// been asked; the answer then streams as [`AnswerEvent::Delta`].
/// Setting `stop` cancels indexing or an answer with no text yet (nothing
/// stored, [`AskError::Cancelled`]), or the answer (a `Stopped` turn with what
/// arrived, no citations). A completed answer's quotes are located in the
/// paper; one cut off at its `reply_tokens` says so in its `error`. A stream
/// that breaks after some text is a `Failed` turn keeping it. The
/// conversation is created, if there is none on this text, in the same
/// transaction that stores the turn.
///
/// The caller passes a fresh `stop` flag (false) for each ask: one already set
/// cancels before the model is asked.
///
/// # Errors
/// See [`AskError`]; every error is logged here with the paper id.
pub fn ask(
    ctx: &mut AskContext,
    paper: &PaperId,
    question: &str,
    stop: &AtomicBool,
    on_event: &mut dyn FnMut(AnswerEvent),
) -> Result<i64, AskError> {
    let fail = |e| fail(paper, e);
    if question.trim().is_empty() {
        return Err(fail(AskError::EmptyQuestion));
    }
    let db = &mut *ctx.db;
    let stored = full_text(db, paper).map_err(fail)?;
    let canonical = &stored.text;
    let conversation =
        current_conversation(db, paper, stored.id).map_err(|e| fail(AskError::Store(e)))?;
    let history = match conversation {
        Some(id) => turns(db, id).map_err(|e| fail(AskError::Store(e)))?,
        None => Vec::new(),
    };
    let internal = |e: &dyn fmt::Display| fail(AskError::Internal(e.to_string()));
    let whole_context = paper_context(canonical).map_err(|e| internal(&e))?;
    let whole = build_messages(&whole_context, &history, question);
    let mode = choose_mode(ctx.window, ctx.reply_tokens, &whole);
    let (messages, passages): (_, Vec<TextSpan>) = match mode {
        Mode::WholePaper => (whole, Vec::new()),
        Mode::Retrieval => {
            let embedder = ctx
                .embedder
                .as_deref_mut()
                .ok_or_else(|| fail(AskError::NeedsEmbedder))?;
            let found = find_passages(
                db,
                stored.id,
                canonical,
                question,
                embedder,
                &mut |done, total| {
                    on_event(AnswerEvent::Indexing { done, total });
                    !stop.load(Ordering::SeqCst)
                },
            )
            .map_err(|e| match e {
                PassageError::Cancelled => fail(AskError::Cancelled),
                e => fail(AskError::Passages(e)),
            });
            // Even after a failed search: indexing may have written the set first (#55).
            // The hits stay good for this ask; only the stored set must not outlive a newer text.
            let dropped =
                drop_superseded_chunk_set(db, stored.id).map_err(|e| fail(AskError::Store(e)));
            // Both errors are logged above; the passage error is the one reported.
            let hits = found?;
            dropped?;
            let context = passages_context(canonical, &hits).map_err(|e| internal(&e))?;
            let messages = build_messages(&context, &history, question);
            let limit = ctx.window.unwrap_or(UNKNOWN_WINDOW_FIT_TOKENS);
            if !fits(Some(limit), ctx.reply_tokens, &messages) {
                return Err(fail(AskError::TooLong {
                    window: limit,
                    has_history: history.iter().any(|t| t.status == TurnStatus::Complete),
                }));
            }
            (messages, hits.iter().map(|h| h.span).collect())
        }
    };
    if stop.load(Ordering::SeqCst) {
        return Err(fail(AskError::Cancelled));
    }
    let mut request = ChatRequest::new(messages).with_model(ctx.model.clone());
    request.max_tokens = i64::try_from(ctx.reply_tokens).unwrap_or(i64::MAX);
    request.temperature = ANSWER_TEMPERATURE;
    request.num_ctx = num_ctx(ctx.window);
    on_event(AnswerEvent::Waiting);
    let mut received = String::new();
    let result = ctx.llm.chat_stream(&request, stop, &mut |delta| {
        if !stop.load(Ordering::SeqCst) {
            if let StreamDelta::Text(text) = delta {
                received.push_str(text);
                on_event(AnswerEvent::Delta { text: text.clone() });
            }
        }
        if stop.load(Ordering::SeqCst) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    let mut turn = NewTurn {
        question: question.to_string(),
        answer: String::new(),
        status: TurnStatus::Complete,
        error: None,
        model: ctx.model.clone(),
        mode,
        passages,
        quotes: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
    };
    match result {
        Ok(response) => {
            let stop_reason = response.stop_reason.as_deref();
            if stop_reason == Some(CANCELLED_STOP_REASON) && received.is_empty() {
                // Stopped before any text was shown: nothing to keep (bmlib #635).
                return Err(fail(AskError::Cancelled));
            }
            if stop_reason == Some(CANCELLED_STOP_REASON) {
                log::info!("paper {}: answer stopped by the user", paper.as_str());
                turn.status = TurnStatus::Stopped;
                // What was shown: bmlib's content may hold a delta that came after the stop.
                turn.answer = std::mem::take(&mut received);
            } else {
                let located = locate(
                    &parse_answer(&response.content),
                    canonical,
                    MIN_QUOTE_SIMILARITY,
                )
                .map_err(|e| internal(&e))?;
                let missing = located.quotes.iter().filter(|q| q.span.is_none()).count();
                if missing > 0 {
                    log::warn!(
                        "paper {}: {missing} of {} quotes not found in the paper",
                        paper.as_str(),
                        located.quotes.len()
                    );
                }
                if stop_reason.is_some_and(|r| LENGTH_STOP_REASONS.contains(&r)) {
                    log::warn!(
                        "paper {}: the answer reached the length limit ({} tokens)",
                        paper.as_str(),
                        ctx.reply_tokens
                    );
                    turn.error = Some(format!(
                        "The answer reached the length limit ({} tokens) and was cut off.",
                        ctx.reply_tokens
                    ));
                }
                turn.quotes = located.quotes;
                turn.answer = response.content;
            }
            turn.input_tokens = response.input_tokens;
            turn.output_tokens = response.output_tokens;
        }
        Err(ChatError::Stream(e)) if !received.is_empty() => {
            log::warn!("paper {}: answer broke off: {e}", paper.as_str());
            turn.status = TurnStatus::Failed;
            turn.error = Some(ChatError::Stream(e).to_string());
            turn.answer = received;
        }
        Err(e) => return Err(fail(AskError::Chat(e))),
    }
    transaction_with(db, |tx| {
        let id = match conversation {
            Some(id) => id,
            None => create_conversation(tx, paper, stored.id)?,
        };
        save_turn(tx, id, &turn)?;
        Ok::<_, StoreError>(id)
    })
    .map_err(|e| fail(AskError::Store(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        EMBEDDING_MODEL, LENGTH_STOP_REASONS, LLM_MODEL_KEY, MIN_REPLY_TOKENS,
        UNKNOWN_WINDOW_FIT_TOKENS,
    };
    use crate::conversations::{current_conversation, save_turn, turns, Mode, NewTurn, TurnStatus};
    use crate::identifiers::Identifiers;
    use crate::intake::SourceKind;
    use crate::passages::tests::FakeEmbedder;
    use crate::store::{
        create_paper, latest_canonical, open_memory_store, save_canonical, save_source,
        set_setting, NewSource, PaperId,
    };
    use bmlib::embed::store::{BoxError, ChunkEmbedder};
    use bmlib::embed::ModelFile;
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_markdown};
    use bmlib::llm::{ChatError, ChatRequest, LLMResponse, StreamDelta, CANCELLED_STOP_REASON};
    use rusqlite::Connection;
    use std::cell::RefCell;
    use std::num::NonZeroU32;
    use std::ops::ControlFlow;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Streams scripted deltas, then returns a scripted result; records each
    /// request. `stop_before: Some((i, flag))` sets `flag` just before delta `i`
    /// is delivered, as a user pressing Stop between two deltas.
    struct FakeChat<'s> {
        deltas: Vec<StreamDelta>,
        result: Result<LLMResponse, ChatError>,
        requests: RefCell<Vec<ChatRequest>>,
        stop_before: Option<(usize, &'s AtomicBool)>,
    }

    impl<'s> FakeChat<'s> {
        /// A fake that streams `deltas`, then ends with `result`.
        fn scripted(deltas: Vec<StreamDelta>, result: Result<LLMResponse, ChatError>) -> Self {
            FakeChat {
                deltas,
                result,
                requests: RefCell::new(Vec::new()),
                stop_before: None,
            }
        }

        /// A fake that streams `deltas` as text, then ends with `result`.
        fn new(deltas: &[&str], result: Result<LLMResponse, ChatError>) -> Self {
            FakeChat::scripted(
                deltas
                    .iter()
                    .map(|d| StreamDelta::Text((*d).into()))
                    .collect(),
                result,
            )
        }

        /// A fake that streams `deltas` and completes with their text.
        fn answering(deltas: &[&str]) -> Self {
            let mut r = LLMResponse::new(deltas.concat());
            r.input_tokens = 11;
            r.output_tokens = 5;
            FakeChat::new(deltas, Ok(r))
        }

        /// How many requests it received.
        fn calls(&self) -> usize {
            self.requests.borrow().len()
        }
    }

    impl ChatStreamer for FakeChat<'_> {
        fn chat_stream(
            &self,
            request: &ChatRequest,
            cancel: &AtomicBool,
            on_delta: &mut dyn FnMut(&StreamDelta) -> ControlFlow<()>,
        ) -> Result<LLMResponse, ChatError> {
            // As bmlib does, a cancel already set sends nothing.
            if cancel.load(Ordering::SeqCst) {
                let mut r = LLMResponse::new(String::new());
                r.stop_reason = Some(CANCELLED_STOP_REASON.into());
                return Ok(r);
            }
            self.requests.borrow_mut().push(request.clone());
            let mut content = String::new();
            for (i, d) in self.deltas.iter().enumerate() {
                if let Some((at, flag)) = self.stop_before {
                    if at == i {
                        flag.store(true, Ordering::SeqCst);
                    }
                }
                // As bmlib does, the delta is in the content before `on_delta` sees it.
                if let StreamDelta::Text(t) = d {
                    content.push_str(t);
                }
                if on_delta(d).is_break() {
                    let mut r = LLMResponse::new(content);
                    r.stop_reason = Some(CANCELLED_STOP_REASON.into());
                    return Ok(r);
                }
            }
            self.result.clone()
        }
    }

    /// `n` words of filler.
    fn words(w: &str, n: usize) -> String {
        format!("{w} ").repeat(n)
    }

    /// A paper that passes the full-text check and says "61 were over 75 years".
    fn short_paper() -> String {
        format!(
            "# Trial\n\n## Methods\n\n{}\n\n## Results\n\nOf the patients, 61 were over 75 years.\n\n## Discussion\n\nIt worked.\n",
            words("method", 1100)
        )
    }

    /// A passing paper far longer than 8 passages.
    fn long_paper() -> String {
        format!(
            "# Trial\n\n## Methods\n\n{}\n\n## Results\n\nOf the patients, 61 were over 75 years.\n\n{}\n\n## Discussion\n\n{}\n",
            words("alpha", 4000),
            words("hazard ratio", 2000),
            words("limitations", 3000)
        )
    }

    /// A store holding one paper with `md` as its canonical text, confirmed or not.
    fn store_with(md: &str, confirmed: bool) -> (Connection, PaperId) {
        let mut c = open_memory_store().unwrap();
        let p = seed(&mut c, md, confirmed);
        (c, p)
    }

    /// Stores a paper in `c` with `md` as its text (confirmed or not).
    fn seed(c: &mut Connection, md: &str, confirmed: bool) -> PaperId {
        let ids = Identifiers {
            pmid: Some("1".into()),
            ..Default::default()
        };
        let p = create_paper(c, &ids, Some("Trial")).unwrap();
        let cid = save_text(c, &p, md);
        if confirmed {
            crate::store::confirm_full_text(c, &p, cid).unwrap();
        }
        p
    }

    /// Saves `md` as a new source and canonical text of `p`; returns the text's id.
    fn save_text(c: &mut Connection, p: &PaperId, md: &str) -> i64 {
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
        )
        .unwrap();
        let text = canonical_from_markdown(md);
        let a = assess_full_text(&text).unwrap();
        save_canonical(c, p, sid, &text, &a).unwrap()
    }

    /// Embeds documents as [`FakeEmbedder`] does but fails every query, so a
    /// search fails only after indexing has written the set.
    struct QueryFails;

    impl ChunkEmbedder for QueryFails {
        fn model(&self) -> Option<&ModelFile> {
            Some(&EMBEDDING_MODEL)
        }
        fn embed_documents(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, BoxError> {
            FakeEmbedder.embed_documents(texts)
        }
        fn embed_query(&mut self, _query: &str) -> Result<Vec<f32>, BoxError> {
            Err("the query embedder is down".into())
        }
    }

    /// How many chunk sets canonical text `id` has.
    fn sets(c: &mut Connection, id: i64) -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM embedded_chunk_sets WHERE paper_key = ?",
            [crate::passages::index_key(id)],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// #55: an ask indexing the older text while a newer text is saved on
    /// another connection leaves no set for the older text, whether its
    /// search after indexing succeeds or fails.
    #[test]
    fn superseded_set_is_dropped_after_the_ask() {
        for query_fails in [false, true] {
            let dir = std::env::temp_dir().join(format!("sa-55-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("store.sqlite");
            let mut c = crate::store::open_store(&path).unwrap();
            let p = seed(&mut c, &long_paper(), false);
            let old = canonical_id(&mut c, &p);
            let mut other = crate::store::open_store(&path).unwrap();
            let chat = FakeChat::answering(&["x"]);
            let (mut fake, mut failing) = (FakeEmbedder, QueryFails);
            let embedder: &mut dyn ChunkEmbedder =
                if query_fails { &mut failing } else { &mut fake };
            let mut ctx = AskContext {
                db: &mut c,
                llm: &chat,
                embedder: Some(embedder),
                model: "m".into(),
                window: Some(10_000),
                reply_tokens: MIN_REPLY_TOKENS,
            };
            let stop = AtomicBool::new(false);
            let mut newer = None;
            // Indexing events come before bmlib writes the set, as a save racing the ask.
            let r = ask(&mut ctx, &p, "hazard ratio", &stop, &mut |e| {
                if matches!(e, AnswerEvent::Indexing { .. }) && newer.is_none() {
                    newer = Some(save_text(&mut other, &p, "# Trial\n\nA newer reading."));
                }
            });
            assert_eq!(r.is_err(), query_fails, "{r:?}");
            if query_fails {
                assert!(matches!(r, Err(AskError::Passages(_))), "{r:?}");
            }
            assert!(newer.is_some());
            assert_eq!(sets(&mut c, old), 0, "query_fails = {query_fails}");
            drop((c, other));
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// A complete earlier turn whose answer is far too long for a small window.
    fn long_turn() -> NewTurn {
        NewTurn {
            question: "Earlier?".into(),
            answer: words("long", 10_000),
            status: TurnStatus::Complete,
            error: None,
            model: "m".into(),
            mode: Mode::Retrieval,
            passages: vec![],
            quotes: vec![],
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    /// The paper's latest canonical id.
    fn canonical_id(c: &mut Connection, p: &PaperId) -> i64 {
        latest_canonical(c, p).unwrap().unwrap().id
    }

    /// Asks `question` with `window`, with or without the fake embedder;
    /// returns the result and the events seen.
    fn run(
        c: &mut Connection,
        p: &PaperId,
        chat: &FakeChat,
        window: Option<u64>,
        with_embedder: bool,
        question: &str,
    ) -> (Result<i64, AskError>, Vec<AnswerEvent>) {
        let mut embedder = FakeEmbedder;
        let mut ctx = AskContext {
            db: c,
            llm: chat,
            embedder: if with_embedder {
                Some(&mut embedder)
            } else {
                None
            },
            model: "ollama:m".into(),
            window,
            reply_tokens: MIN_REPLY_TOKENS,
        };
        let stop = AtomicBool::new(false);
        let mut events = Vec::new();
        let r = ask(&mut ctx, p, question, &stop, &mut |e| events.push(e));
        (r, events)
    }

    /// No model is `NeedsModel` before the paper is looked at (it has no text
    /// here); with a model, a missing or unchecked text is refused; a paper
    /// ready to ask gives the model.
    #[test]
    fn needs_model_before_anything_else() {
        let mut c = open_memory_store().unwrap();
        let p = create_paper(&mut c, &Identifiers::default(), Some("T")).unwrap();
        assert!(matches!(
            ready_to_ask(&mut c, &p),
            Err(AskError::NeedsModel)
        ));
        set_setting(&mut c, LLM_MODEL_KEY, "ollama:m").unwrap();
        assert!(matches!(
            ready_to_ask(&mut c, &p),
            Err(AskError::NoFullText)
        ));
        let (mut c, p) = store_with("# Note\n\nJust a short note.\n", false);
        set_setting(&mut c, LLM_MODEL_KEY, "ollama:m").unwrap();
        assert!(matches!(
            ready_to_ask(&mut c, &p),
            Err(AskError::NotFullText)
        ));
        let (mut c, p) = store_with(&short_paper(), false);
        assert!(matches!(
            ready_to_ask(&mut c, &p),
            Err(AskError::NeedsModel)
        ));
        set_setting(&mut c, LLM_MODEL_KEY, "ollama:m").unwrap();
        assert_eq!(ready_to_ask(&mut c, &p).unwrap(), "ollama:m");
    }

    /// Asks through `slot` with an unknown window (so retrieval); returns the
    /// result, how many requests the model got, and the events seen.
    fn ask_through(
        slot: &crate::embedder::ModelSlot<FakeEmbedder>,
    ) -> (Connection, Result<i64, AskError>, usize, Vec<AnswerEvent>) {
        let (mut c, p) = store_with(&long_paper(), false);
        let chat = FakeChat::answering(&["x"]);
        let mut asker = Asker {
            db: &mut c,
            llm: &chat,
            embedder: slot,
            model: "ollama:m".into(),
            window: None,
            reply_tokens: MIN_REPLY_TOKENS,
        };
        let stop = AtomicBool::new(false);
        let mut events = Vec::new();
        let r = ask_loading_embedder(&mut asker, &p, "hazard ratio", &stop, &mut |e| {
            events.push(e)
        });
        let calls = chat.calls();
        (c, r, calls, events)
    }

    /// A paper needing retrieval, with the model file on disk but not loaded
    /// (a new launch): the user is told it is being checked, it is loaded, and
    /// the question asked once more.
    #[test]
    fn needs_embedder_loads_then_asks_again() {
        use crate::embedder::tests::{put_file, tiny_slot};
        let slot = tiny_slot();
        put_file(&slot, b"model");
        let (mut c, r, calls, events) = ask_through(&slot);
        let conv = r.unwrap();
        assert!(slot.is_loaded());
        assert_eq!(calls, 1);
        assert_eq!(events.first(), Some(&AnswerEvent::CheckingModel));
        assert_eq!(
            events
                .iter()
                .filter(|e| **e == AnswerEvent::CheckingModel)
                .count(),
            1
        );
        assert_eq!(turns(&mut c, conv).unwrap()[0].mode, Mode::Retrieval);
        std::fs::remove_dir_all(slot.path().parent().unwrap()).unwrap();
    }

    /// With no model file the ask is still `NeedsEmbedder`, unasked.
    #[test]
    fn needs_embedder_without_the_file() {
        let slot = crate::embedder::tests::tiny_slot();
        let (_, r, calls, events) = ask_through(&slot);
        assert!(matches!(r, Err(AskError::NeedsEmbedder)));
        assert!(events.is_empty(), "nothing to check: {events:?}");
        assert_eq!(calls, 0);
        assert!(!slot.is_loaded());
    }

    /// A whole-paper answer streams its deltas and stores one located quote.
    #[test]
    fn whole_paper_answer_cited() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::answering(&["Over 75: ", "<q>61 were over 75 years</q>", "."]);
        let (r, events) = run(&mut c, &p, &chat, Some(100_000), true, "How old?");
        let conv = r.unwrap();
        let deltas: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AnswerEvent::Delta { .. }))
            .collect();
        assert_eq!(deltas.len(), 3);
        let got = turns(&mut c, conv).unwrap();
        assert_eq!(got.len(), 1);
        let t = &got[0];
        assert_eq!((t.status, t.mode), (TurnStatus::Complete, Mode::WholePaper));
        assert!(t.passages.is_empty());
        assert_eq!(t.quotes.len(), 1);
        assert_eq!(t.quotes[0].number, Some(1));
        let span = t.quotes[0].span.unwrap();
        let text = latest_canonical(&mut c, &p).unwrap().unwrap().text;
        assert_eq!(&text.text[span.start..span.end], "61 were over 75 years");
        assert_eq!((t.input_tokens, t.output_tokens), (11, 5));
        assert_eq!(t.model, "ollama:m");
    }

    /// A text that failed the check and is unconfirmed is refused unasked.
    #[test]
    fn unconfirmed_text_refused() {
        let (mut c, p) = store_with("# Note\n\nJust a short note.\n", false);
        let chat = FakeChat::answering(&["x"]);
        let (r, _) = run(&mut c, &p, &chat, Some(100_000), true, "Q?");
        assert!(matches!(r, Err(AskError::NotFullText)));
        assert_eq!(chat.calls(), 0);
    }

    /// A failing text the user confirmed is asked.
    #[test]
    fn confirmed_text_is_asked() {
        let (mut c, p) = store_with("# Note\n\nJust a short note.\n", true);
        let chat = FakeChat::answering(&["x"]);
        let (r, _) = run(&mut c, &p, &chat, Some(100_000), true, "Q?");
        assert!(r.is_ok());
        assert_eq!(chat.calls(), 1);
    }

    /// A paper with no canonical text is refused.
    #[test]
    fn no_text_refused() {
        let mut c = open_memory_store().unwrap();
        let p = create_paper(&mut c, &Identifiers::default(), Some("T")).unwrap();
        let chat = FakeChat::answering(&["x"]);
        let (r, _) = run(&mut c, &p, &chat, Some(100_000), true, "Q?");
        assert!(matches!(r, Err(AskError::NoFullText)));
        assert_eq!(chat.calls(), 0);
    }

    /// A blank question is refused.
    #[test]
    fn blank_question_refused() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::answering(&["x"]);
        let (r, _) = run(&mut c, &p, &chat, Some(100_000), true, "  \n");
        assert!(matches!(r, Err(AskError::EmptyQuestion)));
        assert_eq!(chat.calls(), 0);
    }

    /// The request carries the window as `num_ctx` and the answer settings.
    #[test]
    fn num_ctx_is_sent() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::answering(&["ok"]);
        run(&mut c, &p, &chat, Some(100_000), true, "Q?").0.unwrap();
        let req = chat.requests.borrow()[0].clone();
        assert_eq!(req.num_ctx, NonZeroU32::new(100_000));
        assert_eq!(req.max_tokens, MIN_REPLY_TOKENS as i64);
        assert_eq!(req.temperature, crate::config::ANSWER_TEMPERATURE);
        assert_eq!(req.model.as_deref(), Some("ollama:m"));
    }

    /// The window is sent saturated; none, or zero, sends nothing.
    #[test]
    fn num_ctx_saturates() {
        assert_eq!(num_ctx(Some(u64::MAX)), NonZeroU32::new(u32::MAX));
        assert_eq!(num_ctx(Some(8192)), NonZeroU32::new(8192));
        assert_eq!(num_ctx(None), None);
        assert_eq!(num_ctx(Some(0)), None);
    }

    /// A paper too long for the window is asked over its passages.
    #[test]
    fn retrieval_when_too_long() {
        let md = long_paper();
        let (mut c, p) = store_with(&md, false);
        let chat = FakeChat::answering(&["Over 75: <q>61 were over 75 years</q>."]);
        let (r, events) = run(&mut c, &p, &chat, Some(10_000), true, "hazard ratio");
        let conv = r.unwrap();
        // Indexing, then one `Waiting` once the model is asked, then its text.
        let waiting = events.iter().position(|e| *e == AnswerEvent::Waiting);
        let last_indexing = events
            .iter()
            .rposition(|e| matches!(e, AnswerEvent::Indexing { .. }));
        assert!(
            last_indexing.is_some() && last_indexing < waiting,
            "{events:?}"
        );
        assert!(matches!(
            events[waiting.unwrap() + 1..],
            [AnswerEvent::Delta { .. }, ..]
        ));
        let req = chat.requests.borrow()[0].clone();
        let whole = crate::prompt::paper_context(&canonical_from_markdown(&md)).unwrap();
        assert!(!req.messages[1].content.contains(&whole));
        assert!(req.messages[1].content.contains("hazard ratio"));
        let t = &turns(&mut c, conv).unwrap()[0];
        assert_eq!(t.mode, Mode::Retrieval);
        assert!(!t.passages.is_empty());
    }

    /// A retrieval prompt over the window is refused before the model is asked.
    #[test]
    fn too_long_refused() {
        let (mut c, p) = store_with(&long_paper(), false);
        let cid = canonical_id(&mut c, &p);
        let conv = crate::conversations::create_conversation(&mut c, &p, cid).unwrap();
        save_turn(&mut c, conv, &long_turn()).unwrap();
        let chat = FakeChat::answering(&["x"]);
        let (r, events) = run(&mut c, &p, &chat, Some(10_000), true, "hazard ratio");
        assert!(!events.contains(&AnswerEvent::Waiting), "{events:?}");
        let e = r.unwrap_err();
        assert!(
            matches!(
                e,
                AskError::TooLong {
                    window: 10_000,
                    has_history: true
                }
            ),
            "{e:?}"
        );
        assert!(e.to_string().contains("(10000 tokens)"));
        assert!(e.to_string().contains("Start a new conversation"));
        assert_eq!(chat.calls(), 0);
        assert_eq!(turns(&mut c, conv).unwrap().len(), 1);
    }

    /// Retrieval needs an embedder.
    #[test]
    fn no_embedder_refused() {
        let (mut c, p) = store_with(&long_paper(), false);
        let chat = FakeChat::answering(&["x"]);
        let (r, _) = run(&mut c, &p, &chat, Some(10_000), false, "hazard ratio");
        assert!(matches!(r, Err(AskError::NeedsEmbedder)));
        assert_eq!(chat.calls(), 0);
    }

    /// An unknown window asks over the passages, checked against the floor
    /// and with no `num_ctx`; a prompt over the floor is refused, naming it.
    #[test]
    fn unknown_window_checks_the_floor() {
        let (mut c, p) = store_with(&long_paper(), false);
        let chat = FakeChat::answering(&["x"]);
        let conv = run(&mut c, &p, &chat, None, true, "hazard ratio")
            .0
            .unwrap();
        assert_eq!(chat.calls(), 1);
        assert_eq!(chat.requests.borrow()[0].num_ctx, None);
        assert_eq!(turns(&mut c, conv).unwrap()[0].mode, Mode::Retrieval);
        save_turn(&mut c, conv, &long_turn()).unwrap();
        let refused = FakeChat::answering(&["x"]);
        let (r, _) = run(&mut c, &p, &refused, None, true, "hazard ratio");
        assert!(
            matches!(r, Err(AskError::TooLong { window, .. }) if window == UNKNOWN_WINDOW_FIT_TOKENS)
        );
        assert_eq!(refused.calls(), 0);
        assert_eq!(turns(&mut c, conv).unwrap().len(), 2);
    }

    /// Stopping keeps what arrived and locates no quotes.
    #[test]
    fn stopped_turn_has_no_citations() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::answering(&["Over 75: <q>61 were over 75 years</q>", " and", "."]);
        let mut embedder = FakeEmbedder;
        let mut ctx = AskContext {
            db: &mut c,
            llm: &chat,
            embedder: Some(&mut embedder),
            model: "m".into(),
            window: Some(100_000),
            reply_tokens: MIN_REPLY_TOKENS,
        };
        let stop = AtomicBool::new(false);
        let conv = ask(&mut ctx, &p, "Q?", &stop, &mut |e| {
            if matches!(e, AnswerEvent::Delta { .. }) {
                stop.store(true, Ordering::SeqCst)
            }
        })
        .unwrap();
        let t = &turns(&mut c, conv).unwrap()[0];
        assert_eq!(t.status, TurnStatus::Stopped);
        assert_eq!(t.answer, "Over 75: <q>61 were over 75 years</q>");
        assert!(t.quotes.is_empty());
    }

    /// A stream that breaks after some text keeps it as a failed turn.
    #[test]
    fn broken_stream_keeps_partial() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::new(
            &["Over 75: ", "<q>61 were"],
            Err(ChatError::Stream("connection reset".into())),
        );
        let conv = run(&mut c, &p, &chat, Some(100_000), true, "Q?").0.unwrap();
        let t = &turns(&mut c, conv).unwrap()[0];
        assert_eq!(t.status, TurnStatus::Failed);
        assert_eq!(t.answer, "Over 75: <q>61 were");
        assert!(t.error.as_deref().unwrap().contains("connection reset"));
        assert!(t.quotes.is_empty());
    }

    /// A failure before any text stores nothing.
    #[test]
    fn failure_before_text_stores_nothing() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::new(&[], Err(ChatError::Transport("refused".into())));
        let (r, _) = run(&mut c, &p, &chat, Some(100_000), true, "Q?");
        assert!(matches!(r, Err(AskError::Chat(ChatError::Transport(_)))));
        let cid = canonical_id(&mut c, &p);
        assert_eq!(current_conversation(&mut c, &p, cid).unwrap(), None);
    }

    /// A follow-up's request carries the first turn, in the same conversation.
    #[test]
    fn follow_up_sees_history() {
        let (mut c, p) = store_with(&short_paper(), false);
        let first = FakeChat::answering(&["First answer."]);
        let a = run(&mut c, &p, &first, Some(100_000), true, "First question?")
            .0
            .unwrap();
        let second = FakeChat::answering(&["Second."]);
        let b = run(&mut c, &p, &second, Some(100_000), true, "And then?")
            .0
            .unwrap();
        assert_eq!(a, b);
        let req = second.requests.borrow()[0].clone();
        let contents: Vec<&str> = req.messages.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.contains(&"First question?"));
        assert!(contents.contains(&"First answer."));
        assert_eq!(contents.last(), Some(&"And then?"));
        assert_eq!(turns(&mut c, b).unwrap().len(), 2);
    }

    /// Cancelling while indexing stores nothing.
    #[test]
    fn cancel_while_indexing() {
        let (mut c, p) = store_with(&long_paper(), false);
        let chat = FakeChat::answering(&["x"]);
        let mut embedder = FakeEmbedder;
        let mut ctx = AskContext {
            db: &mut c,
            llm: &chat,
            embedder: Some(&mut embedder),
            model: "m".into(),
            window: Some(10_000),
            reply_tokens: MIN_REPLY_TOKENS,
        };
        let stop = AtomicBool::new(false);
        let r = ask(&mut ctx, &p, "hazard ratio", &stop, &mut |e| {
            if matches!(e, AnswerEvent::Indexing { .. }) {
                stop.store(true, Ordering::SeqCst)
            }
        });
        assert!(matches!(r, Err(AskError::Cancelled)));
        assert_eq!(chat.calls(), 0);
        let cid = canonical_id(&mut c, &p);
        assert_eq!(current_conversation(&mut c, &p, cid).unwrap(), None);
    }

    /// Passages that do not fit even without history point to Settings, not
    /// to a new conversation.
    #[test]
    fn passages_too_long_without_history() {
        let (mut c, p) = store_with(&long_paper(), false);
        let chat = FakeChat::answering(&["x"]);
        let (r, _) = run(&mut c, &p, &chat, Some(5_000), true, "hazard ratio");
        let e = r.unwrap_err();
        assert!(
            matches!(
                e,
                AskError::TooLong {
                    window: 5_000,
                    has_history: false
                }
            ),
            "{e:?}"
        );
        let text = e.to_string();
        assert!(text.contains("(5000 tokens)") && text.contains("Settings"));
        assert!(!text.contains("new conversation"));
        assert_eq!(chat.calls(), 0);
    }

    /// The embedder message does not claim the paper is too long.
    #[test]
    fn needs_embedder_wording() {
        let text = AskError::NeedsEmbedder.to_string();
        assert!(!text.contains("too long"));
        assert!(text.contains("search model"));
    }

    /// A stream error before any text is an error, and nothing is stored.
    #[test]
    fn stream_error_before_text_stores_nothing() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::new(&[], Err(ChatError::Stream("closed".into())));
        let (r, _) = run(&mut c, &p, &chat, Some(100_000), true, "Q?");
        assert!(matches!(r, Err(AskError::Chat(ChatError::Stream(_)))));
        let cid = canonical_id(&mut c, &p);
        assert_eq!(current_conversation(&mut c, &p, cid).unwrap(), None);
    }

    /// Stop pressed between deltas: the stored answer is what was shown, not
    /// the delta that arrived after the stop.
    #[test]
    fn stopped_answer_is_what_was_shown() {
        let (mut c, p) = store_with(&short_paper(), false);
        let stop = AtomicBool::new(false);
        let mut chat = FakeChat::answering(&["One ", "two ", "three."]);
        chat.stop_before = Some((1, &stop));
        let mut embedder = FakeEmbedder;
        let mut ctx = AskContext {
            db: &mut c,
            llm: &chat,
            embedder: Some(&mut embedder),
            model: "m".into(),
            window: Some(100_000),
            reply_tokens: MIN_REPLY_TOKENS,
        };
        let mut shown = String::new();
        let conv = ask(&mut ctx, &p, "Q?", &stop, &mut |e| {
            if let AnswerEvent::Delta { text } = e {
                shown.push_str(&text)
            }
        })
        .unwrap();
        let t = &turns(&mut c, conv).unwrap()[0];
        assert_eq!(t.status, TurnStatus::Stopped);
        assert_eq!(shown, "One ");
        assert_eq!(t.answer, shown);
    }

    /// An answer the model cut at the token limit (Anthropic's `max_tokens`,
    /// OpenAI's and Ollama's `length`) is complete with its closed quotes
    /// located, and carries the notice that it was cut off.
    #[test]
    fn length_cut_answer_says_so() {
        for reason in LENGTH_STOP_REASONS {
            let (mut c, p) = store_with(&short_paper(), false);
            let text = "Over 75: <q>61 were over 75 years</q> and then";
            let mut r = LLMResponse::new(text);
            r.stop_reason = Some(reason.into());
            let chat = FakeChat::new(&[text], Ok(r));
            let conv = run(&mut c, &p, &chat, Some(100_000), true, "Q?").0.unwrap();
            let t = &turns(&mut c, conv).unwrap()[0];
            assert_eq!(t.status, TurnStatus::Complete, "{reason}");
            assert_eq!(
                t.error.as_deref(),
                Some("The answer reached the length limit (4096 tokens) and was cut off."),
                "{reason}"
            );
            assert!(t.quotes[0].span.is_some(), "{reason}");
        }
    }

    /// The context's reply room is the request's `max_tokens`, and a cut-off
    /// answer's notice names it.
    #[test]
    fn reply_room_is_sent_and_named() {
        let (mut c, p) = store_with(&short_paper(), false);
        let text = "Cut <q>61 were over 75 years</q> and";
        let mut r = LLMResponse::new(text);
        r.stop_reason = Some("length".into());
        let chat = FakeChat::new(&[text], Ok(r));
        let mut embedder = FakeEmbedder;
        let mut ctx = AskContext {
            db: &mut c,
            llm: &chat,
            embedder: Some(&mut embedder),
            model: "deepseek:m".into(),
            window: Some(1_000_000),
            reply_tokens: 32_768,
        };
        let stop = AtomicBool::new(false);
        let conv = ask(&mut ctx, &p, "Q?", &stop, &mut |_| {}).unwrap();
        assert_eq!(chat.requests.borrow()[0].max_tokens, 32_768);
        let t = &turns(&mut c, conv).unwrap()[0];
        assert_eq!(
            t.error.as_deref(),
            Some("The answer reached the length limit (32768 tokens) and was cut off.")
        );
    }

    /// A model's own end (`stop`, say) carries no notice.
    #[test]
    fn normal_end_has_no_notice() {
        let (mut c, p) = store_with(&short_paper(), false);
        let mut r = LLMResponse::new("Done.");
        r.stop_reason = Some("end_turn".into());
        let chat = FakeChat::new(&["Done."], Ok(r));
        let conv = run(&mut c, &p, &chat, Some(100_000), true, "Q?").0.unwrap();
        assert_eq!(turns(&mut c, conv).unwrap()[0].error, None);
    }

    /// Stop pressed before any text was shown (before the first delta, or
    /// after only thinking): cancelled, nothing stored.
    #[test]
    fn stop_before_any_text_stores_nothing() {
        for at in [0, 1] {
            let (mut c, p) = store_with(&short_paper(), false);
            let stop = AtomicBool::new(false);
            let mut chat = FakeChat::scripted(
                vec![
                    StreamDelta::Thinking("pondering".into()),
                    StreamDelta::Text("Answer.".into()),
                ],
                Ok(LLMResponse::new("Answer.")),
            );
            chat.stop_before = Some((at, &stop));
            let mut embedder = FakeEmbedder;
            let mut ctx = AskContext {
                db: &mut c,
                llm: &chat,
                embedder: Some(&mut embedder),
                model: "m".into(),
                window: Some(100_000),
                reply_tokens: MIN_REPLY_TOKENS,
            };
            let r = ask(&mut ctx, &p, "Q?", &stop, &mut |_| {});
            assert!(matches!(r, Err(AskError::Cancelled)), "stop at {at}: {r:?}");
            let cid = canonical_id(&mut c, &p);
            assert_eq!(current_conversation(&mut c, &p, cid).unwrap(), None);
        }
    }

    /// A thinking delta is neither shown nor stored.
    #[test]
    fn thinking_is_dropped() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::scripted(
            vec![
                StreamDelta::Thinking("pondering".into()),
                StreamDelta::Text("Answer.".into()),
            ],
            Ok(LLMResponse::new("Answer.")),
        );
        let (r, events) = run(&mut c, &p, &chat, Some(100_000), true, "Q?");
        assert_eq!(
            events,
            vec![
                AnswerEvent::Waiting,
                AnswerEvent::Delta {
                    text: "Answer.".into()
                }
            ]
        );
        let t = &turns(&mut c, r.unwrap()).unwrap()[0];
        assert!(!t.answer.contains("pondering"));
    }

    /// A stop already set before the call: cancelled, the model is not asked.
    #[test]
    fn stop_before_the_call_cancels() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::answering(&["x"]);
        let mut embedder = FakeEmbedder;
        let mut ctx = AskContext {
            db: &mut c,
            llm: &chat,
            embedder: Some(&mut embedder),
            model: "m".into(),
            window: Some(100_000),
            reply_tokens: MIN_REPLY_TOKENS,
        };
        let stop = AtomicBool::new(true);
        let r = ask(&mut ctx, &p, "Q?", &stop, &mut |_| {});
        assert!(matches!(r, Err(AskError::Cancelled)));
        assert_eq!(chat.calls(), 0);
        let cid = canonical_id(&mut c, &p);
        assert_eq!(current_conversation(&mut c, &p, cid).unwrap(), None);
    }

    /// A stop while the model still reads the prompt (no text yet, #635):
    /// cancelled, nothing stored.
    #[test]
    fn stop_while_waiting_cancels() {
        let (mut c, p) = store_with(&short_paper(), false);
        let chat = FakeChat::answering(&["x"]);
        let mut embedder = FakeEmbedder;
        let mut ctx = AskContext {
            db: &mut c,
            llm: &chat,
            embedder: Some(&mut embedder),
            model: "m".into(),
            window: Some(100_000),
            reply_tokens: MIN_REPLY_TOKENS,
        };
        let stop = AtomicBool::new(false);
        let r = ask(&mut ctx, &p, "Q?", &stop, &mut |e| {
            if e == AnswerEvent::Waiting {
                stop.store(true, Ordering::SeqCst);
            }
        });
        assert!(matches!(r, Err(AskError::Cancelled)), "{r:?}");
        assert_eq!(chat.calls(), 0);
        let cid = canonical_id(&mut c, &p);
        assert_eq!(current_conversation(&mut c, &p, cid).unwrap(), None);
    }

    /// Events serialise with a `kind` tag.
    #[test]
    fn events_are_tagged() {
        let e = serde_json::to_value(AnswerEvent::Indexing { done: 1, total: 2 }).unwrap();
        assert_eq!(
            e,
            serde_json::json!({"kind": "indexing", "done": 1, "total": 2})
        );
        let d = serde_json::to_value(AnswerEvent::Delta { text: "a".into() }).unwrap();
        assert_eq!(d, serde_json::json!({"kind": "delta", "text": "a"}));
        let m = serde_json::to_value(AnswerEvent::CheckingModel).unwrap();
        assert_eq!(m, serde_json::json!({"kind": "checking_model"}));
        let w = serde_json::to_value(AnswerEvent::Waiting).unwrap();
        assert_eq!(w, serde_json::json!({"kind": "waiting"}));
    }
}
