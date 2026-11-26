//! The Tauri command layer: a thin shell over `studyanalyzer-core`.

// The contract test's `json!` list of commands outgrows the default 128.
#![cfg_attr(test, recursion_limit = "256")]

mod appraisal_commands;
mod commands;
#[cfg(test)]
mod contract;
mod error;
mod evidence_commands;
mod grants;
mod handoff;
mod state;

use std::collections::HashMap;
use std::sync::Mutex;

use bmlib::secrets::KeyBundle;
use studyanalyzer_core::bookmark::Access;
use studyanalyzer_core::config::{DB_FILE_NAME, KEYCHAIN_SERVICE};
use studyanalyzer_core::library::{SettleFailed, Settled};
use studyanalyzer_core::library_setting::{self, Resumed};
use studyanalyzer_core::store::open_store;
use tauri::ipc::Invoke;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{Target, TargetKind};

use crate::grants::GrantedFile;
use crate::handoff::FILES_DROPPED_EVENT;
use crate::state::{library_label, lock, AppState, StopSlot};

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
    let mut db = open_store(&data_dir.join(DB_FILE_NAME))?;
    let (library_access, library_launch_note) = resume_library(&mut db);
    drop(db);
    let state = AppState {
        embedder: AppState::embedder_slot(&data_dir),
        data_dir,
        cache_dir,
        lookup: Mutex::new(None),
        answer_stop: StopSlot::default(),
        pico_stop: StopSlot::default(),
        prisma_stop: StopSlot::default(),
        evidence_stop: StopSlot::default(),
        evidence_appraisal: Mutex::new(None),
        download_stop: StopSlot::default(),
        secrets: Box::new(KeyBundle::keyring(KEYCHAIN_SERVICE)),
        limits: Mutex::new(HashMap::new()),
        grants: Default::default(),
        library_label: library_label(),
        library_op: Mutex::new(()),
        library_access: Mutex::new(library_access),
        library_launch_note: Mutex::new(library_launch_note),
    };
    std::fs::create_dir_all(state.scratch_dir())?;
    clear_scratch(&state.scratch_dir());
    std::fs::create_dir_all(state.fulltext_dir())?;
    app.manage(state);
    Ok(())
}

/// Starts access to the library folder through its bookmark (macOS; design
/// 2026-11-13, §4.1), at launch or on a focus's [`resume_again`]: the
/// access, and the launch's note: what it could not keep (#130) or why it
/// could not reach the folder (#132, #134), which every report of the
/// library ends with. A folder that cannot be reached, or a database
/// failure, is logged and the app goes on without the folder's access; the
/// database failure is the launch's note too (`resume` fails only reading
/// the settings, so an earlier launch's stored fault cannot be read then
/// either).
fn resume_library(db: &mut rusqlite::Connection) -> (Option<Access>, Option<String>) {
    match library_setting::resume(db) {
        Ok(Resumed::Held { access, unsaved }) => (Some(access), unsaved),
        Ok(Resumed::Nothing) => (None, None),
        Ok(Resumed::Lost { why }) => {
            log::warn!("the paper library cannot be reached: {why}");
            (None, Some(why))
        }
        Err(e) => {
            log::error!("the paper library's bookmark could not be checked: {e}");
            let note = format!("the folder's bookmark could not be checked ({e})");
            (None, Some(note))
        }
    }
}

