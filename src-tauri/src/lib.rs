//! The Tauri command layer: a thin shell over `studyanalyzer-core`.

mod commands;
#[cfg(test)]
mod contract;
mod error;
mod grants;
mod handoff;
mod state;

use std::collections::HashMap;
use std::sync::Mutex;

use bmlib::secrets::KeyBundle;
use studyanalyzer_core::config::{DB_FILE_NAME, KEYCHAIN_SERVICE};
use studyanalyzer_core::store::open_store;
use tauri::ipc::Invoke;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{Target, TargetKind};

use crate::grants::GrantedFile;
use crate::handoff::FILES_DROPPED_EVENT;
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
        secrets: Box::new(KeyBundle::keyring(KEYCHAIN_SERVICE)),
        limits: Mutex::new(HashMap::new()),
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
        commands::list_models,
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
    let (handed, papers) = split_drop(paths);
    for path in handed {
        handoff::receive_file(window.app_handle(), path, "a drop");
    }
    let Some(papers) = papers else {
        return;
    };
    let granted: Vec<GrantedFile> = state.grants.replace(papers);
    if let Err(e) = window.emit(FILES_DROPPED_EVENT, granted) {
        log::error!("could not tell the window about the dropped files: {e}");
    }
}

/// Splits dropped paths into the hand-off files to receive and the papers to
/// grant. The papers are `None` when the drop held hand-off files only, so the
/// earlier grants stay as they were; an empty drop clears them.
fn split_drop(
    paths: &[std::path::PathBuf],
) -> (Vec<std::path::PathBuf>, Option<Vec<std::path::PathBuf>>) {
    let (handed, papers): (Vec<_>, Vec<_>) = paths
        .iter()
        .cloned()
        .partition(|p| handoff::is_handoff_path(p));
    let keep_grants = papers.is_empty() && !handed.is_empty();
    (handed, (!keep_grants).then_some(papers))
}

/// On Android, registers the deep-link listener, first thing in `setup` so a
/// link sent while the state is made waits in [`handoff::EarlyLinks`], and
/// returns the plugin's links read just after it (see
/// [`handoff::links_at_setup`]). Elsewhere links come by the app's own routes
/// (the command line, `RunEvent::Opened`) and this returns none: Tauri can
/// defer a listener's registration until an emit that never comes (#94).
fn listen_for_links<R: Runtime>(app: &AppHandle<R>) -> Vec<tauri::Url> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        let handle = app.clone();
        app.deep_link()
            .on_open_url(move |event| handoff::receive_links(&handle, event.urls()));
        match app.deep_link().get_current() {
            Ok(urls) => urls.unwrap_or_default(),
            Err(e) => {
                handoff::report_arrival_failure(
                    app,
                    handoff::LINK,
                    &format!("the links that came before setup could not be read ({e})"),
                );
                Vec::new()
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Vec::new()
    }
}

/// Receives the hand-offs that arrived before `setup` made the state (see
/// [`handoff::Early`]) and closes their queues: the held files, the held and
/// `current` links ([`handoff::links_at_setup`]), and on Linux and Windows
/// what this launch's command line names (macOS and iOS deliver it as
/// `RunEvent::Opened`). Run once the state is made, or after it failed, when
/// each is reported as not received.
fn receive_held<R: Runtime>(app: &AppHandle<R>, current: Vec<tauri::Url>) {
    for (path, route) in app.state::<handoff::EarlyOpened>().take() {
        receive_opened(app, path, route);
    }
    let held = app.state::<handoff::EarlyLinks>().take();
    for link in handoff::links_at_setup(current, held) {
        handoff::receive_link(app, &link);
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        // `args_os`: `args` panics on an argument that is not UTF-8.
        let args = std::env::args_os()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        handoff::receive_args(app, args, std::path::Path::new(""));
    }
}

