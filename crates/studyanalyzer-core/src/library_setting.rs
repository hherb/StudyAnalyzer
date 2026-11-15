//! The Library setting (design `2026-11-13-shared-paper-library-design.md`,
//! §4.1, §4.6, §4.7): what the setting shows ([`status`]), choosing a folder
//! ([`choose`]), going back to no library ([`clear`]) and moving the papers
//! the app keeps into the library later ([`move_in`]). [`choose`] and
//! [`move_in`] re-mark rows in the store, then [`settle`] the waiting ones;
//! [`clear`] keeps waiting rows in the app. The old folder is never changed,
//! and a library source whose file a change leaves there notes that folder.
//! A paper whose file could not be copied or moved is named in
//! [`Changed::failed`].

use std::fmt;
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bmlib::db::{transaction_with, Db};
use bmlib::library::{Library, MARKER_FILE};
use serde::Serialize;

use crate::bookmark;
use crate::config::{
    LIBRARY_BOOKMARK_FAULT_KEY, LIBRARY_BOOKMARK_KEY, LIBRARY_PATH_KEY, SANDBOX_ENV, TRASH_FOLDERS,
};
use crate::intake::SourceKind;
use crate::library::{
    configured_with, installation, opened_before, read_scanned, remember_opened, settle,
    settle_into, Configured, LibraryAt, LibraryFault, Settled,
};
use crate::store::{
    app_to_waiting, bring_back, clear_library_note, count_by_place, delete_setting, get_setting,
    mark_in_library, note_left_behind, note_library, paper_title, papers_with_place, set_setting,
    source_by_id, sources_with_place, waiting_sources, waiting_to_app, PaperId, PlaceCounts,
    SourcePlace, StoreError, StoredSource,
};

/// A source the setting lists: one still waiting for the library, or one
/// whose file was left in another library folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitingSource {
    /// Its paper.
    pub paper_id: String,
    /// Its paper's title.
    pub title: String,
    /// What kind of source it is.
    pub kind: SourceKind,
    /// Why it is not in the library yet, once a move has failed; for one
    /// left behind, a phrase naming the folder its file is in.
    pub note: Option<String>,
}

/// What the Library setting shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryStatus {
    /// The library folder set, if any (a label: the webview never passes it back).
    pub path: Option<String>,
    /// Why the library set cannot be used now, if it cannot.
    pub unreachable: Option<String>,
    /// How many sources are kept at each place.
    pub counts: PlaceCounts,
    /// Every source waiting for the library, oldest first.
    pub waiting: Vec<WaitingSource>,
    /// Every library source whose file a change of the setting left in
    /// another folder, oldest first; its note names that folder.
    pub elsewhere: Vec<WaitingSource>,
    /// What the launch learnt of the folder but could not keep or reach, if
    /// anything (#130), while the library is reachable; an unreachable
    /// library's reason in `unreachable` ends with it instead (#134).
    pub launch_note: Option<String>,
}

/// A paper whose file a change could not copy or move, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaperProblem {
    /// The paper.
    pub paper_id: String,
    /// Its title.
    pub title: String,
    /// Why, as a phrase for the user.
    pub why: String,
}

/// What a change of the setting did. The two counts never overlap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Changed {
    /// Files read back from the old library (into the new one, or the app).
    pub copied: usize,
    /// Files kept in the app (waiting, or moved on purpose) that went into
    /// the library.
    pub moved: usize,
    /// One entry per file that could not be copied or moved.
    pub failed: Vec<PaperProblem>,
}

/// Why a change of the setting was refused or did not finish.
#[derive(Debug)]
pub enum SettingError {
    /// The database failed before the setting changed (its notes of files
    /// left behind are taken back; a file already copied stays copied).
    Store(StoreError),
    /// The folder chosen cannot be a library; why, naming it.
    Refused(String),
    /// The change needs a usable library and there is none; why.
    Unreachable(String),
    /// The library refused what was asked of it (a list, a removal); a
    /// sentence for the user saying what and why, without its final full
    /// stop (`Display` adds it).
    Library(String),
    /// The setting changed to `path`, then the database failed; what the
    /// change had done by then.
    Switched {
        /// The library folder now set.
        path: String,
        /// What was copied, moved or not, before the failure.
        changed: Changed,
        /// The database failure.
        error: StoreError,
    },
}

impl fmt::Display for SettingError {
    /// A message the user can read.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingError::Store(e) => write!(f, "{e}"),
            SettingError::Refused(why) => {
                write!(f, "That folder cannot be the paper library: {why}.")
            }
            SettingError::Unreachable(why) => {
                write!(f, "The paper library cannot be used now: {why}.")
            }
            SettingError::Library(why) => write!(f, "{why}."),
            SettingError::Switched {
                path,
                changed,
                error,
            } => {
                write!(
                    f,
                    "The paper library is now {path}, but then the app's storage failed ({error})."
                )?;
                let mut done = Vec::new();
                if changed.copied > 0 {
                    done.push(format!("{} copied", files(changed.copied)));
                }
                if changed.moved > 0 {
                    done.push(format!("{} moved into it", files(changed.moved)));
                }
                if !done.is_empty() {
                    write!(f, " By then {}.", done.join(" and "))?;
                }
                if !changed.failed.is_empty() {
                    write!(
                        f,
                        " {} could not be copied or moved:",
                        files(changed.failed.len())
                    )?;
                    for p in &changed.failed {
                        write!(f, " {}: {}.", p.title, p.why)?;
                    }
                }
                write!(
                    f,
                    " Files still waiting move in when the app is next opened or brought to the \
                     front; any still kept in the app can be moved from this setting."
                )
            }
        }
    }
}

/// `n` file(s), as the user reads it: "1 file", "2 files".
fn files(n: usize) -> String {
    if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    }
}

impl std::error::Error for SettingError {}

impl From<StoreError> for SettingError {
    /// Wraps a store error.
    fn from(e: StoreError) -> Self {
        SettingError::Store(e)
    }
}

/// The setting as it stands: the folder, whether it can be used, the counts
/// by place, the sources still waiting, and the library sources whose file
/// a change left in another folder. `label` names this installation in the
/// library's records (as [`configured_with`] takes it). `launch_note` is what
/// [`resume`] could not keep or reach this launch ([`Resumed::Held`]'s
/// `unsaved`, a [`Resumed::Lost`] reason, or why `resume` failed), held in
/// memory: it is learnt only at launch, and some of it is what the database
/// failed to keep. An
/// unreachable library's reason ends with it ([`configured_with`]), so the
/// status carries it as `launch_note` only while the library is reachable.
pub fn status(
    db: &mut dyn Db,
    label: &str,
    launch_note: Option<&str>,
) -> Result<LibraryStatus, StoreError> {
    let unreachable = match configured_with(db, label, launch_note)? {
        Configured::Unreachable(why) => Some(why),
        Configured::None | Configured::Ready(_) => None,
    };
    let launch_note = launch_note.filter(|_| unreachable.is_none());
    let waiting = waiting_sources(db, None)?;
    let waiting = listed(db, waiting)?;
    let left = sources_with_place(db, None, SourcePlace::Library)?
        .into_iter()
        .filter(|src| src.library_note.is_some())
        .collect();
    Ok(LibraryStatus {
        path: get_setting(db, LIBRARY_PATH_KEY)?,
        unreachable,
        counts: count_by_place(db)?,
        waiting,
        elsewhere: listed(db, left)?,
        launch_note: launch_note.map(str::to_string),
    })
}

