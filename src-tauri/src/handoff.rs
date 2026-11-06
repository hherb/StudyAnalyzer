//! Receiving hand-offs: a `.studyanalyzer` file (opened, dropped, or named
//! on the command line) or a `studyanalyzer://` link. Each is read, parsed and
//! stored before the webview hears of it ([`HANDOFF_EVENT`]), so a hand-off
//! that launched the app is still found and the copy is made while the
//! granted access lasts. A refused one is stored too, as its refusal.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bmlib::db::Db;
use studyanalyzer_core::config::{HANDOFF_EXTENSION, LINK_SCHEME, MAX_HANDOFF_BYTES};
use studyanalyzer_core::handoff::{parse_handoff, parse_link, Handoff};
use studyanalyzer_core::handoff_store::{save_handoff, save_refused_handoff};
use studyanalyzer_core::store::StoreError;
use tauri::{AppHandle, Emitter, Manager, Runtime, Url};
use tauri_plugin_dialog::{Dialog, MessageDialogKind};

use crate::commands::read_chosen_file;
use crate::state::{lock, AppState};

/// The event the drop handler emits with the new grants (`GrantedFile[]`).
pub const FILES_DROPPED_EVENT: &str = "files-dropped";

/// The event emitted (no payload) once a received hand-off is stored, so the
/// webview reloads the pending list.
pub const HANDOFF_EVENT: &str = "handoff-received";

/// The route of a hand-off named on a command line, for the log and any
/// failure shown.
pub const ARGUMENT: &str = "a command-line argument";

/// The cause shown for a hand-off that arrived when the app's state could not
/// be made.
pub const NOT_STARTED: &str = "StudyAnalyzer did not start";

/// Files that arrived before `setup` had run, each with the route it came by:
/// macOS can send `RunEvent::Opened` before `setup` on a cold launch, and a
/// second launch's single-instance callback can come before it too (on Linux
/// and macOS from another thread, on Windows from the main thread's message
/// loop). `setup` makes the state, or fails to, before it takes the held
/// files, and taking closes the queue. So each file is either held and taken
/// once or received at once, and a missing state is reported ([`NOT_STARTED`]).
pub struct EarlyOpened(Mutex<Option<Vec<(PathBuf, &'static str)>>>);

impl Default for EarlyOpened {
    /// An open, empty queue.
    fn default() -> Self {
        Self(Mutex::new(Some(Vec::new())))
    }
}

impl EarlyOpened {
    /// Holds `path`, which came by `route`, for [`EarlyOpened::take`] and
    /// returns `None`; once the queue is taken, returns `path` to be received
    /// at once.
    #[cfg(any(desktop, target_os = "ios", test))]
    pub fn hold(&self, path: PathBuf, route: &'static str) -> Option<PathBuf> {
        let mut queue = lock(&self.0);
        let Some(held) = queue.as_mut() else {
            return Some(path);
        };
        log::info!("a file arrived from {route} before setup ran; it waits for setup");
        held.push((path, route));
        None
    }

    /// The held files and their routes, closing the queue, so each is
    /// received once and nothing is held after.
    pub fn take(&self) -> Vec<(PathBuf, &'static str)> {
        lock(&self.0).take().unwrap_or_default()
    }
}

/// What the user is told when a hand-off that arrived by `route` could not be
/// received, `cause` saying why.
pub fn arrival_failure(route: &str, cause: &str) -> String {
    format!(
        "A hand-off from {route} could not be received: {cause}. Ask the sending app to send it again."
    )
}

/// What the user is told when the system asked the app to open `path`
/// (macOS *Open With…*), which is not a hand-off file and so is not read.
pub fn not_a_handoff_file(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!(
        "StudyAnalyzer was asked to open {name}, which is not a hand-off file. Open papers from the start screen."
    )
}

/// Logs and shows [`arrival_failure`]'s message (see [`show_error`]).
pub fn report_arrival_failure<R: Runtime>(app: &AppHandle<R>, route: &str, cause: &str) {
    show_error(app, arrival_failure(route, cause));
}

/// Logs `text` and shows it in an error dialog that does not block. Without
/// the dialog plugin (tests), the log is all there is.
pub fn show_error<R: Runtime>(app: &AppHandle<R>, text: String) {
    log::error!("{text}");
    match app.try_state::<Dialog<R>>() {
        Some(dialog) => dialog
            .message(text)
            .title("StudyAnalyzer")
            .kind(MessageDialogKind::Error)
            .show(|_| {}),
        None => log::error!("no dialog to show the hand-off failure in"),
    }
}

/// Stores a parsed hand-off, or its refusal (the text says why), and returns
/// the row id. A refusal is logged with its row.
fn store(db: &mut dyn Db, parsed: Result<Handoff, String>) -> Result<i64, StoreError> {
    match parsed {
        Ok(h) => save_handoff(db, &h),
        Err(why) => {
            let id = save_refused_handoff(db, &why)?;
            log::warn!("hand-off row {id} is a refusal: {why}");
            Ok(id)
        }
    }
}

/// Parses a hand-off file's bytes (or the reason they could not be read) and
/// stores the hand-off or its refusal; returns the row id.
pub fn receive(db: &mut dyn Db, read: Result<Vec<u8>, String>) -> Result<i64, StoreError> {
    store(
        db,
        read.and_then(|bytes| parse_handoff(&bytes).map_err(|e| e.to_string())),
    )
}

/// True when `path` has the hand-off extension, in any case.
pub fn is_handoff_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(HANDOFF_EXTENSION))
}

