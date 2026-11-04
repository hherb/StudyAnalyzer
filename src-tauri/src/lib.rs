//! The Tauri command layer: a thin shell over `studyanalyzer-core`.

mod commands;
#[cfg(test)]
mod contract;
mod error;
mod grants;
mod handoff;
mod secrets;
mod state;

use std::collections::HashMap;
use std::sync::Mutex;

use studyanalyzer_core::config::DB_FILE_NAME;
use studyanalyzer_core::store::open_store;
use tauri::ipc::Invoke;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{Target, TargetKind};

use crate::grants::GrantedFile;
use crate::handoff::FILES_DROPPED_EVENT;
use crate::secrets::KeyringSecrets;
use crate::state::{AppState, StopSlot};

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
        embedder: AppState::embedder_slot(&data_dir),
        data_dir,
        cache_dir,
        lookup: Mutex::new(None),
        answer_stop: StopSlot::default(),
        pico_stop: StopSlot::default(),
        download_stop: StopSlot::default(),
        secrets: Box::new(KeyringSecrets::keyring()),
        windows: Mutex::new(HashMap::new()),
        grants: Default::default(),
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
        commands::pick_file,
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
        commands::pico,
        commands::pico_run,
        commands::run_pico,
        commands::stop_pico,
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
        commands::pending_handoffs,
        commands::open_handoff,
        commands::discard_handoff,
    ]
}

/// Turns files dropped on the window into grants and tells the webview; a
/// dropped hand-off file is received as a hand-off instead. A failure is
/// logged: the webview cannot show what it never heard of.
fn files_dropped<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event else {
        return;
    };
    let Some(state) = window.try_state::<AppState>() else {
        return log::error!("files were dropped but the app state is missing");
    };
    let (handed, papers): (Vec<_>, Vec<_>) = paths
        .iter()
        .cloned()
        .partition(|p| handoff::is_handoff_path(p));
    for path in handed {
        handoff::receive_file(window.app_handle(), path, "a drop");
    }
    // A drop of hand-off files only leaves the earlier grants as they were.
    if papers.is_empty() && !paths.is_empty() {
        return;
    }
    let granted: Vec<GrantedFile> = state.grants.replace(papers);
    if let Err(e) = window.emit(FILES_DROPPED_EVENT, granted) {
        log::error!("could not tell the window about the dropped files: {e}");
    }
}

/// Wires up the routes a hand-off arrives by once the state exists: links
/// while running and the link that launched the app, files opened before
/// the state existed, and on Linux and Windows the hand-off files named on
/// the command line. Every failure is logged.
fn setup_handoffs(app: &AppHandle) {
    for path in app.state::<handoff::EarlyOpened>().take() {
        handoff::receive_file(app, path, OPENED_FILE);
    }
    let handle = app.clone();
    app.deep_link()
        .on_open_url(move |event| handoff::receive_links(&handle, event.urls()));
    match app.deep_link().get_current() {
        Ok(Some(urls)) => handoff::receive_links(app, urls),
        Ok(None) => {}
        Err(e) => log::error!("could not read the link the app was launched with: {e}"),
    }
    // macOS and iOS deliver opened files as `RunEvent::Opened` instead.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        // `args_os`: `args` panics on an argument that is not UTF-8.
        let args = std::env::args_os()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        handoff::receive_args(app, args, std::path::Path::new(""));
    }
    // The installer registers the scheme elsewhere; on Linux (an AppImage)
    // and in Windows debug builds nothing else does.
    #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
    if let Err(e) = app.deep_link().register_all() {
        log::error!("could not register the studyanalyzer:// scheme: {e}");
    }
}

/// A second launch on desktop: receives the hand-off files its arguments name
/// (relative to its working directory; its link went to the deep-link
/// plugin) and brings the main window forward.
#[cfg(desktop)]
fn second_instance<R: Runtime>(app: &AppHandle<R>, args: Vec<String>, cwd: String) {
    handoff::receive_args(app, args, std::path::Path::new(&cwd));
    let Some(window) = app.get_webview_window("main") else {
        return log::error!("a second launch found no main window to show");
    };
    if let Err(e) = window.unminimize().and_then(|()| window.set_focus()) {
        log::warn!("could not bring the main window forward: {e}");
    }
}

/// The route of a file the system asked the app to open, for the log and
/// any failure shown.
const OPENED_FILE: &str = "an opened file";

/// Receives the files macOS and iOS ask the app to open (its `file:` URLs;
/// links go to the deep-link plugin). On a cold launch this can run before
/// `setup` has made the state; such a file is held for `setup_handoffs`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn files_opened<R: Runtime>(app: &AppHandle<R>, urls: &[tauri::Url]) {
    // Checked here, on the main thread `setup` also runs on, so no file slips between.
    let started = app.try_state::<AppState>().is_some();
    let early = app.state::<handoff::EarlyOpened>();
    for url in urls.iter().filter(|u| !handoff::is_link(u)) {
        match url.to_file_path() {
            Ok(path) => {
                if let Some(path) = early.hold_unless_started(started, path) {
                    handoff::receive_file(app, path, OPENED_FILE);
                }
            }
            Err(()) => handoff::report_arrival_failure(
                app,
                OPENED_FILE,
                &format!("its address names no file on this device ({url})"),
            ),
        }
    }
}

/// Handles the app's run events: opened files (macOS, iOS) and the exit.
fn run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    if let RunEvent::Opened { urls } = &event {
        files_opened(app, urls);
    }
    unload_embedder(app, event);
}

/// Drops the loaded search model as the app exits: on Metal, llama.cpp aborts
/// at exit while a model is loaded, and Tauri exits without dropping its state.
fn unload_embedder<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    if let RunEvent::Exit = event {
        if let Some(s) = app.try_state::<AppState>() {
            drop(s.embedder.unload());
        }
    }
}

/// Builds and runs the app with its plugins (single instance first, on
/// desktop, so a second launch hands over before anything else runs).
///
/// Exits with status 1 if Tauri itself cannot start: there is no window or
/// app handle to show a dialog with, so the error is logged to stderr.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().manage(handoff::EarlyOpened::default());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(second_instance));
    match builder
        .plugin(tauri_plugin_deep_link::init())
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
            match setup_state(app) {
                Ok(()) => setup_handoffs(app.handle()),
                Err(e) => report_startup_failure(app.handle(), e.as_ref()),
            }
            Ok(())
        })
        .on_window_event(files_dropped)
        .invoke_handler(handler())
        .build(tauri::generate_context!())
    {
        Ok(app) => app.run(run_event),
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
