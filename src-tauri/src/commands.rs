//! The Tauri commands: thin wrappers that run core functions on a blocking
//! thread and map their errors to [`CommandError`].

use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bmlib::fulltext::mupdf_backend::MupdfExtractor;
use bmlib::llm::LlmClient;
use serde::Serialize;
use studyanalyzer_core::config::{
    BYTES_PER_MB, CONTACT_EMAIL_KEY, EMBEDDING_MODEL, KEEP_MISMATCHED_KEY, MAX_PDF_BYTES,
    NO_FULL_TEXT_YET, RETRIEVAL_ATTEMPTS, RETRY_BASE_DELAY,
};
use studyanalyzer_core::conversations;
use studyanalyzer_core::interrogate::{self, AnswerEvent, Asker};
use studyanalyzer_core::limit::Limit;
use studyanalyzer_core::llm_settings::{self, LlmSettings};
use studyanalyzer_core::open::{self, OpenContext, OpenError, OpenOutcome, OpenRequest};
use studyanalyzer_core::pico::{self, PicoContext};
use studyanalyzer_core::prompt::running_window;
use studyanalyzer_core::retrieval::build_service;
use studyanalyzer_core::store::{self, PaperId, PaperSummary, StoreError};
use studyanalyzer_core::view::{
    self, ConversationView, InterrogatorView, PaperView, PicoRunView, PicoView,
};
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, Manager, Runtime};

use crate::error::{check_contact_email, CommandError, ErrorKind};
use crate::state::{lock, AppState};

/// Runs `work` on a blocking thread with the managed state; `internal` when
/// the state is missing (a failed startup).
async fn blocking<R: Runtime, T, F>(app: AppHandle<R>, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&AppState) -> Result<T, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || match app.try_state::<AppState>() {
        Some(state) => work(&state),
        None => Err(CommandError::internal("the app did not start")),
    })
    .await
    .map_err(|e| CommandError::internal(format!("background task failed: {e}")))?
}

/// The stored paper with the id the frontend sent, or `invalid_input`.
fn known_paper(db: &mut dyn bmlib::db::Db, id: &str) -> Result<PaperId, CommandError> {
    store::paper_by_id(db, id)?
        .ok_or_else(|| CommandError::from(OpenError::UnknownPaper(id.to_string())))
}

/// Reads a file the user chose: refuses a folder and anything over the
/// largest cap, before reading and (for a file that grew meanwhile) while
/// reading, and returns the file name only (never the
/// path) with the bytes. A file that is gone is `invalid_input` (the user can
/// choose again); any other read failure (permission, disk) is `internal`.
fn read_chosen_file(path: &str) -> Result<(Option<String>, Vec<u8>), CommandError> {
    let path = Path::new(path);
    let io = |e: std::io::Error| {
        let message = format!("The file could not be read: {e}");
        match e.kind() {
            std::io::ErrorKind::NotFound => CommandError::invalid_input(message),
            _ => CommandError::internal(message),
        }
    };
    let meta = std::fs::metadata(path).map_err(io)?;
    if meta.is_dir() {
        return Err(CommandError::invalid_input(
            "That is a folder: choose a file in it instead.",
        ));
    }
    // Not yet read, so the format is unknown: the message names none.
    let too_large = |size: String| {
        CommandError::new(
            ErrorKind::TooLarge,
            format!(
                "This file is {size}; the largest file StudyAnalyzer opens is {:.1} MB.",
                MAX_PDF_BYTES as f64 / BYTES_PER_MB
            ),
        )
    };
    if meta.len() > MAX_PDF_BYTES as u64 {
        return Err(too_large(format!(
            "{:.1} MB",
            meta.len() as f64 / BYTES_PER_MB
        )));
    }
    // A file that grew since `metadata` is refused here, not read whole.
    let bytes = std::fs::File::open(path)
        .and_then(|f| read_capped(f, MAX_PDF_BYTES))
        .map_err(io)?
        .ok_or_else(|| too_large("larger than that now (it grew while being read)".into()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    Ok((name, bytes))
}

/// Reads `reader` to its end, or `None` once it holds more than `cap` bytes
/// (it reads at most one byte past the cap).
fn read_capped(reader: impl Read, cap: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    reader.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() <= cap).then_some(bytes))
}