/// True for every URL but a `file:` one: on macOS the deep-link plugin also
/// reports opened files as URLs, and those arrive through `RunEvent::Opened`.
pub fn is_link(url: &Url) -> bool {
    url.scheme() != "file"
}

/// True when a command-line argument is a `studyanalyzer:` link, in any case.
#[cfg(desktop)]
fn is_link_arg(arg: &str) -> bool {
    arg.to_ascii_lowercase()
        .starts_with(&format!("{LINK_SCHEME}:"))
}

/// The hand-off files among a command line's arguments: the program name,
/// `studyanalyzer:` links (see [`unreceived_links`]) and anything without the
/// hand-off extension are skipped, each skip logged. Any program can launch
/// the app with arguments, so a path is only ever read as a hand-off.
#[cfg(desktop)]
pub fn handoff_paths(args: &[String]) -> Vec<PathBuf> {
    args.iter()
        .skip(1)
        .filter(|arg| {
            if is_link_arg(arg) {
                log::info!("argument {arg:?} skipped: a link, not a file");
                false
            } else if !is_handoff_path(Path::new(arg)) {
                log::info!("argument {arg:?} skipped: not a .{HANDOFF_EXTENSION} file");
                false
            } else {
                true
            }
        })
        .map(PathBuf::from)
        .collect()
}

/// The links among a command line's arguments that nothing receives. On Linux
/// and Windows the deep-link plugin takes a link only as the sole argument
/// (`handle_cli_arguments`); on macOS it reads no arguments.
#[cfg(desktop)]
pub fn unreceived_links(args: &[String]) -> Vec<&str> {
    if cfg!(any(target_os = "linux", target_os = "windows")) && args.len() == 2 {
        return Vec::new();
    }
    args.iter()
        .skip(1)
        .map(String::as_str)
        .filter(|arg| is_link_arg(arg))
        .collect()
}

/// Runs `work` (reading, parsing and storing a hand-off) on a blocking
/// thread, logs one line naming `route` and the row, then tells the webview.
/// A failure to store is shown in a dialog: the webview has nothing to list.
fn spawn_receive<R: Runtime>(
    app: &AppHandle<R>,
    route: &'static str,
    work: impl FnOnce(&mut dyn Db) -> Result<i64, StoreError> + Send + 'static,
) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<AppState>() else {
            return report_arrival_failure(&app, route, NOT_STARTED);
        };
        let stored = state
            .connect()
            .map_err(|e| e.message)
            .and_then(|mut db| work(&mut db).map_err(|e| e.to_string()));
        match stored {
            Ok(id) => log::info!("hand-off received from {route}: row {id}"),
            Err(e) => {
                return report_arrival_failure(&app, route, &format!("it was not saved ({e})"))
            }
        }
        if let Err(e) = app.emit(HANDOFF_EVENT, ()) {
            log::error!("could not tell the window about the hand-off: {e}");
        }
    });
}

