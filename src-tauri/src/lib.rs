//! The Tauri command layer: a thin shell over `studyanalyzer-core`.

mod commands;
#[cfg(test)]
mod contract;
mod error;
mod secrets;
mod state;

use std::collections::HashMap;
use std::sync::Mutex;

use studyanalyzer_core::config::DB_FILE_NAME;
use studyanalyzer_core::store::open_store;
use tauri::ipc::Invoke;
use tauri::{Manager, RunEvent, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{Target, TargetKind};

use crate::secrets::KeyringSecrets;
use crate::state::{lock, AppState, StopSlot};

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
        answer_stop: StopSlot::default(),
        download_stop: StopSlot::default(),
        secrets: Box::new(KeyringSecrets::keyring()),
        windows: Mutex::new(HashMap::new()),
        embedder: Mutex::new(None),
    };
    std::fs::create_dir_all(state.scratch_dir())?;
    clear_scratch(&state.scratch_dir());
    std::fs::create_dir_all(state.fulltext_dir())?;
    app.manage(state);
    Ok(())
}

/// Every command the frontend can invoke. Shared by [`run`] and the contract
/// test, which calls each through Tauri's mock runtime.
pub(crate) fn handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::open_file,
        commands::open_paste,
        commands::open_identifier,
        commands::cancel_lookup,
        commands::attach_file,
        commands::attach_paste,
        commands::reopen_paper,
        commands::recent_papers,
        commands::paper_view,
        commands::original_pdf,
        commands::confirm_full_text,
        commands::get_contact_email,
        commands::set_contact_email,
        commands::get_keep_mismatched,
        commands::set_keep_mismatched,
        commands::ask,
        commands::stop_answer,
        commands::new_conversation,
        commands::interrogator,
        commands::conversation,
        commands::paper_view_at,
        commands::llm_settings,
        commands::set_model,
        commands::set_context_tokens,
        commands::clear_context_tokens,
        commands::set_api_key,
        commands::delete_api_key,
        commands::model_window,
        commands::embedder_status,
        commands::download_embedder,
        commands::cancel_download,
    ]
}

/// Drops the loaded search model as the app exits: on Metal, llama.cpp aborts
/// at exit while a model is loaded, and Tauri exits without dropping its state.
fn unload_embedder<R: Runtime>(app: &tauri::AppHandle<R>, event: RunEvent) {
    if let RunEvent::Exit = event {
        if let Some(s) = app.try_state::<AppState>() {
            drop(lock(&s.embedder).take());
        }
    }
}

/// Builds and runs the app with the dialog and log plugins.
///
/// Exits with status 1 if Tauri itself cannot start: there is no window or
/// app handle to show a dialog with, so the error is logged to stderr.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    match tauri::Builder::default()
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
        .invoke_handler(handler())
        .build(tauri::generate_context!())
    {
        Ok(app) => app.run(unload_embedder),
        Err(e) => {
            eprintln!("StudyAnalyzer could not start: {e}");
            std::process::exit(1);
        }
    }
}

/// A mock app with every command and no state, for the contract test.
#[cfg(test)]
fn handler_app() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .invoke_handler(handler())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
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