/// Reads a chosen file as an open request.
fn read_for_open(path: &str) -> Result<OpenRequest, CommandError> {
    let (name, bytes) = read_chosen_file(path)?;
    Ok(OpenRequest::File { name, bytes })
}

/// Runs an open-family operation with a full [`OpenContext`]. `lookup` is the
/// contact email and time limit of a lookup by identifier; commands that make
/// no network request pass `None`.
fn with_context<T>(
    state: &AppState,
    lookup: Option<(&str, &Limit)>,
    f: impl FnOnce(&mut OpenContext) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    let unused;
    let (email, limit) = match lookup {
        Some(lookup) => lookup,
        None => {
            // These commands never reach `resolve` or `retrieve`; a spent limit makes a
            // stray request fail at once rather than run uncancellable.
            unused = Limit::new(Duration::ZERO);
            ("", &unused)
        }
    };
    let service = build_service(state.fulltext_dir(), email, limit.clone());
    let pdf = MupdfExtractor::new();
    let scratch = state.scratch_dir();
    let mut db = state.connect()?;
    let keep_mismatched = store::get_flag(&mut db, KEEP_MISMATCHED_KEY)?;
    let mut ctx = OpenContext {
        db: &mut db,
        source: &service,
        pdf: &pdf,
        scratch_dir: &scratch,
        attempts: RETRIEVAL_ATTEMPTS,
        base_delay: RETRY_BASE_DELAY,
        limit,
        keep_mismatched,
    };
    f(&mut ctx)
}

/// Opens a file the user chose (dialog or drop).
#[tauri::command]
pub async fn open_file<R: Runtime>(
    app: AppHandle<R>,
    path: String,
) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        let req = read_for_open(&path)?;
        with_context(s, None, |ctx| Ok(open::open(ctx, req)?))
    })
    .await
}

/// Opens pasted text.
#[tauri::command]
pub async fn open_paste<R: Runtime>(
    app: AppHandle<R>,
    text: String,
) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        with_context(s, None, |ctx| {
            Ok(open::open(ctx, OpenRequest::Paste { text })?)
        })
    })
    .await
}

/// Opens a PMID, DOI or PMC ID; needs the stored contact email, even when the
/// paper is already stored and would reopen offline. The lookup gives up after
/// [`RETRIEVAL_DEADLINE`](studyanalyzer_core::config::RETRIEVAL_DEADLINE)
/// and stops early on [`cancel_lookup`] (`cancelled`).
#[tauri::command]
pub async fn open_identifier<R: Runtime>(
    app: AppHandle<R>,
    input: String,
) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        // Registered before anything else, so a Cancel pressed at once is not lost.
        let limit = s.begin_lookup();
        let result = (|| -> Result<OpenOutcome, CommandError> {
            let email = store::get_setting(&mut s.connect()?, CONTACT_EMAIL_KEY)?
                .ok_or_else(CommandError::needs_email)?;
            with_context(s, Some((&email, &limit)), |ctx| {
                Ok(open::open(ctx, OpenRequest::Identifier { input })?)
            })
        })();
        s.end_lookup(&limit);
        result
    })
    .await
}

/// Cancels the running lookup by identifier, if there is one. It ends with a
/// `cancelled` error once its current request (or backoff wait) is over,
/// unless it was reopening a stored paper or already saving the result.
/// Logs and does nothing else when the app state is missing (a failed startup).
#[tauri::command]
pub fn cancel_lookup<R: Runtime>(app: AppHandle<R>) {
    match app.try_state::<AppState>() {
        Some(s) if !s.cancel_lookup() => log::info!("cancel pressed with no lookup running"),
        Some(_) => {}
        None => log::error!("cancel pressed but the app state is missing"),
    }
}