/// `sources` as the setting lists them, each with its paper's title.
fn listed(db: &mut dyn Db, sources: Vec<StoredSource>) -> Result<Vec<WaitingSource>, StoreError> {
    let mut out = Vec::new();
    for src in sources {
        out.push(WaitingSource {
            paper_id: src.paper_id.as_str().to_string(),
            title: paper_title(db, &src.paper_id)?,
            kind: src.kind,
            note: src.library_note,
        });
    }
    Ok(out)
}

/// The note a library source gets when a change leaves its file in `old`.
fn left_note(old: &str) -> String {
    format!("its file was left in the library folder {old}")
}

/// Makes `root` the library (an empty folder becomes one; a folder of other
/// files is [`SettingError::Refused`], nothing changed). A folder this app
/// opened before that has lost its marker (an unplugged drive's mount
/// point) is refused too, not made a new library. With `bring`, the papers
/// come along: from no library, every source kept in the app moves in
/// (§4.7); from another library, this installation's files there are copied
/// in, one paper at a time, and the setting changes only after the copy, so
/// an interruption leaves the old setting with every row still readable.
/// Waiting sources then move into the new library. Declined or failed, a
/// file the new library lacks reads as unavailable (§4.7), its row noting
/// the old folder; from an old library that cannot be reached each such
/// file is named in [`Changed::failed`]. Choosing the folder already set
/// copies nothing. The old folder is never changed. A database failure after
/// the setting changed is [`SettingError::Switched`], keeping what was done.
/// `launch_note` is the launch's note ([`status`]), ending the reason an
/// unreachable old library gives each file it cannot copy.
pub fn choose(
    db: &mut dyn Db,
    label: &str,
    launch_note: Option<&str>,
    root: &Path,
    bring: bool,
) -> Result<Changed, SettingError> {
    let path = root.to_str().ok_or_else(|| {
        SettingError::Refused(format!(
            "the name of {} is not text the app can keep",
            root.display()
        ))
    })?;
    if opened_before(db, path)? && root.is_dir() && !root.join(MARKER_FILE).exists() {
        log::warn!(
            "the folder {path} has lost the library's marker; it is not made a library anew"
        );
        return Err(SettingError::Refused(format!(
            "{path} no longer holds the paper library (is its drive unplugged?), so it is not made a new one"
        )));
    }
    if in_trash(root) {
        log::warn!("the folder {path} is in the Trash; it is not made the library");
        return Err(SettingError::Refused(format!(
            "{path} is in the Trash, and emptying the Trash would delete the library"
        )));
    }
    // Bookmarked first: a refused bookmark leaves the folder without a marker.
    let mark = bookmark_of(root).map_err(|e| {
        log::warn!("the folder {path} cannot be bookmarked: {e}");
        SettingError::Refused(format!("the folder {path} cannot be remembered: {e}"))
    })?;
    let library = Library::open(root).map_err(|e| {
        log::warn!("the folder {path} is refused as the library: {e}");
        SettingError::Refused(e.to_string())
    })?;
    let new = LibraryAt {
        library,
        who: installation(db, label)?,
    };
    let old = configured_with(db, label, launch_note)?;
    let old_path = get_setting(db, LIBRARY_PATH_KEY)?;
    let mut changed = Changed::default();
    let left = old_path.as_deref().filter(|&p| p != path).map(left_note);
    let switched = (|| {
        if let Some(left) = &left {
            leave(db, &old, Some(&new), left, bring, &mut changed)?;
        }
        transaction_with(db, |tx| {
            set_setting(tx, LIBRARY_PATH_KEY, path)?;
            match &mark {
                Some(mark) => set_setting(tx, LIBRARY_BOOKMARK_KEY, mark)?,
                None => delete_setting(tx, LIBRARY_BOOKMARK_KEY)?,
            }
            delete_setting(tx, LIBRARY_BOOKMARK_FAULT_KEY)?;
            // Files left in this folder by an earlier change are in the library again.
            clear_library_note(tx, &left_note(path))?;
            remember_opened(tx, path)
        })
    })();
    if let Err(e) = switched {
        take_back(db, left.as_deref());
        return Err(e.into());
    }
    log::info!("the paper library is now {path}");
    let mut after_switch = || {
        if bring && matches!(old, Configured::None) {
            let n = app_to_waiting(db)?;
            log::info!("{n} source(s) kept in the app are to move into the library");
        }
        settle_all(db, &new, &mut changed)
    };
    match after_switch() {
        Ok(()) => Ok(changed),
        Err(error) => Err(SettingError::Switched {
            path: path.to_string(),
            changed,
            error,
        }),
    }
}

/// Sets no library. With `bring`, this installation's files in the old
/// library are copied back into the app first; one that cannot be read is
/// named. Without `bring`, or failed, a source in the library reads as
/// unavailable until that folder is chosen again, its row noting the
/// folder. Waiting sources stay in the app for good. `launch_note` is as
/// [`choose`] takes it.
pub fn clear(
    db: &mut dyn Db,
    label: &str,
    launch_note: Option<&str>,
    bring: bool,
) -> Result<Changed, SettingError> {
    let mut changed = Changed::default();
    let old = configured_with(db, label, launch_note)?;
    let left = get_setting(db, LIBRARY_PATH_KEY)?.as_deref().map(left_note);
    let cleared = (|| {
        if let Some(left) = &left {
            leave(db, &old, None, left, bring, &mut changed)?;
        }
        transaction_with(db, |tx| {
            let kept = waiting_to_app(tx)?;
            delete_setting(tx, LIBRARY_PATH_KEY)?;
            delete_setting(tx, LIBRARY_BOOKMARK_KEY)?;
            delete_setting(tx, LIBRARY_BOOKMARK_FAULT_KEY)?;
            Ok::<_, StoreError>(kept)
        })
    })();
    let kept = match cleared {
        Ok(kept) => kept,
        Err(e) => {
            take_back(db, left.as_deref());
            return Err(e.into());
        }
    };
    log::info!("no paper library is set now; {kept} waiting source(s) stay in the app");
    Ok(changed)
}

/// The first part of a change away from the old library `old`: every
/// library source without a note takes the note `left` (its file stays in
/// the old folder), then with `bring` this installation's files there are
/// copied out (into `new`, or back into the app) or, from an old library
/// that cannot be reached, each is named in `changed`.
fn leave(
    db: &mut dyn Db,
    old: &Configured,
    new: Option<&LibraryAt>,
    left: &str,
    bring: bool,
    changed: &mut Changed,
) -> Result<(), StoreError> {
    let n = note_left_behind(db, left)?;
    log::info!("{n} library source(s) noted: {left}");
    if bring {
        match old {
            Configured::None => {}
            Configured::Ready(old) => copy_out(db, old, new, left, changed)?,
            Configured::Unreachable(why) => name_each(db, why, changed)?,
        }
    }
    Ok(())
}

/// Takes back the note `left` after a change failed before the setting
/// moved: the folder it names is still the library. A failure to do so is
/// logged; the note then stays until that folder is chosen again.
fn take_back(db: &mut dyn Db, left: Option<&str>) {
    let Some(left) = left else { return };
    match clear_library_note(db, left) {
        Ok(n) => log::info!("the change failed; {n} library source(s) no longer note: {left}"),
        Err(e) => log::error!("the change failed, and its notes ({left}) stay: {e}"),
    }
}

