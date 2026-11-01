//! The Tauri commands: thin wrappers that run core functions on a blocking
//! thread and map their errors to [`CommandError`].

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use bmlib::fulltext::mupdf_backend::MupdfExtractor;
use studyanalyzer_core::config::{
    BYTES_PER_MB, CONTACT_EMAIL_KEY, KEEP_MISMATCHED_KEY, MAX_PDF_BYTES, RETRIEVAL_ATTEMPTS,
    RETRY_BASE_DELAY,
};
use studyanalyzer_core::limit::Limit;
use studyanalyzer_core::open::{self, OpenContext, OpenError, OpenOutcome, OpenRequest};
use studyanalyzer_core::retrieval::build_service;
use studyanalyzer_core::store::{self, PaperId, PaperSummary};
use studyanalyzer_core::view::{self, PaperView};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, Runtime};

use crate::error::{check_contact_email, CommandError, ErrorKind};
use crate::state::AppState;

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

#[cfg(test)]
mod tests {
    use super::*;

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