/// Resolves the library's bookmark again while the launch could not reach it
/// (no access held, the launch's note kept), so a drive or share the user
/// connected since is reached without a relaunch (#126). The access and note
/// are replaced by this try's; true when what the status shows changed
/// (access held now, or another reason), so the webview rereads it.
fn resume_again(state: &AppState, db: &mut rusqlite::Connection) -> bool {
    if lock(&state.library_access).is_some() {
        return false;
    }
    let Some(before) = state.launch_note() else {
        return false;
    };
    let (access, note) = resume_library(db);
    let regained = access.is_some();
    if regained {
        log::info!("the paper library can be reached again");
    }
    let changed = regained || note.as_deref() != Some(before.as_str());
    *lock(&state.library_access) = access;
    *lock(&state.library_launch_note) = note;
    changed
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
        commands::library_status,
        commands::library_settle_listening,
        commands::choose_library,
        commands::clear_library,
        commands::move_into_library,
        commands::library_papers,
        commands::remove_library_paper,
        commands::remove_library_file,
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
        commands::prisma,
        commands::prisma_run,
        commands::run_prisma,
        commands::stop_prisma,
        evidence_commands::evidence,
        evidence_commands::evidence_run,
        evidence_commands::run_evidence,
        evidence_commands::stop_evidence,
        appraisal_commands::appraisal,
        appraisal_commands::start_appraisal,
        appraisal_commands::confirm_appraisal,
        appraisal_commands::edit_appraisal,
        appraisal_commands::save_appraisal,
        appraisal_commands::reveal_appraisal,
        appraisal_commands::close_revision,
        appraisal_commands::set_aside_appraisal,
        appraisal_commands::run_appraisal,
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

/// The event telling the webview that waiting sources were settled, or that
/// settling failed: a [`LibrarySettled`] payload. Sent only when a source
/// moved, one still waits, or it failed.
pub const LIBRARY_SETTLED_EVENT: &str = "library-settled";

/// What a settle of the waiting sources did, for the webview to say.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LibrarySettled {
    /// How many sources moved into the library.
    pub moved: usize,
    /// How many still wait; after a failure (`error` set), only those
    /// noted before it.
    pub left: usize,
    /// Why settling failed (already logged), or `None`.
    pub error: Option<String>,
}

/// Moves the sources waiting for the shared library into it, off the main
/// thread (design 2026-11-13, §4.6: checked at launch and when the app
/// becomes active). At launch (`wait`) it waits for the library lock; on
/// activation it skips a turn while a change of the setting, another settle
/// or a paper open holds it. What to settle and when to tell is core's
/// `library::settle_waiting`; when something moved or is left, or settling
/// failed, [`LIBRARY_SETTLED_EVENT`] says so, and is logged. On focus the
/// library lost at launch is tried again first ([`resume_again`]); when its
/// status changed, a 0/0 event with nothing else to tell makes the status
/// line reread.
fn settle_waiting<R: Runtime>(app: &AppHandle<R>, wait: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<AppState>() else {
            log::error!("no app state; waiting sources are not moved into the library");
            return;
        };
        let _op = if wait {
            state.library_op()
        } else {
            match state.try_library_op() {
                Some(guard) => guard,
                None => {
                    log::debug!("the library lock is held; waiting sources settle next time");
                    return;
                }
            }
        };
        let payload = match state.connect() {
            Err(e) => {
                log::error!(
                    "moving waiting sources into the library failed: {}",
                    e.message
                );
                LibrarySettled {
                    moved: 0,
                    left: 0,
                    error: Some(e.message),
                }
            }
            Ok(mut db) => {
                // On focus: a drive or share connected since launch is reached now (#126).
                let changed = !wait && resume_again(&state, &mut db);
                let settled =
                    studyanalyzer_core::library::settle_waiting(&mut db, &state.library_label);
                match settled_event(settled, changed) {
                    Some(payload) => payload,
                    None => return,
                }
            }
        };
        deliver_settled(&app, payload);
    });
}

/// What settles told before the webview listened for
/// [`LIBRARY_SETTLED_EVENT`]: the launch settle usually finishes first, and
/// an event emitted then is lost. Handed over once by
/// [`SettleInbox::listening`].
#[derive(Default)]
pub struct SettleInbox(Mutex<Inbox>);

/// [`SettleInbox`]'s state, under one lock so a settle cannot hold its
/// payload just after the webview took what was held.
#[derive(Default)]
struct Inbox {
    /// Whether the webview has said it listens.
    listening: bool,
    /// What settles told before then, merged.
    held: Option<LibrarySettled>,
}

impl SettleInbox {
    /// Marks the webview as listening and returns what was held, once.
    pub fn listening(&self) -> Option<LibrarySettled> {
        let mut inbox = lock(&self.0);
        inbox.listening = true;
        inbox.held.take()
    }

    /// Holds `payload` until the webview listens, merged with what is held;
    /// `Some(payload)` back when it already listens, to be emitted.
    fn hold(&self, payload: LibrarySettled) -> Option<LibrarySettled> {
        let mut inbox = lock(&self.0);
        if inbox.listening {
            return Some(payload);
        }
        inbox.held = Some(merged(inbox.held.take(), payload));
        None
    }
}