/// The library set, when it can be used; otherwise
/// [`SettingError::Unreachable`], saying why (ending with `launch_note`, the
/// launch's note, as [`configured_with`] does).
pub(crate) fn ready(
    db: &mut dyn Db,
    label: &str,
    launch_note: Option<&str>,
) -> Result<LibraryAt, SettingError> {
    match configured_with(db, label, launch_note)? {
        Configured::Ready(lib) => Ok(lib),
        Configured::None => Err(SettingError::Unreachable(
            "no library folder is set".to_string(),
        )),
        Configured::Unreachable(why) => Err(SettingError::Unreachable(why)),
    }
}

/// Moves every source kept in the app into the library set (§4.7, run from
/// the setting). Without a usable library it is [`SettingError::Unreachable`]
/// (its reason ending with `launch_note`, see [`ready`]).
pub fn move_in(
    db: &mut dyn Db,
    label: &str,
    launch_note: Option<&str>,
) -> Result<Changed, SettingError> {
    let lib = ready(db, label, launch_note)?;
    let n = app_to_waiting(db)?;
    log::info!("{n} source(s) kept in the app are to move into the library");
    let mut changed = Changed::default();
    settle_all(db, &lib, &mut changed)?;
    Ok(changed)
}

/// Reads every source kept in the `old` library back, paper by paper, from
/// one scan of it: with `new`, as waiting, then settled into it, so the
/// database holds one paper's bytes at a time; without, into the app. A
/// source whose file cannot be read, or is damaged, is named and left as it
/// was; a database failure stops the copy (the setting has not changed, so
/// every row still reads). One the new library does not take goes back to
/// the old one (whose file is still there, as the setting has not switched
/// yet), its row noting `left`, so the bytes never pile up in the
/// database, and is named.
fn copy_out(
    db: &mut dyn Db,
    old: &LibraryAt,
    new: Option<&LibraryAt>,
    left: &str,
    changed: &mut Changed,
) -> Result<(), StoreError> {
    let scan = match old.library.scan() {
        Ok(scan) => scan,
        Err(e) => {
            log::warn!("the old library cannot be read to copy from: {e}");
            return name_each(db, &e.to_string(), changed);
        }
    };
    let place = if new.is_some() {
        SourcePlace::Waiting
    } else {
        SourcePlace::App
    };
    for paper in papers_with_place(db, SourcePlace::Library)? {
        let mut brought = Vec::new();
        for src in sources_with_place(db, Some(&paper), SourcePlace::Library)? {
            // Err(why): a file that cannot be read, or whose bytes are not its digest's.
            let kept = match read_scanned(old, &scan, &src) {
                Ok(bytes) => match transaction_with(db, |tx| bring_back(tx, src.id, &bytes, place))
                {
                    Ok(kept) => Ok(kept),
                    Err(StoreError::Corrupt(why)) => Err(why),
                    Err(e) => return Err(e),
                },
                Err(LibraryFault::Unavailable(why)) => Err(why),
                Err(LibraryFault::Store(e)) => return Err(e),
            };
            match kept {
                Ok(true) => {
                    changed.copied += 1;
                    brought.push(src.id);
                }
                Ok(false) => log::info!("source {} left the library meanwhile", src.id),
                Err(why) => {
                    log::warn!(
                        "source {} of paper {} is not copied out of the old library: {why}",
                        src.id,
                        paper.as_str()
                    );
                    changed.failed.push(problem(db, &paper, why)?);
                }
            }
        }
        let Some(new) = new else { continue };
        let settled = settle(db, new, Some(&paper))?.moved;
        let mut copies_settled = brought.len();
        for id in brought {
            let src = source_by_id(db, id)?;
            if src.place != SourcePlace::Waiting {
                continue;
            }
            copies_settled -= 1;
            let why = src
                .library_note
                .unwrap_or_else(|| "the new library did not take it".to_string());
            let back = transaction_with(db, |tx| {
                let back = mark_in_library(tx, id)?;
                if back {
                    note_library(tx, id, left)?;
                }
                Ok::<_, StoreError>(back)
            })?;
            if !back {
                log::warn!("source {id} stopped waiting meanwhile; it is not put back");
                continue;
            }
            changed.copied -= 1;
            log::warn!("source {id} stays in the old library only: {why}");
            changed
                .failed
                .push(problem(db, &paper, format!("{left} only: {why}"))?);
        }
        // The paper's own waiting sources settled with the copies; count only those.
        changed.moved += settled.saturating_sub(copies_settled);
    }
    Ok(())
}

/// Adds one failed entry per source kept in the old library, which cannot
/// be reached (`why`).
fn name_each(db: &mut dyn Db, why: &str, changed: &mut Changed) -> Result<(), StoreError> {
    for paper in papers_with_place(db, SourcePlace::Library)? {
        for _ in sources_with_place(db, Some(&paper), SourcePlace::Library)? {
            let why = format!("its file could not be copied from the old library: {why}");
            changed.failed.push(problem(db, &paper, why)?);
        }
    }
    Ok(())
}

/// Moves every waiting source into `lib`, adding what moved and what is left
/// to `changed`, also when the database fails partway.
fn settle_all(db: &mut dyn Db, lib: &LibraryAt, changed: &mut Changed) -> Result<(), StoreError> {
    let mut settled = Settled::default();
    let result = settle_into(db, lib, None, &mut settled);
    changed.moved += settled.moved;
    for (paper, why) in settled.left {
        changed.failed.push(problem(db, &paper, why)?);
    }
    result
}

/// A [`PaperProblem`] for `paper`, with its title.
fn problem(db: &mut dyn Db, paper: &PaperId, why: String) -> Result<PaperProblem, StoreError> {
    Ok(PaperProblem {
        paper_id: paper.as_str().to_string(),
        title: paper_title(db, paper)?,
        why,
    })
}

/// What [`resume`] found at launch.
pub enum Resumed {
    /// No bookmark is needed (no library, or not macOS).
    Nothing,
    /// The library folder's access, started; hold it until exit.
    Held {
        /// The access.
        access: bookmark::Access,
        /// What this launch learnt but could not keep, the launch's note
        /// (#130): [`status`]'s `launch_note` while the library is
        /// reachable, else the end of its unreachable reason (#134). It is a
        /// stale bookmark not renewed, a moved folder's name the app cannot
        /// keep, or a failed write of a bookmark, a moved path or the
        /// clearing of a fault.
        unsaved: Option<String>,
    },
    /// The folder cannot be reached through its bookmark (it did not
    /// resolve, resolved into the Trash, or a sandbox granted no access).
    Lost {
        /// Why, as a phrase for the user; the launch holds it as its note
        /// ([`status`]), as written or not (#132, #134).
        why: String,
    },
}

