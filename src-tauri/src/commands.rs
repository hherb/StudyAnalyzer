//! The Tauri commands: thin wrappers that run core functions on a blocking
//! thread and map their errors to [`CommandError`].

use std::path::Path;

use studyanalyzer_core::config::{
    CONTACT_EMAIL_KEY, MAX_PDF_BYTES, RETRIEVAL_ATTEMPTS, RETRY_BASE_DELAY,
};
use studyanalyzer_core::intake::{IntakeError, SourceKind};
use studyanalyzer_core::open::{self, OpenContext, OpenOutcome, OpenRequest};
use studyanalyzer_core::retrieval::build_service;
use studyanalyzer_core::store::{self, PaperId, PaperSummary};
use studyanalyzer_core::view::{self, PaperView};
use tauri::{AppHandle, Manager};

use crate::error::{check_contact_email, CommandError};
use crate::state::AppState;

/// Runs `work` on a blocking thread with the managed state.
async fn blocking<T, F>(app: AppHandle, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&AppState) -> Result<T, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || work(&app.state::<AppState>()))
        .await
        .map_err(|e| CommandError::internal(format!("background task failed: {e}")))?
}

/// Reads a file the user chose: refuses anything over the largest cap before
/// reading, and returns the file name only (never the path) with the bytes.
fn read_chosen_file(path: &str) -> Result<(String, Vec<u8>), CommandError> {
    let path = Path::new(path);
    let io =
        |e: std::io::Error| CommandError::invalid_input(format!("The file could not be read: {e}"));
    let len = std::fs::metadata(path).map_err(io)?.len();
    if len > MAX_PDF_BYTES as u64 {
        return Err(IntakeError::TooLarge {
            kind: SourceKind::Pdf,
            len: usize::try_from(len).unwrap_or(usize::MAX),
            max: MAX_PDF_BYTES,
        }
        .into());
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((name, std::fs::read(path).map_err(io)?))
}

/// Runs an open-family operation with a full [`OpenContext`].
fn with_context<T>(
    state: &AppState,
    email: Option<&str>,
    f: impl FnOnce(&mut OpenContext) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    let service = build_service(state.fulltext_dir(), email.unwrap_or_default());
    let pdf = state.pdf_extractor();
    let scratch = state.scratch_dir();
    let mut db = lock(state)?;
    let mut ctx = OpenContext {
        db: &mut *db,
        source: &service,
        pdf: &pdf,
        scratch_dir: &scratch,
        attempts: RETRIEVAL_ATTEMPTS,
        base_delay: RETRY_BASE_DELAY,
    };
    f(&mut ctx)
}

/// Locks the database; a poisoned lock is an internal error.
fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, CommandError> {
    state
        .db
        .lock()
        .map_err(|_| CommandError::internal("the database lock was poisoned"))
}

/// Opens a file the user chose (dialog or drop).
#[tauri::command]
pub async fn open_file(app: AppHandle, path: String) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        let (name, bytes) = read_chosen_file(&path)?;
        let req = OpenRequest::File {
            name: Some(name),
            bytes,
        };
        with_context(s, None, |ctx| Ok(open::open(ctx, req)?))
    })
    .await
}

/// Opens pasted text.
#[tauri::command]
pub async fn open_paste(app: AppHandle, text: String) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        with_context(s, None, |ctx| {
            Ok(open::open(ctx, OpenRequest::Paste { text })?)
        })
    })
    .await
}

/// Opens a PMID, DOI or PMC ID; needs the stored contact email.
#[tauri::command]
pub async fn open_identifier(app: AppHandle, input: String) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        let email = store::get_setting(&mut *lock(s)?, CONTACT_EMAIL_KEY)?
            .ok_or_else(CommandError::needs_email)?;
        with_context(s, Some(&email), |ctx| {
            Ok(open::open(ctx, OpenRequest::Identifier { input })?)
        })
    })
    .await
}

/// Adds a file the user chose to an existing paper.
#[tauri::command]
pub async fn attach_file(
    app: AppHandle,
    paper_id: String,
    path: String,
) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        let (name, bytes) = read_chosen_file(&path)?;
        let req = OpenRequest::File {
            name: Some(name),
            bytes,
        };
        with_context(s, None, |ctx| Ok(open::attach(ctx, &paper_id, req)?))
    })
    .await
}

/// Adds pasted text to an existing paper.
#[tauri::command]
pub async fn attach_paste(
    app: AppHandle,
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
pub async fn reopen_paper(app: AppHandle, paper_id: String) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        with_context(s, None, |ctx| Ok(open::reopen(ctx, &paper_id)?))
    })
    .await
}

/// The papers opened so far, most recent first.
#[tauri::command]
pub async fn recent_papers(app: AppHandle) -> Result<Vec<PaperSummary>, CommandError> {
    blocking(app, |s| Ok(store::recent_papers(&mut *lock(s)?)?)).await
}

/// The paper's canonical text as the frontend renders it.
#[tauri::command]
pub async fn paper_view(app: AppHandle, paper_id: String) -> Result<PaperView, CommandError> {
    blocking(app, move |s| {
        let mut db = lock(s)?;
        let id = PaperId(paper_id);
        let canonical = store::latest_canonical(&mut *db, &id)?.ok_or_else(|| {
            CommandError::invalid_input("This paper has no full text to show yet.")
        })?;
        let title = store::paper_title(&mut *db, &id)?;
        let source = store::source_by_id(&mut *db, canonical.source_id)?;
        Ok(view::paper_view(
            &id.0,
            &title,
            &canonical,
            source.decoded.as_ref(),
        )?)
    })
    .await
}

/// Records that the user confirmed the paper's latest text is the whole paper.
#[tauri::command]
pub async fn confirm_full_text(app: AppHandle, paper_id: String) -> Result<(), CommandError> {
    blocking(app, move |s| {
        let mut db = lock(s)?;
        let canonical =
            store::latest_canonical(&mut *db, &PaperId(paper_id))?.ok_or_else(|| {
                CommandError::invalid_input("This paper has no full text to confirm.")
            })?;
        Ok(store::confirm_full_text(&mut *db, canonical.id)?)
    })
    .await
}

/// The stored contact email, if any.
#[tauri::command]
pub async fn get_contact_email(app: AppHandle) -> Result<Option<String>, CommandError> {
    blocking(app, |s| {
        Ok(store::get_setting(&mut *lock(s)?, CONTACT_EMAIL_KEY)?)
    })
    .await
}

/// Stores the contact email after checking its shape.
#[tauri::command]
pub async fn set_contact_email(app: AppHandle, email: String) -> Result<(), CommandError> {
    blocking(app, move |s| {
        let email = check_contact_email(&email)?;
        Ok(store::set_setting(
            &mut *lock(s)?,
            CONTACT_EMAIL_KEY,
            &email,
        )?)
    })
    .await
}