/// Two settles' reports as one: the files moved by both, and what the later
/// one found still waiting and why it failed (the earlier one's count and
/// failure are out of date).
fn merged(earlier: Option<LibrarySettled>, later: LibrarySettled) -> LibrarySettled {
    LibrarySettled {
        moved: earlier.map_or(0, |e| e.moved) + later.moved,
        ..later
    }
}

/// Tells the webview what a settle did, or holds it until the webview
/// listens ([`SettleInbox`]).
fn deliver_settled<R: Runtime>(app: &AppHandle<R>, payload: LibrarySettled) {
    let payload = match app.try_state::<SettleInbox>() {
        Some(inbox) => inbox.hold(payload),
        None => {
            log::error!(
                "the app has no inbox for settles; a report before the webview listens is lost"
            );
            Some(payload)
        }
    };
    if let Some(payload) = payload {
        if let Err(e) = app.emit(LIBRARY_SETTLED_EVENT, payload) {
            log::error!("the webview was not told of the settle: {e}");
        }
    }
}

/// The event for what core's `library::settle_waiting` did, logged; `None`
/// when there is nothing to tell, unless `reread` (the library's status
/// changed, #126): then a 0/0 event, which shows nothing but makes the
/// status line reread. A failure partway keeps the counts made before it
/// (#129).
fn settled_event(
    settled: Result<Option<Settled>, SettleFailed>,
    reread: bool,
) -> Option<LibrarySettled> {
    match settled {
        Ok(None) if reread => Some(LibrarySettled {
            moved: 0,
            left: 0,
            error: None,
        }),
        Ok(None) => None,
        Ok(Some(s)) => {
            log::info!(
                "{} waiting source(s) moved into the library, {} still waiting",
                s.moved,
                s.left.len()
            );
            Some(LibrarySettled {
                moved: s.moved,
                left: s.left.len(),
                error: None,
            })
        }
        Err(f) => {
            log::error!(
                "moving waiting sources into the library failed after {} moved: {}",
                f.settled.moved,
                f.error
            );
            Some(LibrarySettled {
                moved: f.settled.moved,
                left: f.settled.left.len(),
                error: Some(f.error.to_string()),
            })
        }
    }
}

/// Settles the waiting sources when a window gains focus (the app became
/// active).
fn settle_on_focus<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::Focused(true) = event {
        settle_waiting(window.app_handle(), false);
    }
}

/// Handles the app's run events: opened files and links (macOS, iOS) and the
/// exit.
fn run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    if let RunEvent::Opened { urls } = &event {
        urls_opened(app, urls);
    }
    on_exit(app, event);
}