/// Adds a file the user chose to an existing paper.
#[tauri::command]
pub async fn attach_file<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
    path: String,
) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        let req = read_for_open(&path)?;
        with_context(s, None, |ctx| Ok(open::attach(ctx, &paper_id, req)?))
    })
    .await
}

/// Adds pasted text to an existing paper.
#[tauri::command]
pub async fn attach_paste<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
    text: String,
) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        let req = OpenRequest::Paste { text };
        with_context(s, None, |ctx| Ok(open::attach(ctx, &paper_id, req)?))
    })
    .await
}

/// Reopens a stored paper (offline).
#[tauri::command]
pub async fn reopen_paper<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        with_context(s, None, |ctx| Ok(open::reopen(ctx, &paper_id)?))
    })
    .await
}

/// The papers opened so far, most recent first.
#[tauri::command]
pub async fn recent_papers<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<PaperSummary>, CommandError> {
    blocking(app, |s| Ok(store::recent_papers(&mut s.connect()?)?)).await
}

/// The paper's canonical text as the frontend renders it.
#[tauri::command]
pub async fn paper_view<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
) -> Result<PaperView, CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let id = known_paper(&mut db, &paper_id)?;
        let canonical = store::latest_canonical(&mut db, &id)?.ok_or_else(|| {
            CommandError::invalid_input("This paper has no full text to show yet.")
        })?;
        let title = store::paper_title(&mut db, &id)?;
        let (kind, decoded) = store::source_info(&mut db, canonical.source_id)?;
        Ok(view::paper_view(
            id.as_str(),
            &title,
            &canonical,
            kind,
            decoded.as_ref(),
        )?)
    })
    .await
}

/// The PDF the paper's text was derived from, as raw bytes (not a JSON array),
/// for the Original tab.
#[tauri::command]
pub async fn original_pdf<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
) -> Result<Response, CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let id = known_paper(&mut db, &paper_id)?;
        let bytes = store::original_pdf(&mut db, &id)?.ok_or_else(|| {
            CommandError::invalid_input("This paper's text was not derived from a PDF.")
        })?;
        Ok(Response::new(bytes))
    })
    .await
}

/// Records that the user confirmed the text they saw (`canonical_id`, from
/// its `PaperView`) is the whole paper. A text a re-derive stored since stays
/// unconfirmed, so its banner shows again. A `canonical_id` that is not one
/// of this paper's texts (a stale view) is `invalid_input`; the frontend then reloads the view.
#[tauri::command]
pub async fn confirm_full_text<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
    canonical_id: i64,
) -> Result<(), CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let id = known_paper(&mut db, &paper_id)?;
        store::confirm_full_text(&mut db, &id, canonical_id).map_err(|e| match e {
            store::StoreError::NotFound(what) => {
                log::warn!("confirm of {what} refused: the view is stale");
                CommandError::invalid_input(
                    "The text shown had changed since it was loaded. Check the text now shown and confirm again.",
                )
            }
            e => e.into(),
        })
    })
    .await
}

/// The stored contact email, if any.
#[tauri::command]
pub async fn get_contact_email<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<String>, CommandError> {
    blocking(app, |s| {
        Ok(store::get_setting(&mut s.connect()?, CONTACT_EMAIL_KEY)?)
    })
    .await
}

/// Stores the contact email after checking its shape.
#[tauri::command]
pub async fn set_contact_email<R: Runtime>(
    app: AppHandle<R>,
    email: String,
) -> Result<(), CommandError> {
    blocking(app, move |s| {
        let email = check_contact_email(&email)?;
        Ok(store::set_setting(
            &mut s.connect()?,
            CONTACT_EMAIL_KEY,
            &email,
        )?)
    })
    .await
}

/// Whether a mismatching document opens as a paper of its own (a debugging setting).
#[tauri::command]
pub async fn get_keep_mismatched<R: Runtime>(app: AppHandle<R>) -> Result<bool, CommandError> {
    blocking(app, |s| {
        Ok(store::get_flag(&mut s.connect()?, KEEP_MISMATCHED_KEY)?)
    })
    .await
}

