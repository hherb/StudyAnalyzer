//! The Tauri command layer: a thin shell over `studyanalyzer-core`.

mod commands;
mod error;
mod state;

use std::sync::{Mutex, OnceLock};

use studyanalyzer_core::config::DB_FILE_NAME;
use studyanalyzer_core::store::open_store;
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};

use crate::state::{AppState, PDFIUM_CACHE_ENV, PDFIUM_CACHE_SUBDIR};

/// Creates the app's directories, opens the database and registers the state.
fn setup_state(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    let cache_dir = app.path().app_cache_dir()?;
    let state = AppState {
        db: Mutex::new(open_state_db(&data_dir)?),
        data_dir,
        cache_dir,
        pdf: OnceLock::new(),
    };
    std::fs::create_dir_all(state.scratch_dir())?;
    std::fs::create_dir_all(state.fulltext_dir())?;
    let pdfium_dir = state.cache_dir.join(PDFIUM_CACHE_SUBDIR);
    std::fs::create_dir_all(&pdfium_dir)?;
    // Safe here: setup runs before any command thread reads the variable; the
    // sandbox denies PDFium's default cache location.
    std::env::set_var(PDFIUM_CACHE_ENV, &pdfium_dir);
    app.manage(state);
    Ok(())
}

/// Creates the data directory and opens (migrating) the database in it.
fn open_state_db(
    data_dir: &std::path::Path,
) -> Result<rusqlite::Connection, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data_dir)?;
    Ok(open_store(&data_dir.join(DB_FILE_NAME))?)
}

/// Builds and runs the app with the dialog and log plugins.
///
/// Panics if Tauri cannot start, which leaves nothing to report to.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                ])
                .build(),
        )
        .setup(|app| setup_state(app))
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
        .expect("error while running StudyAnalyzer");
}