/// Wires up the routes a hand-off arrives by once the state exists: what
/// arrived until now ([`receive_held`]) and the scheme's registration where
/// no installer made it. Every failure is reported.
fn setup_handoffs(app: &AppHandle, current: Vec<tauri::Url>) {
    receive_held(app, current);
    // The installer registers the scheme elsewhere; on Linux (an AppImage)
    // and in Windows debug builds nothing else does. Off the main thread: on
    // Linux it runs `xdg-mime`, and setup need not wait for it.
    #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
    {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            use tauri_plugin_deep_link::DeepLinkExt;
            if let Err(e) = handle.deep_link().register_all() {
                handoff::show_error(
                    &handle,
                    format!("Links from other apps will not open StudyAnalyzer: its studyanalyzer:// links could not be registered ({e})."),
                );
            }
        });
    }
}

/// A second launch on desktop: receives the hand-off files (relative to its
/// working directory) and the links its arguments name, and brings the main
/// window forward.
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

/// Receives a file the system asked the app to open, or one held until
/// setup, `route` naming how it came: a hand-off file, by its extension as on
/// the command line; any other (a PDF through *Open With…*) is not read, and
/// the user is told so.
fn receive_opened<R: Runtime>(app: &AppHandle<R>, path: std::path::PathBuf, route: &'static str) {
    if handoff::is_handoff_path(&path) {
        handoff::receive_file(app, path, route);
    } else {
        handoff::show_error(app, handoff::not_a_handoff_file(&path));
    }
}

/// Receives the files and links macOS and iOS ask the app to open. On a cold
/// launch this can run before `setup` has made the state; such a file or link
/// is held for [`receive_held`].
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn urls_opened<R: Runtime>(app: &AppHandle<R>, urls: &[tauri::Url]) {
    let early = app.state::<handoff::EarlyOpened>();
    for url in urls {
        if handoff::is_link(url) {
            handoff::receive_or_hold_link(app, url.to_string());
            continue;
        }
        match url.to_file_path() {
            Ok(path) => {
                if let Some((path, route)) = early.hold((path, OPENED_FILE), OPENED_FILE) {
                    receive_opened(app, path, route);
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

/// Handles the app's run events: opened files and links (macOS, iOS) and the
/// exit.
fn run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    if let RunEvent::Opened { urls } = &event {
        urls_opened(app, urls);
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
    let builder = tauri::Builder::default()
        .manage(handoff::EarlyOpened::default())
        .manage(handoff::EarlyLinks::default());
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
            let current = listen_for_links(app.handle());
            match setup_state(app) {
                Ok(()) => setup_handoffs(app.handle(), current),
                Err(e) => {
                    report_startup_failure(app.handle(), e.as_ref());
                    receive_held(app.handle(), current);
                }
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

    /// Hand-off files are received and the rest granted; a drop of hand-off
    /// files only leaves the grants alone, an empty one clears them (#67).
    #[test]
    fn split_drop_partitions_handoffs() {
        use std::path::PathBuf;
        let p = |s: &str| PathBuf::from(s);
        let (handed, papers) = split_drop(&[p("/a/x.studyanalyzer"), p("/a/y.pdf"), p("/a/z.txt")]);
        assert_eq!(handed, [p("/a/x.studyanalyzer")]);
        assert_eq!(papers, Some(vec![p("/a/y.pdf"), p("/a/z.txt")]));
        let (handed, papers) = split_drop(&[p("/a/x.studyanalyzer")]);
        assert_eq!(handed, [p("/a/x.studyanalyzer")]);
        assert_eq!(papers, None);
        assert_eq!(split_drop(&[]), (vec![], Some(vec![])));
    }

    /// A scratch directory that cannot be listed is logged, not a panic.
    #[test]
    fn clear_scratch_missing_directory_is_harmless() {
        clear_scratch(
            &std::env::temp_dir().join(format!("sa-no-scratch-{}", uuid::Uuid::new_v4())),
        );
    }
}