/// Turns the debugging setting [`get_keep_mismatched`] reads on or off.
#[tauri::command]
pub async fn set_keep_mismatched<R: Runtime>(
    app: AppHandle<R>,
    keep: bool,
) -> Result<(), CommandError> {
    blocking(app, move |s| {
        Ok(store::set_flag(
            &mut s.connect()?,
            KEEP_MISMATCHED_KEY,
            keep,
        )?)
    })
    .await
}

/// A `NotFound` from the store as `invalid_input` with `message` (the
/// frontend sent an id that names nothing, or a stale one); other failures as they are.
fn not_found_is_input(e: StoreError, message: &str) -> CommandError {
    match e {
        StoreError::NotFound(what) => {
            log::warn!("{what} not found");
            CommandError::invalid_input(message)
        }
        e => e.into(),
    }
}

/// `model`'s context window as its provider states it (`None`: it states
/// none), asked once per launch and then remembered. A failure is an error,
/// never a guess, and is asked again next time.
fn cached_window(
    s: &AppState,
    client: &LlmClient,
    model: &str,
) -> Result<Option<u64>, CommandError> {
    if let Some(window) = lock(&s.windows).get(model) {
        return Ok(*window);
    }
    // Not locked while the provider is asked: a slow one must not block others.
    let window = client
        .context_window(model)
        .map_err(|e| CommandError::from_window(e, model))?;
    lock(&s.windows).insert(model.to_string(), window);
    Ok(window)
}

/// Asks `question` of the paper `paper_id` with the chosen model, streaming
/// events to `on_event`; returns the conversation the turn went into. The
/// model and the paper's text are checked before the provider is reached, so
/// a paper without full text is refused as such even offline or keyless.
fn ask_with(
    s: &AppState,
    paper_id: &str,
    question: &str,
    stop: &AtomicBool,
    on_event: &Channel<AnswerEvent>,
) -> Result<ConversationView, CommandError> {
    let mut db = s.connect()?;
    let paper = known_paper(&mut db, paper_id)?;
    // Never a chat error, so no provider to name.
    let model =
        interrogate::ready_to_ask(&mut db, &paper).map_err(|e| CommandError::from_ask(e, ""))?;
    let (provider, client) = llm_settings::llm_client(&model, s.secrets.as_ref())?;
    let maximum = cached_window(s, &client, &model)?;
    let window = running_window(llm_settings::context_tokens(&mut db)?, maximum);
    let mut send = |event: AnswerEvent| {
        if let Err(e) = on_event.send(event) {
            log::warn!("an answer event could not reach the window: {e}");
        }
    };
    let mut asker = Asker {
        db: &mut db,
        llm: &client,
        embedder: &s.embedder,
        model,
        window,
    };
    let id = interrogate::ask_loading_embedder(&mut asker, &paper, question, stop, &mut send)
        .map_err(|e| CommandError::from_ask(e, &provider))?;
    Ok(view::conversation_view(&mut db, id)?)
}

/// Asks a question of the paper with the model chosen in Settings. The paper
/// id is checked first, then that a model is chosen (none: `needs_model`),
/// then that the paper has full text. Indexing progress and the answer's text
/// stream to `on_event`; [`stop_answer`] stops it (a stopped answer is kept as
/// a `stopped` turn; a stop before any text stores nothing and is
/// `cancelled`). Returns the conversation with the new turn.
#[tauri::command]
pub async fn ask<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
    question: String,
    on_event: Channel<AnswerEvent>,
) -> Result<ConversationView, CommandError> {
    // Registered before the blocking task is queued, so a Stop pressed at once is not lost.
    let stop = app.try_state::<AppState>().map(|s| s.answer_stop.begin());
    blocking(app, move |s| {
        let stop = stop.ok_or_else(|| CommandError::internal("the app did not start"))?;
        let result = ask_with(s, &paper_id, &question, &stop, &on_event);
        s.answer_stop.end(&stop);
        result
    })
    .await
}

