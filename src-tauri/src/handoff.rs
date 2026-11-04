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

/// Files the system asked the app to open before its state existed: macOS can
/// send `RunEvent::Opened` before `setup` runs on a cold launch. Both run on
/// the main thread, so a path is either held here or received, never both,
/// and `setup` takes the held ones once.
#[derive(Default)]
pub struct EarlyOpened(Mutex<Vec<PathBuf>>);

impl EarlyOpened {
    /// `path` back to be received now when the app has `started`; otherwise
    /// it is held for [`EarlyOpened::take`] and `None` is returned.
    #[cfg(any(target_os = "macos", target_os = "ios", test))]
    pub fn hold_unless_started(&self, started: bool, path: PathBuf) -> Option<PathBuf> {
        if started {
            return Some(path);
        }
        log::info!("a file was opened before the app finished starting; it waits for setup");
        lock(&self.0).push(path);
        None
    }

    /// The held paths, emptying the queue, so each is received once.
    pub fn take(&self) -> Vec<PathBuf> {
        std::mem::take(&mut *lock(&self.0))
    }
}

/// What the user is told when a hand-off that arrived by `route` could not be
/// received, `cause` saying why.
pub fn arrival_failure(route: &str, cause: &str) -> String {
    format!(
        "A hand-off from {route} could not be received: {cause}. Ask the sending app to send it again."
    )
}

/// Logs and shows [`arrival_failure`]'s message in a dialog that does not
/// block. Without the dialog plugin (tests), the log is all there is.
pub fn report_arrival_failure<R: Runtime>(app: &AppHandle<R>, route: &str, cause: &str) {
    let text = arrival_failure(route, cause);
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

/// The hand-off files among a command line's arguments: the program name,
/// `studyanalyzer:` links (the deep-link plugin's) and anything without the
/// hand-off extension are skipped, each skip logged. Any program can launch
/// the app with arguments, so a path is only ever read as a hand-off.
pub fn handoff_paths(args: &[String]) -> Vec<PathBuf> {
    let link_prefix = format!("{LINK_SCHEME}:");
    args.iter()
        .skip(1)
        .filter(|arg| {
            if arg.to_ascii_lowercase().starts_with(&link_prefix) {
                log::info!("argument {arg:?} skipped: a link, which the deep-link plugin takes");
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
            return report_arrival_failure(&app, route, "StudyAnalyzer did not start");
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
/// relative ones resolved against `cwd`.
pub fn receive_args<R: Runtime>(app: &AppHandle<R>, args: Vec<String>, cwd: &Path) {
    for path in handoff_paths(&args) {
        receive_file(app, cwd.join(path), "a command-line argument");
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
        assert_eq!(early.hold_unless_started(false, path.clone()), None);
        assert_eq!(
            early.hold_unless_started(true, path.clone()),
            Some(path.clone())
        );
        assert_eq!(early.take(), [path]);
        assert!(early.take().is_empty());
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

    /// A `file:` URL is a file, never a link; every other URL is a link.
    #[test]
    fn file_urls_are_not_links() {
        let file: tauri::Url = "file:///x/b.studyanalyzer".parse().unwrap();
        let link: tauri::Url = "studyanalyzer://open?pmid=1".parse().unwrap();
        assert!(!is_link(&file));
        assert!(is_link(&link));
    }
}