/// Receives the hand-off file at `path`, read at most one byte past
/// `MAX_HANDOFF_BYTES` (a failure to read is stored as the refusal). `route`
/// names how it came ("an opened file", "a command-line argument", "a drop")
/// for the log and any failure shown.
pub fn receive_file<R: Runtime>(app: &AppHandle<R>, path: PathBuf, route: &'static str) {
    spawn_receive(app, route, move |db| {
        let read = read_chosen_file(&path, MAX_HANDOFF_BYTES)
            .map(|(_, bytes)| bytes)
            .map_err(|e| e.message);
        receive(db, read)
    });
}

/// Receives a `studyanalyzer://` link.
pub fn receive_link<R: Runtime>(app: &AppHandle<R>, link: &str) {
    let link = link.to_string();
    spawn_receive(app, "a link", move |db| {
        store(db, parse_link(&link).map_err(|e| e.to_string()))
    });
}

/// Receives each link among `urls`, skipping `file:` URLs (see [`is_link`]).
pub fn receive_links<R: Runtime>(app: &AppHandle<R>, urls: Vec<Url>) {
    for url in urls {
        if is_link(&url) {
            receive_link(app, url.as_str());
        } else {
            log::info!("a file URL reached the link handler; it is received as an opened file");
        }
    }
}

