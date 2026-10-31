//! The Tauri command layer: a thin shell over `studyanalyzer-core`.

mod commands;
#[cfg(test)]
mod contract;
mod error;
mod state;

use std::sync::Mutex;

use studyanalyzer_core::config::DB_FILE_NAME;
use studyanalyzer_core::store::open_store;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{Target, TargetKind};

use crate::state::AppState;

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

/// Reports a startup failure: logs it, hides the windows on desktop (they have
/// no state, so every command would fail) and shows a blocking error dialog,
/// then exits once it is dismissed, or at once if the dialog fails. The dialog
/// runs on its own thread because blocking on the main thread (where setup
/// runs) would deadlock it. A window is hidden, not closed, because closing the
/// last one would exit before the dialog. On phones the window stays: iOS shows
/// the dialog inside it (hiding it would hide the dialog), and Android cannot
/// hide it.
fn report_startup_failure(app: &tauri::AppHandle, error: &dyn std::error::Error) {
    let text = format!("StudyAnalyzer could not start: {error}");
    log::error!("{text}");
    #[cfg(desktop)]
    for (label, window) in app.webview_windows() {
        if let Err(e) = window.hide() {
            log::warn!("could not hide window {label} after the startup failure: {e}");
        }
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        // The plugin panics when its dialog never answers; exit anyway, or a
        // hidden window would leave the app running with nothing on screen.
        let shown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle
                .dialog()
                .message(text)
                .title("StudyAnalyzer")
                .kind(MessageDialogKind::Error)
                .blocking_show()
        }));
        if shown.is_err() {
            log::error!("the startup-failure dialog could not be shown; exiting");
        }
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
        lookup: Mutex::new(None),
    };
    std::fs::create_dir_all(state.scratch_dir())?;
    clear_scratch(&state.scratch_dir());
    std::fs::create_dir_all(state.fulltext_dir())?;
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
            // A failure is shown to the user; on desktop the windows, which have
            // no state, stay hidden until the dialog exits the app.
            if let Err(e) = setup_state(app) {
                report_startup_failure(app.handle(), e.as_ref());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_file,
            commands::open_paste,
            commands::open_identifier,
            commands::cancel_lookup,
            commands::attach_file,
            commands::attach_paste,
            commands::reopen_paper,
            commands::recent_papers,
            commands::paper_view,
            commands::confirm_full_text,
            commands::get_contact_email,
            commands::set_contact_email,
            commands::get_keep_mismatched,
            commands::set_keep_mismatched,
        ])
        .run(tauri::generate_context!())
    {
        eprintln!("StudyAnalyzer could not start: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files are removed; a folder and its contents are left alone, and a
    /// symlink to a file outside goes without its target.
    #[test]
    fn clear_scratch_removes_files_only() {
        let root = std::env::temp_dir().join(format!("sa-scratch-{}", uuid::Uuid::new_v4()));
        let scratch = root.join("scratch");
        let nested = scratch.join("folder");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(scratch.join("left.pdf"), b"pdf").unwrap();
        std::fs::write(nested.join("inner.pdf"), b"pdf").unwrap();
        let outside = root.join("outside.pdf");
        std::fs::write(&outside, b"keep").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, scratch.join("link.pdf")).unwrap();

        clear_scratch(&scratch);

        assert!(!scratch.join("left.pdf").exists());
        assert!(nested.join("inner.pdf").exists());
        assert!(outside.exists());
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(scratch.join("link.pdf")).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A scratch directory that cannot be listed is logged, not a panic.
    #[test]
    fn clear_scratch_missing_directory_is_harmless() {
        clear_scratch(
            &std::env::temp_dir().join(format!("sa-no-scratch-{}", uuid::Uuid::new_v4())),
        );
    }
}