/// At launch, on macOS: resolves the library's bookmark and starts access to
/// its folder (design §4.1). A folder moved or renamed since is followed (its
/// new path stored), a stale bookmark is made again, and a library set before
/// bookmarks were kept gets one while its path can still be reached. A
/// bookmark that does not resolve, resolves into the Trash, or whose access
/// a sandbox did not grant is [`Resumed::Lost`], whose reason the launch
/// holds as its note (also stored in [`LIBRARY_BOOKMARK_FAULT_KEY`], see
/// `lost`). Once access has started, what is learnt is written in one transaction; a failed write,
/// or a stale bookmark that cannot be made again, is logged and the access
/// still [`Resumed::Held`], with why in its `unsaved`.
pub fn resume(db: &mut dyn Db) -> Result<Resumed, StoreError> {
    if !cfg!(target_os = "macos") {
        return Ok(Resumed::Nothing);
    }
    let Some(path) = get_setting(db, LIBRARY_PATH_KEY)? else {
        return Ok(Resumed::Nothing);
    };
    let stored = get_setting(db, LIBRARY_BOOKMARK_KEY)?;
    let data = match &stored {
        Some(mark) => STANDARD
            .decode(mark)
            .map_err(|e| format!("its stored bookmark is damaged: {e}")),
        None => {
            log::info!("the library {path} has no bookmark yet; making one");
            bookmark::create(Path::new(&path))
                .map_err(|e| format!("no bookmark was kept for it ({e})"))
        }
    };
    let (data, resolved) = match data.and_then(|d| bookmark::resolve(&d).map(|r| (d, r))) {
        Ok(found) => found,
        Err(why) => return Ok(lost(db, &path, format!("{why}; choose the folder again"))),
    };
    if in_trash(&resolved.path) {
        let why = format!(
            "the folder is in the Trash ({}); put it back, or choose another folder",
            resolved.path.display()
        );
        return Ok(lost(db, &path, why));
    }
    if access_refused(resolved.started, std::env::var_os(SANDBOX_ENV).is_some()) {
        let why = "access to the folder was not granted; choose the folder again".to_string();
        return Ok(lost(db, &path, why));
    }
    let mut unsaved = Vec::new();
    let renewed = if resolved.stale {
        bookmark::create(&resolved.path)
            .inspect_err(|e| {
                log::warn!("the stale bookmark of the library {path} was not renewed: {e}");
                unsaved.push(format!(
                    "the folder's bookmark is out of date and could not be made again ({e}); \
                     choose the folder again so the next launch can reach it"
                ));
            })
            .ok()
    } else {
        None
    };
    // A bookmark made here for a path set before bookmarks were kept is stored too.
    let mark = renewed.or_else(|| stored.is_none().then_some(data));
    let now = if moved(&path, &resolved.path) {
        let now = resolved.path.to_str();
        match now {
            Some(now) => log::info!("the library folder {path} has moved to {now}; following it"),
            None => {
                log::warn!(
                    "the library folder {path} has moved to {}, a name the app cannot keep",
                    resolved.path.display()
                );
                unsaved.push(format!(
                    "the folder moved to {}, a name the app cannot keep; rename it or \
                     choose another folder",
                    resolved.path.display()
                ));
            }
        }
        now
    } else {
        None
    };
    // Access has started: a failed write is logged and the access still held.
    let kept = transaction_with(db, |tx| {
        if let Some(mark) = &mark {
            set_setting(tx, LIBRARY_BOOKMARK_KEY, &STANDARD.encode(mark))?;
        }
        if let Some(now) = now {
            set_setting(tx, LIBRARY_PATH_KEY, now)?;
        }
        delete_setting(tx, LIBRARY_BOOKMARK_FAULT_KEY)
    });
    if let Err(e) = kept {
        log::error!(
            "what the launch learnt of the library {path} was not kept (now at {}; \
             bookmark to store: {}): {e}",
            resolved.path.display(),
            mark.is_some()
        );
        unsaved.push(match now {
            Some(now) => format!(
                "the folder was found at {now}, but the app could not save that ({e}); \
                 choose the folder again"
            ),
            None => format!("what the app learnt of the folder could not be saved ({e})"),
        });
    }
    Ok(Resumed::Held {
        access: resolved.access,
        unsaved: (!unsaved.is_empty()).then(|| unsaved.join("; ")),
    })
}

/// Whether access to the library is lost: the system did not start it
/// (`started` false) in a sandboxed process, which needs the grant.
/// Unsandboxed, a failed start is harmless.
fn access_refused(started: bool, sandboxed: bool) -> bool {
    !started && sandboxed
}

/// [`Resumed::Lost`] with `why`, also kept in [`LIBRARY_BOOKMARK_FAULT_KEY`]
/// for a reader without the launch's note ([`configured`]). A failure to
/// keep it is only logged: the launch holds `why` as its note either way.
fn lost(db: &mut dyn Db, path: &str, why: String) -> Resumed {
    log::warn!("the library {path} cannot be reached through its bookmark: {why}");
    if let Err(e) = set_setting(db, LIBRARY_BOOKMARK_FAULT_KEY, &why) {
        log::error!("why the library {path} cannot be reached was not kept: {e}");
    }
    Resumed::Lost { why }
}

/// Whether `path` lies in a Trash folder ([`TRASH_FOLDERS`]).
fn in_trash(path: &Path) -> bool {
    path.components()
        .any(|c| TRASH_FOLDERS.iter().any(|t| c.as_os_str() == *t))
}

/// Whether the folder stored as `stored` is not `now` (`/var` and
/// `/private/var` are one folder).
fn moved(stored: &str, now: &Path) -> bool {
    let stored = Path::new(stored);
    stored != now && stored.canonicalize().ok() != now.canonicalize().ok()
}