/// Stops the running answer, if there is one. Logs and does nothing else when
/// the app state is missing (a failed startup).
#[tauri::command]
pub fn stop_answer<R: Runtime>(app: AppHandle<R>) {
    match app.try_state::<AppState>() {
        Some(s) if !s.answer_stop.stop() => log::info!("stop pressed with no answer running"),
        Some(_) => {}
        None => log::error!("stop pressed but the app state is missing"),
    }
}

/// Starts a new conversation on the paper's latest text (or returns the
/// current one if it has no turns yet).
#[tauri::command]
pub async fn new_conversation<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
) -> Result<ConversationView, CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let paper = known_paper(&mut db, &paper_id)?;
        let canonical = store::latest_canonical(&mut db, &paper)?
            .ok_or_else(|| CommandError::new(ErrorKind::NotFullText, NO_FULL_TEXT_YET.into()))?;
        let id = match conversations::current_conversation(&mut db, &paper, canonical.id)? {
            Some(id) if conversations::turns(&mut db, id)?.is_empty() => id,
            _ => conversations::create_conversation(&mut db, &paper, canonical.id)?,
        };
        Ok(view::conversation_view(&mut db, id)?)
    })
    .await
}

/// What the Interrogator tab opens with; `not_full_text` for a paper with no text.
#[tauri::command]
pub async fn interrogator<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
) -> Result<InterrogatorView, CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let paper = known_paper(&mut db, &paper_id)?;
        view::interrogator_view(&mut db, &paper).map_err(|e| match e {
            StoreError::NotFound(_) => {
                CommandError::new(ErrorKind::NotFullText, NO_FULL_TEXT_YET.into())
            }
            e => e.into(),
        })
    })
    .await
}

/// One conversation with its turns (an earlier one, from the Interrogator's list).
#[tauri::command]
pub async fn conversation<R: Runtime>(
    app: AppHandle<R>,
    conversation_id: i64,
) -> Result<ConversationView, CommandError> {
    blocking(app, move |s| {
        view::conversation_view(&mut s.connect()?, conversation_id)
            .map_err(|e| not_found_is_input(e, "That conversation is not stored."))
    })
    .await
}

/// The paper view of one of the paper's texts, not necessarily its latest
/// (an earlier conversation's, from `ConversationView.canonical_id`).
#[tauri::command]
pub async fn paper_view_at<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
    canonical_id: i64,
) -> Result<PaperView, CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let paper = known_paper(&mut db, &paper_id)?;
        view::stored_paper_view(&mut db, &paper, canonical_id)
            .map_err(|e| not_found_is_input(e, "That text is not one of this paper's."))
    })
    .await
}

/// What the PICO tab opens with (never runs anything); `not_full_text` for a
/// paper with no text.
#[tauri::command]
pub async fn pico<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
) -> Result<PicoView, CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let paper = known_paper(&mut db, &paper_id)?;
        view::pico_view(&mut db, &paper).map_err(|e| match e {
            StoreError::NotFound(_) => {
                CommandError::new(ErrorKind::NotFullText, NO_FULL_TEXT_YET.into())
            }
            e => e.into(),
        })
    })
    .await
}

/// The stored PICO run `run_id`, or `invalid_input` when there is none.
fn pico_run_with(s: &AppState, run_id: i64) -> Result<PicoRunView, CommandError> {
    view::pico_run_view(&mut s.connect()?, run_id)
        .map_err(|e| not_found_is_input(e, "That PICO run is not stored."))
}

/// One stored PICO run (an earlier one, from the PICO tab's list).
#[tauri::command]
pub async fn pico_run<R: Runtime>(
    app: AppHandle<R>,
    run_id: i64,
) -> Result<PicoRunView, CommandError> {
    blocking(app, move |s| pico_run_with(s, run_id)).await
}

