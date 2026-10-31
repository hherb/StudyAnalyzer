//! The Tauri commands: thin wrappers that run core functions on a blocking
//! thread and map their errors to [`CommandError`].

use std::path::Path;
use std::time::Duration;

use bmlib::fulltext::mupdf_backend::MupdfExtractor;
use studyanalyzer_core::config::{
    BYTES_PER_MB, CONTACT_EMAIL_KEY, MAX_PDF_BYTES, RETRIEVAL_ATTEMPTS, RETRY_BASE_DELAY,
};
use studyanalyzer_core::limit::Limit;
use studyanalyzer_core::open::{self, OpenContext, OpenError, OpenOutcome, OpenRequest};
use studyanalyzer_core::retrieval::build_service;
use studyanalyzer_core::store::{self, PaperId, PaperSummary};
use studyanalyzer_core::view::{self, PaperView};
use tauri::{AppHandle, Manager};

use crate::error::{check_contact_email, CommandError, ErrorKind};
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

/// The stored paper with the id the frontend sent, or `invalid_input`.
fn known_paper(db: &mut dyn bmlib::db::Db, id: &str) -> Result<PaperId, CommandError> {
    store::paper_by_id(db, id)?
        .ok_or_else(|| CommandError::from(OpenError::UnknownPaper(id.to_string())))
}

/// Reads a file the user chose: refuses a folder and anything over the
/// largest cap before reading, and returns the file name only (never the
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
    let len = meta.len();
    if len > MAX_PDF_BYTES as u64 {
        // Not yet read, so the format is unknown: the message names none.
        return Err(CommandError::new(
            ErrorKind::TooLarge,
            format!(
                "This file is {:.1} MB; the largest file StudyAnalyzer opens is {:.1} MB.",
                len as f64 / BYTES_PER_MB,
                MAX_PDF_BYTES as f64 / BYTES_PER_MB
            ),
        ));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    Ok((name, std::fs::read(path).map_err(io)?))
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
            // These commands never reach `retrieve`; a spent limit makes a
            // stray request fail at once rather than run uncancellable.
            unused = Limit::new(Duration::ZERO);
            ("", &unused)
        }
    };
    let service = build_service(state.fulltext_dir(), email, limit.clone());
    let pdf = MupdfExtractor::new();
    let scratch = state.scratch_dir();
    let mut db = state.connect()?;
    let mut ctx = OpenContext {
        db: &mut db,
        source: &service,
        pdf: &pdf,
        scratch_dir: &scratch,
        attempts: RETRIEVAL_ATTEMPTS,
        base_delay: RETRY_BASE_DELAY,
        limit,
    };
    f(&mut ctx)
}

/// Opens a file the user chose (dialog or drop).
#[tauri::command]
pub async fn open_file(app: AppHandle, path: String) -> Result<OpenOutcome, CommandError> {
    blocking(app, move |s| {
        let req = read_for_open(&path)?;
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

/// Opens a PMID, DOI or PMC ID; needs the stored contact email, even when the
/// paper is already stored and would reopen offline. The lookup gives up after
/// [`RETRIEVAL_DEADLINE`](studyanalyzer_core::config::RETRIEVAL_DEADLINE)
/// and stops early on [`cancel_lookup`] (`cancelled`).
#[tauri::command]
pub async fn open_identifier(app: AppHandle, input: String) -> Result<OpenOutcome, CommandError> {
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
pub fn cancel_lookup(app: AppHandle) {
    match app.try_state::<AppState>() {
        Some(s) if !s.cancel_lookup() => log::info!("cancel pressed with no lookup running"),
        Some(_) => {}
        None => log::error!("cancel pressed but the app state is missing"),
    }
}

/// Adds a file the user chose to an existing paper.
#[tauri::command]
pub async fn attach_file(
    app: AppHandle,
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
    blocking(app, |s| Ok(store::recent_papers(&mut s.connect()?)?)).await
}

/// The paper's canonical text as the frontend renders it.
#[tauri::command]
pub async fn paper_view(app: AppHandle, paper_id: String) -> Result<PaperView, CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let id = known_paper(&mut db, &paper_id)?;
        let canonical = store::latest_canonical(&mut db, &id)?.ok_or_else(|| {
            CommandError::invalid_input("This paper has no full text to show yet.")
        })?;
        let title = store::paper_title(&mut db, &id)?;
        let decoded = store::source_decoding(&mut db, canonical.source_id)?;
        Ok(view::paper_view(
            &id.0,
            &title,
            &canonical,
            decoded.as_ref(),
        )?)
    })
    .await
}

/// Records that the user confirmed the paper's latest text is the whole paper.
#[tauri::command]
pub async fn confirm_full_text(app: AppHandle, paper_id: String) -> Result<(), CommandError> {
    blocking(app, move |s| {
        let mut db = s.connect()?;
        let id = known_paper(&mut db, &paper_id)?;
        let canonical = store::latest_canonical(&mut db, &id)?.ok_or_else(|| {
            CommandError::invalid_input("This paper has no full text to confirm.")
        })?;
        Ok(store::confirm_full_text(&mut db, canonical.id)?)
    })
    .await
}

/// The stored contact email, if any.
#[tauri::command]
pub async fn get_contact_email(app: AppHandle) -> Result<Option<String>, CommandError> {
    blocking(app, |s| {
        Ok(store::get_setting(&mut s.connect()?, CONTACT_EMAIL_KEY)?)
    })
    .await
}

/// Stores the contact email after checking its shape.
#[tauri::command]
pub async fn set_contact_email(app: AppHandle, email: String) -> Result<(), CommandError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A known paper id comes back as a `PaperId`; an unknown one is `invalid_input`.
    #[test]
    fn known_paper_checks_the_id() {
        let mut db = store::open_memory_store().unwrap();
        let p = store::create_paper(&mut db, &Default::default(), None).unwrap();
        assert_eq!(known_paper(&mut db, &p.0).unwrap(), p);
        let e = known_paper(&mut db, "nope").unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
        assert!(e.message.contains("nope"));
    }

    /// The name is the file name only; a missing file or a folder is `invalid_input`.
    #[test]
    fn read_chosen_file_name_only_and_missing_file() {
        let dir = std::env::temp_dir().join(format!("sa-cmd-{}", std::process::id()));
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

    /// A file the app may not read is the app's fault (`internal`), not the user's.
    #[cfg(unix)]
    #[test]
    fn read_chosen_file_unreadable_is_internal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sa-cmd-perm-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("sa-cmd-big-{}", std::process::id()));
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
