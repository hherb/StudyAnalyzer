//! The Tauri command layer: a thin shell over `studyanalyzer-core`.

mod commands;
mod error;
mod state;

use std::sync::OnceLock;

use studyanalyzer_core::config::DB_FILE_NAME;
use studyanalyzer_core::store::open_store;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{Target, TargetKind};

use crate::state::{AppState, PDFIUM_CACHE_ENV, PDFIUM_CACHE_SUBDIR};

/// Removes leftover files from the scratch directory (a crash can strand PDFs
/// there). Failures are logged, never fatal: stale scratch files are harmless.
fn clear_scratch(dir: &std::path::Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return log::warn!("could not list scratch directory {}: {e}", dir.display()),
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(e) => {
                log::warn!("could not read a scratch entry in {}: {e}", dir.display());
                continue;
            }
        };
        if path.is_file() {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("could not remove scratch file {}: {e}", path.display());
            }
        }
    }
}

/// Reports a startup failure: logs it and shows a blocking error dialog, then
/// exits once it is dismissed. The dialog runs on its own thread because
/// blocking on the main thread (where setup runs) would deadlock it.
fn report_startup_failure(app: &tauri::AppHandle, error: &dyn std::error::Error) {
    let text = format!("StudyAnalyzer could not start: {error}");
    log::error!("{text}");
    let handle = app.clone();
    std::thread::spawn(move || {
        handle
            .dialog()
            .message(text)
            .title("StudyAnalyzer")
            .kind(MessageDialogKind::Error)
            .blocking_show();
        handle.exit(1);
    });
}

/// Creates the app's directories, opens the database and registers the state.
fn setup_state(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    let cache_dir = app.path().app_cache_dir()?;
    // Opening once here migrates the schema before any command connects.
    std::fs::create_dir_all(&data_dir)?;
    open_store(&data_dir.join(DB_FILE_NAME))?;
    let state = AppState {
        data_dir,
        cache_dir,
        pdf: OnceLock::new(),
    };
    std::fs::create_dir_all(state.scratch_dir())?;
    clear_scratch(&state.scratch_dir());
    std::fs::create_dir_all(state.fulltext_dir())?;
    let pdfium_dir = state.cache_dir.join(PDFIUM_CACHE_SUBDIR);
    std::fs::create_dir_all(&pdfium_dir)?;
    // Safe here: setup runs before any command thread reads the variable; the
    // sandbox denies PDFium's default cache location.
    std::env::set_var(PDFIUM_CACHE_ENV, &pdfium_dir);
    app.manage(state);
    Ok(())
}

/// Builds and runs the app with the dialog and log plugins.
///
/// Exits with status 1 if Tauri itself cannot start: there is no window or
/// app handle to show a dialog with, so the error is logged to stderr.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(e) = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                ])
                .build(),
        )
        .setup(|app| {
            // A failure is shown to the user; the window then has no state, so
            // its commands report an error until the dialog exits the app.
            if let Err(e) = setup_state(app) {
                report_startup_failure(app.handle(), e.as_ref());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_file,
            commands::open_paste,
            commands::open_identifier,
            commands::attach_file,
            commands::attach_paste,
            commands::reopen_paper,
            commands::recent_papers,
            commands::paper_view,
            commands::confirm_full_text,
            commands::get_contact_email,
            commands::set_contact_email,
        ])
        .run(tauri::generate_context!())
    {
        eprintln!("StudyAnalyzer could not start: {e}");
        std::process::exit(1);
    }
}