/// Runs PICO on the paper `paper_id` with the chosen model and returns the
/// tab's view. The model, the paper's text and the provider's key are checked
/// before the provider is reached.
fn run_pico_with(
    s: &AppState,
    paper_id: &str,
    force: bool,
    stop: &AtomicBool,
) -> Result<PicoView, CommandError> {
    let mut db = s.connect()?;
    let paper = known_paper(&mut db, paper_id)?;
    // Never a chat error, so no provider to name.
    let model =
        interrogate::ready_to_ask(&mut db, &paper).map_err(|e| CommandError::from_ask(e, ""))?;
    let (provider, client) = llm_settings::llm_client(&model, s.secrets.as_ref())?;
    // bmlib would report a missing key as a failed request, after the paper was read.
    if let Some(env_var) = llm_settings::missing_key(&provider, &client) {
        return Err(CommandError::needs_api_key(&provider, env_var));
    }
    let maximum = cached_window(s, &client, &model)?;
    let window = running_window(llm_settings::context_tokens(&mut db)?, maximum);
    let mut chat = pico::pico_chat(&client, &model, window, stop);
    let condenser = pico::pico_condenser_model(&client, &model, window, stop);
    let mut ctx = PicoContext {
        db: &mut db,
        chat: &mut chat,
        condenser_model: &condenser,
        model,
        window,
        retry_delay: RETRY_BASE_DELAY,
    };
    pico::run_pico(&mut ctx, &paper, force, stop)
        .map_err(|e| CommandError::from_pico(e, &provider))?;
    Ok(view::pico_view(&mut db, &paper)?)
}

/// Runs PICO on the paper with the model chosen in Settings: the suitability
/// check, then the extraction (`force` skips the check). One run at a time: a
/// second is `invalid_input`. [`stop_pico`] stops it before its next model
/// call; a stopped or failed run stores nothing. Returns the tab's view.
#[tauri::command]
pub async fn run_pico<R: Runtime>(
    app: AppHandle<R>,
    paper_id: String,
    force: bool,
) -> Result<PicoView, CommandError> {
    // Claimed before the blocking task is queued, so a Stop pressed at once is not lost.
    let stop = app.try_state::<AppState>().map(|s| s.pico_stop.try_begin());
    blocking(app, move |s| {
        let stop = stop
            .ok_or_else(|| CommandError::internal("the app did not start"))?
            .ok_or_else(|| CommandError::invalid_input("A PICO extraction is already running."))?;
        let result = run_pico_with(s, &paper_id, force, &stop);
        s.pico_stop.end(&stop);
        result
    })
    .await
}

/// Stops the running PICO extraction, if there is one. Logs and does nothing
/// else when the app state is missing (a failed startup).
#[tauri::command]
pub fn stop_pico<R: Runtime>(app: AppHandle<R>) {
    match app.try_state::<AppState>() {
        Some(s) if !s.pico_stop.stop() => log::info!("stop pressed with no PICO run"),
        Some(_) => {}
        None => log::error!("PICO stop pressed but the app state is missing"),
    }
}

/// The model, the context-window limit and each provider's key status (never a key).
#[tauri::command]
pub async fn llm_settings<R: Runtime>(app: AppHandle<R>) -> Result<LlmSettings, CommandError> {
    blocking(app, |s| {
        Ok(llm_settings::llm_settings(
            &mut s.connect()?,
            s.secrets.as_ref(),
        )?)
    })
    .await
}

/// Chooses the `provider:model` that answers.
#[tauri::command]
pub async fn set_model<R: Runtime>(app: AppHandle<R>, model: String) -> Result<(), CommandError> {
    blocking(app, move |s| {
        Ok(llm_settings::set_model(&mut s.connect()?, &model)?)
    })
    .await
}

/// Limits the context window to `tokens`.
#[tauri::command]
pub async fn set_context_tokens<R: Runtime>(
    app: AppHandle<R>,
    tokens: u64,
) -> Result<(), CommandError> {
    blocking(app, move |s| {
        Ok(llm_settings::set_context_tokens(&mut s.connect()?, tokens)?)
    })
    .await
}

/// Removes the context-window limit (the model's own maximum applies).
#[tauri::command]
pub async fn clear_context_tokens<R: Runtime>(app: AppHandle<R>) -> Result<(), CommandError> {
    blocking(app, |s| {
        Ok(llm_settings::clear_context_tokens(&mut s.connect()?)?)
    })
    .await
}