/// Receives the hand-off files a command line names (see [`handoff_paths`]),
/// relative ones resolved against `cwd`, and tells the user of each link
/// nothing receives ([`unreceived_links`]). Files that come before `setup` ran
/// wait in [`EarlyOpened`].
#[cfg(desktop)]
pub fn receive_args<R: Runtime>(app: &AppHandle<R>, args: Vec<String>, cwd: &Path) {
    let early = app.state::<EarlyOpened>();
    for path in handoff_paths(&args) {
        if let Some(path) = early.hold(cwd.join(path), ARGUMENT) {
            receive_file(app, path, ARGUMENT);
        }
    }
    for link in unreceived_links(&args) {
        let cause = format!(
            "a link on the command line ({link}) is read only as the sole argument, on Linux and Windows"
        );
        report_arrival_failure(app, ARGUMENT, &cause);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studyanalyzer_core::handoff_store::pending_handoffs;
    use studyanalyzer_core::store::open_memory_store;

    /// A valid file is stored as a hand-off, not a refusal.
    #[test]
    fn receive_stores_a_valid_file() {
        let mut db = open_memory_store().unwrap();
        let file = br#"{"format":"studyanalyzer-handoff","version":1,"sender":"MFC","identifiers":{"pmid":"123"}}"#;
        let id = receive(&mut db, Ok(file.to_vec())).unwrap();
        let [row] = &pending_handoffs(&mut db).unwrap()[..] else {
            panic!("expected one row")
        };
        assert_eq!(row.id, id);
        assert_eq!(row.sender.as_deref(), Some("MFC"));
        assert_eq!(row.identifiers.pmid.as_deref(), Some("123"));
        assert_eq!(row.refusal, None);
    }

    /// A file that is not valid becomes a refusal row with the parser's reason.
    #[test]
    fn receive_stores_a_refusal() {
        let mut db = open_memory_store().unwrap();
        let bad = b"{not json".to_vec();
        let why = studyanalyzer_core::handoff::parse_handoff(&bad)
            .unwrap_err()
            .to_string();
        let id = receive(&mut db, Ok(bad)).unwrap();
        let [row] = &pending_handoffs(&mut db).unwrap()[..] else {
            panic!("expected one row")
        };
        assert_eq!((row.id, row.refusal.as_deref()), (id, Some(why.as_str())));
    }

    /// A file that could not be read becomes a refusal row naming the failure.
    #[test]
    fn receive_stores_a_read_failure() {
        let mut db = open_memory_store().unwrap();
        receive(&mut db, Err("permission denied".into())).unwrap();
        let rows = pending_handoffs(&mut db).unwrap();
        assert!(rows[0]
            .refusal
            .as_deref()
            .unwrap()
            .contains("permission denied"));
    }

    /// Only `.studyanalyzer` paths are taken: not the program, a link or another file.
    #[cfg(desktop)]
    #[test]
    fn args_skip_non_handoffs() {
        let args: Vec<String> = [
            "app",
            "studyanalyzer://open?pmid=1",
            "/x/a.pdf",
            "/x/b.studyanalyzer",
            "/x/C.StudyAnalyzer",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            handoff_paths(&args),
            [
                PathBuf::from("/x/b.studyanalyzer"),
                PathBuf::from("/x/C.StudyAnalyzer")
            ]
        );
        // The program name is skipped even when it looks like a hand-off.
        assert!(handoff_paths(&["/x/b.studyanalyzer".to_string()]).is_empty());
    }

    /// A file opened before the app started is held and received once, at
    /// setup; one opened after is received at once and never held.
    #[test]
    fn early_opened_holds_until_setup_once() {
        let early = EarlyOpened::default();
        let path = PathBuf::from("/x/b.studyanalyzer");
        assert_eq!(early.hold(path.clone(), ARGUMENT), None);
        assert_eq!(early.take(), [(path.clone(), ARGUMENT)]);
        assert_eq!(early.hold(path.clone(), ARGUMENT), Some(path));
        assert!(early.take().is_empty());
    }

    /// A command line that comes before setup has its files held, joined to
    /// its working directory and labelled as arguments.
    #[cfg(desktop)]
    #[test]
    fn args_before_setup_are_held() {
        let app = tauri::test::mock_builder()
            .manage(EarlyOpened::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let args = ["app", "b.studyanalyzer"].map(String::from).to_vec();
        receive_args(app.handle(), args, Path::new("/cwd"));
        assert_eq!(
            app.state::<EarlyOpened>().take(),
            [(PathBuf::from("/cwd/b.studyanalyzer"), ARGUMENT)]
        );
    }

    /// A link among other arguments is received by nothing; a sole one is the
    /// deep-link plugin's on Linux and Windows only.
    #[cfg(desktop)]
    #[test]
    fn links_nothing_receives() {
        let link = "StudyAnalyzer://open?pmid=1";
        let many = ["app", link, "/x/b.studyanalyzer"]
            .map(String::from)
            .to_vec();
        assert_eq!(unreceived_links(&many), [link]);
        let sole = ["app", link].map(String::from).to_vec();
        assert_eq!(
            unreceived_links(&sole).is_empty(),
            cfg!(any(target_os = "linux", target_os = "windows"))
        );
        assert!(unreceived_links(&["app".to_string()]).is_empty());
    }

    /// The message names the route and the cause, and says what to do.
    #[test]
    fn arrival_failure_names_route_and_cause() {
        assert_eq!(
            arrival_failure("a drop", "it was not saved (disk full)"),
            "A hand-off from a drop could not be received: it was not saved (disk full). \
             Ask the sending app to send it again."
        );
    }

    /// An opened file that is not a hand-off is named, and the user is sent
    /// to the start screen.
    #[test]
    fn not_a_handoff_file_names_the_file() {
        assert_eq!(
            not_a_handoff_file(Path::new("/x/paper.pdf")),
            "StudyAnalyzer was asked to open paper.pdf, which is not a hand-off file. \
             Open papers from the start screen."
        );
    }

    /// A `file:` URL is a file, never a link; every other URL is a link.
    #[test]
    fn file_urls_are_not_links() {
        let file: tauri::Url = "file:///x/b.studyanalyzer".parse().unwrap();
        let link: tauri::Url = "studyanalyzer://open?pmid=1".parse().unwrap();
        assert!(!is_link(&file));
        assert!(is_link(&link));
    }
}