/// The bookmark of the folder `root` to store (base64), on macOS; `None`
/// elsewhere, where the path alone is kept.
fn bookmark_of(root: &Path) -> Result<Option<String>, String> {
    if cfg!(target_os = "macos") {
        bookmark::create(root).map(|data| Some(STANDARD.encode(data)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identifiers::Identifiers;
    use crate::library::configured;
    use crate::store::{create_paper, open_memory_store, save_source, source_by_id, NewSource};
    use bmlib::library::{find_digest, sha256_hex};
    use rusqlite::Connection;
    use std::path::PathBuf;

    const LABEL: &str = "StudyAnalyzer on a test";

    /// A fresh, empty folder under the temp dir.
    fn temp_folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sa-libset-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A paper with PMID `pmid` and one plain-text source of `bytes` at `place`.
    fn paper(db: &mut Connection, pmid: &str, bytes: &[u8], place: SourcePlace) -> (PaperId, i64) {
        let ids = Identifiers {
            pmid: Some(pmid.into()),
            ..Default::default()
        };
        let p = create_paper(db, &ids, Some(&format!("Trial {pmid}"))).unwrap();
        let src = NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: bytes.to_vec(),
            decoded: None,
        };
        (p.clone(), save_source(db, &p, src, place).unwrap())
    }

    /// Whether the library at `root` holds a file with `bytes`.
    fn holds(root: &Path, bytes: &[u8]) -> bool {
        let scan = Library::open(root).unwrap().scan().unwrap();
        find_digest(&scan, &sha256_hex(bytes), None).is_some()
    }

    /// The place of source `id`.
    fn place(db: &mut Connection, id: i64) -> SourcePlace {
        source_by_id(db, id).unwrap().place
    }

    /// A database whose library is `a`, holding two papers' files there.
    fn in_library_a() -> (Connection, PathBuf, [i64; 2]) {
        let mut db = open_memory_store().unwrap();
        let a = temp_folder();
        let (_, s1) = paper(&mut db, "1", b"one", SourcePlace::App);
        let (_, s2) = paper(&mut db, "2", b"two", SourcePlace::App);
        let c = choose(&mut db, LABEL, None, &a, true).unwrap();
        assert_eq!((c.moved, c.failed.len()), (2, 0));
        (db, a, [s1, s2])
    }

    /// A folder of the user's own files is refused, and nothing changes.
    #[test]
    fn choose_refuses_a_folder_of_user_files() {
        let mut db = open_memory_store().unwrap();
        let (_, s) = paper(&mut db, "1", b"one", SourcePlace::App);
        let dir = temp_folder();
        std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
        let e = choose(&mut db, LABEL, None, &dir, true).unwrap_err();
        assert!(
            matches!(&e, SettingError::Refused(why) if why.contains("notes.txt")),
            "{e}"
        );
        assert_eq!(get_setting(&mut db, LIBRARY_PATH_KEY).unwrap(), None);
        assert_eq!(place(&mut db, s), SourcePlace::App);
        std::fs::remove_dir_all(dir).ok();
    }

    /// From no library, the app's sources move in only when asked.
    #[test]
    fn choose_from_none_moves_app_sources_when_asked() {
        let mut db = open_memory_store().unwrap();
        let (_, s) = paper(&mut db, "1", b"one", SourcePlace::App);
        let dir = temp_folder();
        let c = choose(&mut db, LABEL, None, &dir, false).unwrap();
        assert_eq!(c, Changed::default());
        assert_eq!(place(&mut db, s), SourcePlace::App);
        assert_eq!(
            get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().as_deref(),
            dir.to_str()
        );
        std::fs::remove_dir_all(&dir).ok();

        let mut db = open_memory_store().unwrap();
        let (_, s) = paper(&mut db, "1", b"one", SourcePlace::App);
        let dir = temp_folder();
        let c = choose(&mut db, LABEL, None, &dir, true).unwrap();
        assert_eq!((c.copied, c.moved, c.failed.len()), (0, 1, 0));
        assert_eq!(place(&mut db, s), SourcePlace::Library);
        assert!(holds(&dir, b"one"));
        std::fs::remove_dir_all(dir).ok();
    }

    /// A switch with `bring` copies this installation's files into the new
    /// folder and leaves the old one as it was.
    #[test]
    fn switch_copies_this_installations_files() {
        let (mut db, a, [s1, s2]) = in_library_a();
        let b = temp_folder();
        let c = choose(&mut db, LABEL, None, &b, true).unwrap();
        assert_eq!((c.copied, c.moved, c.failed.len()), (2, 0, 0), "{c:?}");
        assert!(holds(&b, b"one") && holds(&b, b"two"));
        assert!(holds(&a, b"one") && holds(&a, b"two"));
        for s in [s1, s2] {
            let src = source_by_id(&mut db, s).unwrap();
            assert_eq!((src.place, src.library_note), (SourcePlace::Library, None));
            assert_eq!(crate::store::source_bytes(&mut db, s).unwrap(), None);
        }
        assert_eq!(
            get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().as_deref(),
            b.to_str()
        );
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    /// A switch from a library that cannot be reached still switches, and
    /// names every paper whose file it could not copy; it claims no copy.
    #[test]
    fn switch_with_old_unreachable_names_each_paper() {
        let (mut db, a, [s1, _]) = in_library_a();
        std::fs::remove_dir_all(&a).unwrap();
        let b = temp_folder();
        let c = choose(&mut db, LABEL, None, &b, true).unwrap();
        assert_eq!((c.copied, c.moved), (0, 0));
        let titles: Vec<&str> = c.failed.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, ["Trial 1", "Trial 2"]);
        assert!(c.failed[0].why.contains("old library"), "{c:?}");
        assert_eq!(place(&mut db, s1), SourcePlace::Library);
        assert_eq!(
            get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().as_deref(),
            b.to_str()
        );
        std::fs::remove_dir_all(b).ok();
    }

    /// A new library that refuses every file leaves each copied row in the
    /// old one (no bytes pile up in the database) and names each paper.
    #[cfg(unix)]
    #[test]
    fn switch_into_a_refusing_library_keeps_rows_in_the_old_one() {
        use std::os::unix::fs::PermissionsExt;
        let (mut db, a, [s1, s2]) = in_library_a();
        let b = temp_folder();
        Library::open(&b).unwrap();
        let papers = b.join("papers");
        std::fs::set_permissions(&papers, std::fs::Permissions::from_mode(0o555)).unwrap();
        let c = choose(&mut db, LABEL, None, &b, true).unwrap();
        std::fs::set_permissions(&papers, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(c.moved, 0, "{c:?}");
        let titles: Vec<&str> = c.failed.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, ["Trial 1", "Trial 2"], "{c:?}");
        let old = a.to_str().unwrap();
        assert!(c.failed[0].why.contains(old), "{c:?}");
        for s in [s1, s2] {
            let src = source_by_id(&mut db, s).unwrap();
            assert_eq!(src.place, SourcePlace::Library);
            assert!(src.library_note.unwrap().contains(old));
            assert_eq!(crate::store::source_bytes(&mut db, s).unwrap(), None);
        }
        assert_eq!(count_by_place(&mut db).unwrap().waiting, 0);
        assert!(holds(&a, b"one") && holds(&a, b"two"));
        assert_eq!(
            get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().as_deref(),
            b.to_str()
        );
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    /// The library folder chosen again after it lost its marker (an
    /// unplugged drive's empty mount point) is refused, not made anew.
    #[test]
    fn choose_refuses_a_library_folder_that_lost_its_marker() {
        let (mut db, a, _) = in_library_a();
        std::fs::remove_dir_all(&a).unwrap();
        std::fs::create_dir_all(&a).unwrap();
        let e = choose(&mut db, LABEL, None, &a, false).unwrap_err();
        assert!(
            matches!(&e, SettingError::Refused(why) if why.contains("unplugged")),
            "{e}"
        );
        assert!(!a.join(bmlib::library::MARKER_FILE).exists());
        std::fs::remove_dir_all(a).ok();
    }

    /// Choosing the folder already set copies nothing and re-marks nothing.
    #[test]
    fn switch_to_the_same_folder_copies_nothing() {
        let (mut db, a, [s1, _]) = in_library_a();
        let c = choose(&mut db, LABEL, None, &a, true).unwrap();
        assert_eq!(c, Changed::default());
        assert_eq!(place(&mut db, s1), SourcePlace::Library);
        std::fs::remove_dir_all(a).ok();
    }

    /// Going to no library with `bring` copies what it can back into the
    /// app and names a file the old library no longer has; waiting sources
    /// stay in the app; the setting is gone.
    #[test]
    fn clear_brings_files_back_and_names_a_missing_one() {
        let (mut db, a, [s1, s2]) = in_library_a();
        let (_, w) = paper(&mut db, "3", b"three", SourcePlace::Waiting);
        let scan = Library::open(&a).unwrap().scan().unwrap();
        let two = find_digest(&scan, &sha256_hex(b"two"), None).unwrap();
        std::fs::remove_file(a.join("papers").join(&two.folder).join(&two.name)).unwrap();
        let c = clear(&mut db, LABEL, None, true).unwrap();
        assert_eq!(c.copied, 1);
        assert_eq!(c.failed.len(), 1);
        assert_eq!(c.failed[0].title, "Trial 2");
        assert_eq!(place(&mut db, s1), SourcePlace::App);
        assert_eq!(
            crate::store::source_bytes(&mut db, s1).unwrap(),
            Some(b"one".to_vec())
        );
        assert_eq!(place(&mut db, s2), SourcePlace::Library);
        assert_eq!(place(&mut db, w), SourcePlace::App);
        assert_eq!(get_setting(&mut db, LIBRARY_PATH_KEY).unwrap(), None);
        // A later read of the file left behind names the folder it was in.
        let src = source_by_id(&mut db, s2).unwrap();
        let e = crate::library::read_source(&mut db, &Configured::None, &src).unwrap_err();
        assert!(e.to_string().contains(a.to_str().unwrap()), "{e}");
        std::fs::remove_dir_all(a).ok();
    }

    /// Without `bring`, no library leaves every library row as it is, noting
    /// the folder its file is in; waiting rows stay in the app.
    #[test]
    fn clear_without_bring_notes_the_old_folder() {
        let (mut db, a, [s1, s2]) = in_library_a();
        let (_, w) = paper(&mut db, "3", b"three", SourcePlace::Waiting);
        let c = clear(&mut db, LABEL, None, false).unwrap();
        assert_eq!(c, Changed::default());
        for s in [s1, s2] {
            let src = source_by_id(&mut db, s).unwrap();
            assert_eq!(src.place, SourcePlace::Library);
            assert!(src.library_note.unwrap().contains(a.to_str().unwrap()));
        }
        assert_eq!(place(&mut db, w), SourcePlace::App);
        let s = status(&mut db, LABEL, None).unwrap();
        assert_eq!((s.path, s.counts.library, s.counts.app), (None, 2, 1));
        std::fs::remove_dir_all(a).ok();
    }

    /// Without `bring`, a switch copies nothing: rows stay in the library,
    /// noting the old folder, which is not touched; waiting sources settle
    /// into the new one.
    #[test]
    fn switch_without_bring_notes_the_old_folder() {
        let (mut db, a, [s1, _]) = in_library_a();
        let (_, w) = paper(&mut db, "3", b"three", SourcePlace::Waiting);
        let b = temp_folder();
        let c = choose(&mut db, LABEL, None, &b, false).unwrap();
        assert_eq!((c.copied, c.moved, c.failed.len()), (0, 1, 0), "{c:?}");
        let src = source_by_id(&mut db, s1).unwrap();
        assert_eq!(src.place, SourcePlace::Library);
        assert!(src.library_note.unwrap().contains(a.to_str().unwrap()));
        assert!(holds(&a, b"one") && !holds(&b, b"one"));
        assert_eq!(place(&mut db, w), SourcePlace::Library);
        assert!(holds(&b, b"three"));
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    /// The status lists files a switch left in the old folder; choosing that
    /// folder again clears their notes, so they are listed no more (#118).
    #[test]
    fn status_lists_files_left_in_another_folder() {
        let (mut db, a, [s1, s2]) = in_library_a();
        assert!(status(&mut db, LABEL, None).unwrap().elsewhere.is_empty());
        let b = temp_folder();
        choose(&mut db, LABEL, None, &b, false).unwrap();
        let st = status(&mut db, LABEL, None).unwrap();
        assert_eq!(st.elsewhere.len(), 2, "{st:?}");
        let note = st.elsewhere[0].note.as_deref().unwrap();
        assert!(note.contains(a.to_str().unwrap()), "{note}");
        choose(&mut db, LABEL, None, &a, false).unwrap();
        assert!(status(&mut db, LABEL, None).unwrap().elsewhere.is_empty());
        for s in [s1, s2] {
            assert_eq!(source_by_id(&mut db, s).unwrap().library_note, None);
        }
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    /// A database failure after the setting changed says so and keeps what
    /// the change had done (#118).
    #[test]
    fn a_failure_after_the_switch_keeps_what_was_done() {
        let mut db = open_memory_store().unwrap();
        paper(&mut db, "1", b"one", SourcePlace::App);
        db.execute_batch(
            "CREATE TRIGGER no_update BEFORE UPDATE ON sources BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
        let dir = temp_folder();
        let e = choose(&mut db, LABEL, None, &dir, true).unwrap_err();
        match &e {
            SettingError::Switched { path, changed, .. } => {
                assert_eq!(Some(path.as_str()), dir.to_str());
                assert_eq!(changed, &Changed::default());
            }
            other => panic!("not Switched: {other}"),
        }
        assert!(e.to_string().starts_with("The paper library is now"), "{e}");
        assert_eq!(
            get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().as_deref(),
            dir.to_str()
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A settle that fails partway still counts what moved before the
    /// failure, and the message names the paper it failed on.
    #[test]
    fn a_failure_partway_through_the_settle_counts_what_moved() {
        let mut db = open_memory_store().unwrap();
        paper(&mut db, "1", b"one", SourcePlace::App);
        let (_, s2) = paper(&mut db, "2", b"two", SourcePlace::App);
        db.execute_batch(&format!(
            "CREATE TRIGGER no_settle BEFORE UPDATE ON sources \
             WHEN OLD.id = {s2} AND OLD.place = 'waiting' \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;"
        ))
        .unwrap();
        let dir = temp_folder();
        let e = choose(&mut db, LABEL, None, &dir, true).unwrap_err();
        match &e {
            SettingError::Switched { changed, .. } => assert_eq!(changed.moved, 1, "{changed:?}"),
            other => panic!("not Switched: {other}"),
        }
        assert!(e.to_string().contains("1 file moved"), "{e}");
        assert!(holds(&dir, b"one"));
        std::fs::remove_dir_all(dir).ok();
    }

    /// A change that fails before the setting moves takes back its notes,
    /// so the library still set is not listed as another folder.
    #[test]
    fn a_failure_before_the_switch_takes_back_its_notes() {
        let (mut db, a, [s1, s2]) = in_library_a();
        db.execute_batch(
            "CREATE TRIGGER no_bring BEFORE UPDATE ON sources WHEN NEW.place != OLD.place \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
        let b = temp_folder();
        let e = choose(&mut db, LABEL, None, &b, true).unwrap_err();
        assert!(matches!(&e, SettingError::Store(_)), "{e}");
        let e = clear(&mut db, LABEL, None, true).unwrap_err();
        assert!(matches!(&e, SettingError::Store(_)), "{e}");
        assert_eq!(
            get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().as_deref(),
            a.to_str()
        );
        assert!(status(&mut db, LABEL, None).unwrap().elsewhere.is_empty());
        for s in [s1, s2] {
            assert_eq!(source_by_id(&mut db, s).unwrap().library_note, None);
        }
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    /// Every path that reports an unreachable library ends its reason with
    /// the launch's note, not with a stored fault an earlier launch left
    /// (#134): a move into it, and a switch or a clear with `bring` from it,
    /// whose failed entries carry the reason. While it is reachable, the
    /// status carries the note itself (#130).
    #[test]
    fn an_unreachable_library_ends_with_the_launch_note() {
        let (mut db, a, _) = in_library_a();
        let note = Some("this launch's");
        let st = status(&mut db, LABEL, note).unwrap();
        assert_eq!((st.unreachable, st.launch_note.as_deref()), (None, note));
        std::fs::remove_dir_all(&a).unwrap();
        set_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY, "earlier").unwrap();
        let ends =
            |why: &str| why.ends_with("; at launch: this launch's") && !why.contains("earlier");
        match move_in(&mut db, LABEL, note) {
            Err(SettingError::Unreachable(why)) => assert!(ends(&why), "{why}"),
            other => panic!("not unreachable: {other:?}"),
        }
        let b = temp_folder();
        let c = choose(&mut db, LABEL, note, &b, true).unwrap();
        assert_eq!(c.failed.len(), 2);
        assert!(c.failed.iter().all(|f| ends(&f.why)), "{:?}", c.failed);
        std::fs::remove_dir_all(b).ok();
        let (mut db, a, _) = in_library_a();
        std::fs::remove_dir_all(&a).unwrap();
        set_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY, "earlier").unwrap();
        let c = clear(&mut db, LABEL, note, true).unwrap();
        assert_eq!(c.failed.len(), 2);
        assert!(c.failed.iter().all(|f| ends(&f.why)), "{:?}", c.failed);
    }

    /// A switch with `bring` copies what it can read; one file gone from the
    /// old library is named and left in the library, without bytes.
    #[test]
    fn switch_names_a_file_the_old_library_lost() {
        let (mut db, a, [s1, s2]) = in_library_a();
        let scan = Library::open(&a).unwrap().scan().unwrap();
        let two = find_digest(&scan, &sha256_hex(b"two"), None).unwrap();
        std::fs::remove_file(a.join("papers").join(&two.folder).join(&two.name)).unwrap();
        let b = temp_folder();
        let c = choose(&mut db, LABEL, None, &b, true).unwrap();
        assert_eq!((c.copied, c.moved), (1, 0), "{c:?}");
        let titles: Vec<&str> = c.failed.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, ["Trial 2"]);
        assert!(holds(&b, b"one"));
        assert_eq!(source_by_id(&mut db, s1).unwrap().library_note, None);
        assert_eq!(place(&mut db, s2), SourcePlace::Library);
        assert_eq!(crate::store::source_bytes(&mut db, s2).unwrap(), None);
        assert_eq!(count_by_place(&mut db).unwrap().waiting, 0);
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    /// A folder opened before is still checked for its marker after the
    /// setting moved away from it, to another folder or to none.
    #[test]
    fn a_folder_left_and_chosen_again_must_keep_its_marker() {
        let (mut db, a, _) = in_library_a();
        let b = temp_folder();
        choose(&mut db, LABEL, None, &b, false).unwrap();
        clear(&mut db, LABEL, None, false).unwrap();
        for dir in [&a, &b] {
            std::fs::remove_dir_all(dir).unwrap();
            std::fs::create_dir_all(dir).unwrap();
            let e = choose(&mut db, LABEL, None, dir, false).unwrap_err();
            assert!(matches!(&e, SettingError::Refused(_)), "{e}");
            std::fs::remove_dir_all(dir).ok();
        }
    }

    /// The status names an unreachable library and lists each waiting source.
    #[test]
    fn status_lists_waiting_sources() {
        let mut db = open_memory_store().unwrap();
        let empty = status(&mut db, LABEL, None).unwrap();
        assert_eq!((empty.path, empty.unreachable), (None, None));
        let (_, w) = paper(&mut db, "1", b"one", SourcePlace::Waiting);
        crate::store::note_waiting(&mut db, w, "unplugged").unwrap();
        let gone = temp_folder().join("missing");
        set_setting(&mut db, LIBRARY_PATH_KEY, gone.to_str().unwrap()).unwrap();
        let s = status(&mut db, LABEL, None).unwrap();
        assert_eq!(s.path.as_deref(), gone.to_str());
        assert!(s.unreachable.as_deref().unwrap().contains("missing"));
        assert_eq!(s.counts.waiting, 1);
        assert_eq!(
            s.waiting,
            [WaitingSource {
                paper_id: s.waiting[0].paper_id.clone(),
                title: "Trial 1".into(),
                kind: SourceKind::PlainText,
                note: Some("unplugged".into()),
            }]
        );
    }

    /// Moving in later needs a usable library, then moves the app's sources.
    #[test]
    fn move_in_needs_a_library() {
        let mut db = open_memory_store().unwrap();
        let (_, s) = paper(&mut db, "1", b"one", SourcePlace::App);
        assert!(matches!(
            move_in(&mut db, LABEL, None),
            Err(SettingError::Unreachable(_))
        ));
        let gone = temp_folder().join("missing");
        set_setting(&mut db, LIBRARY_PATH_KEY, gone.to_str().unwrap()).unwrap();
        assert!(matches!(
            move_in(&mut db, LABEL, None),
            Err(SettingError::Unreachable(why)) if why.contains("missing")
        ));
        assert_eq!(place(&mut db, s), SourcePlace::App);
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        let c = move_in(&mut db, LABEL, None).unwrap();
        assert_eq!((c.moved, c.failed.len()), (1, 0));
        assert_eq!(place(&mut db, s), SourcePlace::Library);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The bookmark stored, decoded.
    #[cfg(target_os = "macos")]
    fn stored_bookmark(db: &mut Connection) -> Option<Vec<u8>> {
        use base64::Engine;
        get_setting(db, LIBRARY_BOOKMARK_KEY)
            .unwrap()
            .map(|b| base64::engine::general_purpose::STANDARD.decode(b).unwrap())
    }

    /// The same folder however it is spelled (`/var` is `/private/var`).
    #[cfg(target_os = "macos")]
    fn same(a: &Path, b: &Path) -> bool {
        a.canonicalize().unwrap() == b.canonicalize().unwrap()
    }

    /// Choosing a folder stores its bookmark beside the path.
    #[cfg(target_os = "macos")]
    #[test]
    fn choose_stores_a_bookmark() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        let data = stored_bookmark(&mut db).expect("a bookmark");
        assert!(same(&bookmark::resolve(&data).unwrap().path, &dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// No library: no bookmark either.
    #[cfg(target_os = "macos")]
    #[test]
    fn clear_removes_the_bookmark() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        set_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY, "old").unwrap();
        clear(&mut db, LABEL, None, false).unwrap();
        assert_eq!(stored_bookmark(&mut db), None);
        assert_eq!(
            get_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY).unwrap(),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A choice clears an earlier launch's fault.
    #[cfg(target_os = "macos")]
    #[test]
    fn choose_clears_a_fault() {
        let mut db = open_memory_store().unwrap();
        set_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY, "old").unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        assert_eq!(
            get_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY).unwrap(),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resume_without_a_library_is_nothing() {
        let mut db = open_memory_store().unwrap();
        assert!(matches!(resume(&mut db).unwrap(), Resumed::Nothing));
    }

    /// A library set before bookmarks were kept gets one at launch.
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_makes_a_bookmark_for_a_path_set_before() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        set_setting(&mut db, LIBRARY_PATH_KEY, dir.to_str().unwrap()).unwrap();
        assert!(matches!(resume(&mut db).unwrap(), Resumed::Held { .. }));
        assert!(stored_bookmark(&mut db).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A folder renamed between launches is followed.
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_follows_a_renamed_library() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        let moved = dir.with_extension("moved");
        std::fs::rename(&dir, &moved).unwrap();
        assert!(matches!(
            resume(&mut db).unwrap(),
            Resumed::Held { unsaved: None, .. }
        ));
        let path = get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().unwrap();
        assert!(same(Path::new(&path), &moved), "{path}");
        assert!(matches!(
            configured(&mut db, LABEL).unwrap(),
            Configured::Ready(_)
        ));
        std::fs::remove_dir_all(&moved).ok();
    }

    /// A failed write once access has started still holds the access, and
    /// the launch's writes are one: the moved path written before the
    /// refused delete of the fault is rolled back with it (#119); what was
    /// not kept is in `unsaved`, which the status shows (#130).
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_holds_the_access_when_its_writes_fail() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        set_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY, "earlier").unwrap();
        let moved = dir.with_extension("moved");
        std::fs::rename(&dir, &moved).unwrap();
        db.execute_batch(
            "CREATE TRIGGER no_delete BEFORE DELETE ON settings BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
        let unsaved = match resume(&mut db).unwrap() {
            Resumed::Held { unsaved, .. } => unsaved.expect("what was not kept"),
            _ => panic!("not held"),
        };
        assert!(unsaved.contains("could not save"), "{unsaved}");
        let name = moved.file_name().unwrap().to_str().unwrap();
        assert!(unsaved.contains(name), "{unsaved}");
        let st = status(&mut db, LABEL, Some(&unsaved)).unwrap();
        // The earlier fault, whose clearing was rolled back, is not read as
        // this launch's (#132); the note ends the reason instead (#134).
        let unreachable = st.unreachable.expect("the old path is gone");
        assert!(!unreachable.contains("earlier"), "{unreachable}");
        assert!(
            unreachable.ends_with(&format!("; at launch: {unsaved}")),
            "{unreachable}"
        );
        assert_eq!(st.launch_note, None);
        let path = get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().unwrap();
        assert_eq!(
            Some(path.as_str()),
            dir.to_str(),
            "the moved path rolled back"
        );
        assert_eq!(
            get_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY)
                .unwrap()
                .as_deref(),
            Some("earlier")
        );
        std::fs::remove_dir_all(&moved).ok();
    }

    /// A failed write when the folder did not move does not say it was found
    /// elsewhere (#130).
    #[cfg(target_os = "macos")]
    #[test]
    fn a_failed_write_in_place_does_not_claim_a_move() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        // The trigger fires only on a row, so there is a fault to clear.
        set_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY, "earlier").unwrap();
        db.execute_batch(
            "CREATE TRIGGER no_delete BEFORE DELETE ON settings BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
        let unsaved = match resume(&mut db).unwrap() {
            Resumed::Held { unsaved, .. } => unsaved.expect("what was not kept"),
            _ => panic!("not held"),
        };
        assert!(!unsaved.contains("found at"), "{unsaved}");
        assert!(unsaved.contains("refused"), "{unsaved}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Only a start that failed in a sandbox means the access is lost.
    #[test]
    fn a_failed_start_is_refused_only_in_a_sandbox() {
        assert!(access_refused(false, true));
        assert!(!access_refused(true, true));
        assert!(!access_refused(false, false));
        assert!(!access_refused(true, false));
    }

    /// A folder deleted between launches is lost, and the status says why.
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_of_a_deleted_folder_is_lost() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(resume(&mut db).unwrap(), Resumed::Lost { .. }));
        assert!(get_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY)
            .unwrap()
            .is_some());
        match configured(&mut db, LABEL).unwrap() {
            Configured::Unreachable(why) => {
                assert!(why.contains("; at launch: "), "{why}");
                assert!(why.contains("choose the folder again"), "{why}")
            }
            _ => panic!("not unreachable"),
        }
    }

    /// A folder moved to the Trash is not followed there: emptying the Trash
    /// would delete the library.
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_does_not_follow_a_library_into_the_trash() {
        let mut db = open_memory_store().unwrap();
        let home = temp_folder();
        let dir = home.join("Papers");
        std::fs::create_dir(&dir).unwrap();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        std::fs::create_dir(home.join(".Trash")).unwrap();
        std::fs::rename(&dir, home.join(".Trash/Papers")).unwrap();
        match resume(&mut db).unwrap() {
            Resumed::Lost { why, .. } => assert!(why.contains("Trash"), "{why}"),
            _ => panic!("not lost"),
        }
        assert_eq!(
            get_setting(&mut db, LIBRARY_PATH_KEY).unwrap().as_deref(),
            dir.to_str()
        );
        assert!(stored_bookmark(&mut db).is_some());
        std::fs::remove_dir_all(&home).ok();
    }

    /// Only a whole `.Trash` or `.Trashes` component is a Trash folder.
    #[test]
    fn in_trash_matches_whole_trash_components() {
        for p in [
            "/Users/me/.Trash/Papers",
            "/Volumes/Ext/.Trashes/501/Papers",
        ] {
            assert!(in_trash(Path::new(p)), "{p}");
        }
        for p in [
            "/Users/me/Trash/Papers",
            "/Users/me/.Trash-old/Papers",
            "/Users/me/Papers",
        ] {
            assert!(!in_trash(Path::new(p)), "{p}");
        }
    }

    /// A folder in the Trash is refused, and left without a library marker.
    #[test]
    fn choose_refuses_a_folder_in_the_trash() {
        let mut db = open_memory_store().unwrap();
        let home = temp_folder();
        let dir = home.join(".Trash/Papers");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(
            choose(&mut db, LABEL, None, &dir, false),
            Err(SettingError::Refused(_))
        ));
        assert!(!dir.join(MARKER_FILE).exists());
        assert_eq!(get_setting(&mut db, LIBRARY_PATH_KEY).unwrap(), None);
        std::fs::remove_dir_all(&home).ok();
    }

    /// A lost folder's reason the database refuses to keep is still the
    /// launch's note; with that note the status ends the unreachable reason
    /// with it, not with the earlier launch's fault (#132, #134).
    #[cfg(target_os = "macos")]
    #[test]
    fn a_lost_reason_not_kept_replaces_the_earlier_fault() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        set_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY, "earlier").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        db.execute_batch(
            "CREATE TRIGGER no_update BEFORE UPDATE ON settings BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
        let why = match resume(&mut db).unwrap() {
            Resumed::Lost { why } => why,
            _ => panic!("not lost"),
        };
        assert_eq!(
            get_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY).unwrap(),
            Some("earlier".into()),
            "the write was refused"
        );
        let st = status(&mut db, LABEL, Some(&why)).unwrap();
        let unreachable = st.unreachable.expect("unreachable");
        assert!(!unreachable.contains("earlier"), "{unreachable}");
        assert!(
            unreachable.ends_with(&format!("; at launch: {why}")),
            "{unreachable}"
        );
        assert_eq!(st.launch_note, None, "in the reason, not twice");
        // Without a note the stored fault is this launch's and is shown.
        let st = status(&mut db, LABEL, None).unwrap();
        assert!(st.unreachable.unwrap().ends_with("; at launch: earlier"));
    }

    /// A launch that reaches the folder clears an earlier launch's fault,
    /// so putting a library back from the Trash recovers it.
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_clears_an_earlier_fault() {
        let mut db = open_memory_store().unwrap();
        let home = temp_folder();
        let dir = home.join("Papers");
        std::fs::create_dir(&dir).unwrap();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        let trashed = home.join(".Trash/Papers");
        std::fs::create_dir(home.join(".Trash")).unwrap();
        std::fs::rename(&dir, &trashed).unwrap();
        assert!(matches!(resume(&mut db).unwrap(), Resumed::Lost { .. }));
        std::fs::rename(&trashed, &dir).unwrap();
        assert!(matches!(resume(&mut db).unwrap(), Resumed::Held { .. }));
        assert_eq!(
            get_setting(&mut db, LIBRARY_BOOKMARK_FAULT_KEY).unwrap(),
            None
        );
        std::fs::remove_dir_all(&home).ok();
    }

    /// A damaged stored bookmark is lost, and says the fix.
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_of_a_damaged_bookmark_says_choose_it_again() {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        choose(&mut db, LABEL, None, &dir, false).unwrap();
        set_setting(&mut db, LIBRARY_BOOKMARK_KEY, "not base64!").unwrap();
        match resume(&mut db).unwrap() {
            Resumed::Lost { why, .. } => {
                assert!(why.contains("damaged"), "{why}");
                assert!(why.contains("choose the folder again"), "{why}")
            }
            _ => panic!("not lost"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A library set before bookmarks were kept that cannot be reached at
    /// launch says the fix: choose it again.
    #[cfg(target_os = "macos")]
    #[test]
    fn resume_of_an_old_unreachable_path_says_choose_it_again() {
        let mut db = open_memory_store().unwrap();
        let gone = temp_folder().join("gone");
        set_setting(&mut db, LIBRARY_PATH_KEY, gone.to_str().unwrap()).unwrap();
        match resume(&mut db).unwrap() {
            Resumed::Lost { why, .. } => assert!(why.contains("choose the folder again"), "{why}"),
            _ => panic!("not lost"),
        }
    }
}