/// Saves `provider`'s API key in the keychain. The key is never logged or returned.
#[tauri::command]
pub async fn set_api_key<R: Runtime>(
    app: AppHandle<R>,
    provider: String,
    key: String,
) -> Result<(), CommandError> {
    blocking(app, move |s| {
        Ok(llm_settings::save_api_key(
            s.secrets.as_ref(),
            &provider,
            &key,
        )?)
    })
    .await
}

/// Removes `provider`'s API key from the keychain (absent is fine).
#[tauri::command]
pub async fn delete_api_key<R: Runtime>(
    app: AppHandle<R>,
    provider: String,
) -> Result<(), CommandError> {
    blocking(app, move |s| {
        Ok(llm_settings::remove_api_key(s.secrets.as_ref(), &provider)?)
    })
    .await
}

/// `model`'s context window as its provider states it, `null` when it states
/// none. A model the provider lacks is `invalid_input` naming it.
#[tauri::command]
pub async fn model_window<R: Runtime>(
    app: AppHandle<R>,
    model: String,
) -> Result<Option<u64>, CommandError> {
    blocking(app, move |s| {
        let model = model.trim();
        let (_, client) = llm_settings::llm_client(model, s.secrets.as_ref())?;
        cached_window(s, &client, model)
    })
    .await
}

/// Whether the search model is on this device, and how large its download is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbedderStatus {
    /// True when it is loaded, or its file is on disk with the pinned size
    /// (the checksum is checked when it is first needed).
    pub downloaded: bool,
    /// The download's size in bytes.
    pub size_bytes: u64,
}

/// How far the search-model download has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DownloadProgress {
    /// Bytes received so far.
    pub done: u64,
    /// Bytes in all.
    pub total: u64,
}

/// Whether the search model is downloaded: cheap, it neither hashes nor loads it.
#[tauri::command]
pub async fn embedder_status<R: Runtime>(
    app: AppHandle<R>,
) -> Result<EmbedderStatus, CommandError> {
    blocking(app, |s| {
        Ok(EmbedderStatus {
            downloaded: s.embedder.downloaded(),
            size_bytes: EMBEDDING_MODEL.size,
        })
    })
    .await
}

/// Downloads the search model (once; 1.1 GB) into app storage and loads it,
/// reporting progress to `on_progress`. [`cancel_download`] stops it
/// (`cancelled`, nothing kept).
#[tauri::command]
pub async fn download_embedder<R: Runtime>(
    app: AppHandle<R>,
    on_progress: Channel<DownloadProgress>,
) -> Result<(), CommandError> {
    // Registered before the blocking task is queued, so a Cancel pressed at once is not lost.
    let stop = app.try_state::<AppState>().map(|s| s.download_stop.begin());
    blocking(app, move |s| {
        let stop = stop.ok_or_else(|| CommandError::internal("the app did not start"))?;
        let mut progress = |done, total| {
            if let Err(e) = on_progress.send(DownloadProgress { done, total }) {
                log::warn!("download progress could not reach the window: {e}");
            }
        };
        let url = s.embedder.model().url();
        let result = s.embedder.download(
            &url,
            &stop,
            RETRIEVAL_ATTEMPTS,
            RETRY_BASE_DELAY,
            &mut progress,
        );
        s.download_stop.end(&stop);
        Ok(result?)
    })
    .await
}