/// As the app exits: drops the loaded search model (on Metal, llama.cpp
/// aborts at exit while a model is loaded, and Tauri exits without dropping
/// its state) and stops access to the library folder.
fn on_exit<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    if let RunEvent::Exit = event {
        if let Some(s) = app.try_state::<AppState>() {
            drop(s.embedder.unload());
            drop(lock(&s.library_access).take());
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
        .manage(handoff::EarlyLinks::default())
        .manage(SettleInbox::default());
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
                Ok(()) => {
                    settle_waiting(app.handle(), true);
                    setup_handoffs(app.handle(), current);
                }
                Err(e) => {
                    report_startup_failure(app.handle(), e.as_ref());
                    receive_held(app.handle(), current);
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            files_dropped(window, event);
            settle_on_focus(window, event);
        })
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
    use studyanalyzer_core::store::StoreError;

    /// Nothing to tell sends nothing; a settle failing partway sends what
    /// moved before it with the failure (#129).
    #[test]
    fn settled_event_keeps_the_counts_of_a_failure() {
        assert_eq!(settled_event(Ok(None), false), None);
        let moved = |moved| Settled {
            moved,
            ..Settled::default()
        };
        assert_eq!(
            settled_event(Ok(Some(moved(3))), false),
            Some(LibrarySettled {
                moved: 3,
                left: 0,
                error: None
            })
        );
        let failed = SettleFailed {
            settled: moved(2),
            error: StoreError::Corrupt("refused".into()),
        };
        let e = settled_event(Err(failed), true).unwrap();
        assert_eq!((e.moved, e.left), (2, 0));
        assert!(e.error.unwrap().contains("refused"));
    }

    /// A changed library status with nothing settled is a 0/0 event, so the
    /// status line rereads; counts to tell are kept, not replaced (#126).
    #[test]
    fn settled_event_rereads_a_changed_status() {
        let nothing = LibrarySettled {
            moved: 0,
            left: 0,
            error: None,
        };
        assert_eq!(settled_event(Ok(None), true), Some(nothing));
        let two = Settled {
            moved: 2,
            ..Settled::default()
        };
        assert_eq!(settled_event(Ok(Some(two)), true).unwrap().moved, 2);
    }

    /// Before the webview listens, settles are held and merged (moves added,
    /// the later count and failure kept) and handed over once; after, each
    /// is passed on to be emitted.
    #[test]
    fn settle_inbox_holds_until_the_webview_listens() {
        let report = |moved, left, error: Option<&str>| LibrarySettled {
            moved,
            left,
            error: error.map(String::from),
        };
        let inbox = SettleInbox::default();
        assert_eq!(inbox.hold(report(2, 1, Some("old"))), None);
        assert_eq!(inbox.hold(report(1, 0, None)), None);
        assert_eq!(inbox.listening(), Some(report(3, 0, None)));
        assert_eq!(inbox.listening(), None, "handed over once");
        let later = report(0, 1, None);
        assert_eq!(inbox.hold(later.clone()), Some(later));
        assert_eq!(inbox.listening(), None, "nothing held once it listens");
    }

    /// Nothing is resolved again when the launch left no note (no library,
    /// or one reached at launch).
    #[test]
    fn resume_again_only_after_a_lost_launch() {
        let s = AppState::for_tests(std::env::temp_dir());
        let mut db = studyanalyzer_core::store::open_memory_store().unwrap();
        assert!(!resume_again(&s, &mut db));
        assert!(lock(&s.library_launch_note).is_none());
    }

    /// A library held with a note (what the launch could not keep, #130) is
    /// not resolved again: its access and note stay.
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_again_leaves_a_held_library_alone() {
        let dir = std::env::temp_dir().join(format!("sa-held-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let data = studyanalyzer_core::bookmark::create(&dir).unwrap();
        let access = studyanalyzer_core::bookmark::resolve(&data).unwrap().access;
        let s = AppState::for_tests(std::env::temp_dir());
        *lock(&s.library_access) = Some(access);
        *lock(&s.library_launch_note) = Some("unsaved".into());
        let mut db = studyanalyzer_core::store::open_memory_store().unwrap();
        assert!(!resume_again(&s, &mut db));
        assert_eq!(lock(&s.library_launch_note).as_deref(), Some("unsaved"));
        assert!(lock(&s.library_access).is_some());
        drop(lock(&s.library_access).take());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A library lost at launch is reached on a later focus once a library is
    /// back at its path, the launch's note gone; while it is not, the note is
    /// this try's reason, told as a change (#126).
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_again_reaches_a_library_back_since_launch() {
        let root = std::env::temp_dir().join(format!("sa-again-{}", uuid::Uuid::new_v4()));
        let dir = root.join("Papers");
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = studyanalyzer_core::store::open_memory_store().unwrap();
        library_setting::choose(&mut db, "a test", None, &dir, false).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let s = AppState::for_tests(std::env::temp_dir());
        *lock(&s.library_launch_note) = Some("lost at launch".into());
        assert!(resume_again(&s, &mut db), "another reason is a change");
        let note = lock(&s.library_launch_note).clone().unwrap();
        assert!(note.contains("choose the folder again"), "{note}");
        assert!(!resume_again(&s, &mut db), "the same reason is none");
        std::fs::create_dir(&dir).unwrap();
        bmlib::library::Library::open(&dir).unwrap();
        assert!(resume_again(&s, &mut db));
        assert!(lock(&s.library_access).is_some());
        assert!(lock(&s.library_launch_note).is_none());
        assert!(!resume_again(&s, &mut db), "held: not again");
        drop(lock(&s.library_access).take());
        std::fs::remove_dir_all(&root).ok();
    }

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