/// Cancels the running search-model download, if there is one. Logs and does
/// nothing else when the app state is missing (a failed startup).
#[tauri::command]
pub fn cancel_download<R: Runtime>(app: AppHandle<R>) {
    match app.try_state::<AppState>() {
        Some(s) if !s.download_stop.stop() => log::info!("cancel pressed with no download running"),
        Some(_) => {}
        None => log::error!("cancel pressed but the app state is missing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state over a fresh temporary directory; returns it and the directory.
    fn temp_state() -> (AppState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sa-state-{}", uuid::Uuid::new_v4()));
        (AppState::for_tests(dir.clone()), dir)
    }

    /// A window learnt this launch is answered without asking the provider
    /// (the client here could reach no server).
    #[test]
    fn cached_window_answers_from_the_cache() {
        let (s, _) = temp_state();
        let (_, client) = llm_settings::llm_client("ollama:m", s.secrets.as_ref()).unwrap();
        lock(&s.windows).insert("ollama:m".into(), Some(4096));
        lock(&s.windows).insert("ollama:none".into(), None);
        assert_eq!(cached_window(&s, &client, "ollama:m").unwrap(), Some(4096));
        assert_eq!(cached_window(&s, &client, "ollama:none").unwrap(), None);
    }

    /// A `NotFound` becomes `invalid_input` with the given message; other failures keep their kind.
    #[test]
    fn not_found_is_the_users_input() {
        let e = not_found_is_input(StoreError::NotFound("conversation 9".into()), "gone");
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::InvalidInput, "gone")
        );
        let e = not_found_is_input(StoreError::Corrupt("x".into()), "gone");
        assert_eq!(e.kind, ErrorKind::Storage);
    }

    /// A PICO run that is not stored is `invalid_input`.
    #[test]
    fn pico_run_not_stored_is_input() {
        let (s, dir) = temp_state();
        std::fs::create_dir_all(&dir).unwrap();
        let e = pico_run_with(&s, 7).unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::InvalidInput, "That PICO run is not stored.")
        );
    }

    /// A known paper id comes back as a `PaperId`; an unknown one is `invalid_input`.
    #[test]
    fn known_paper_checks_the_id() {
        let mut db = store::open_memory_store().unwrap();
        let p = store::create_paper(&mut db, &Default::default(), None).unwrap();
        assert_eq!(known_paper(&mut db, p.as_str()).unwrap(), p);
        let e = known_paper(&mut db, "nope").unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
        assert!(e.message.contains("nope"));
    }

    /// The name is the file name only; a missing file or a folder is `invalid_input`.
    #[test]
    fn read_chosen_file_name_only_and_missing_file() {
        let dir = std::env::temp_dir().join(format!("sa-cmd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("paper.txt");
        std::fs::write(&f, b"hello").unwrap();
        let (name, bytes) = read_chosen_file(f.to_str().unwrap()).unwrap();
        assert_eq!(name.as_deref(), Some("paper.txt"));
        assert_eq!(bytes, b"hello");
        assert_eq!(
            read_chosen_file(dir.join("nope").to_str().unwrap())
                .unwrap_err()
                .kind,
            ErrorKind::InvalidInput
        );
        let folder = read_chosen_file(dir.to_str().unwrap()).unwrap_err();
        assert_eq!(folder.kind, ErrorKind::InvalidInput);
        assert!(folder.message.contains("folder"), "{}", folder.message);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `read_capped` keeps a reader up to the cap and refuses one past it.
    #[test]
    fn read_capped_stops_one_past_the_cap() {
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(read_capped(&b"abcd"[..], 3).unwrap(), None);
    }

    /// A file the app may not read is the app's fault (`internal`), not the user's.
    #[cfg(unix)]
    #[test]
    fn read_chosen_file_unreadable_is_internal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sa-cmd-perm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("locked.txt");
        std::fs::write(&f, b"hello").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o000)).unwrap();
        let e = read_chosen_file(f.to_str().unwrap()).unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(e.kind, ErrorKind::Internal, "{}", e.message);
    }

    /// A sparse file over the cap is `too_large` without being read.
    #[test]
    fn read_chosen_file_refuses_oversize() {
        let dir = std::env::temp_dir().join(format!("sa-cmd-big-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("big.pdf");
        std::fs::File::create(&f)
            .unwrap()
            .set_len(MAX_PDF_BYTES as u64 + 1)
            .unwrap();
        let e = read_chosen_file(f.to_str().unwrap()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::TooLarge);
        assert!(!e.message.contains("PDF"), "{}", e.message);
        assert!(e.message.contains("100.0 MB"), "{}", e.message);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
