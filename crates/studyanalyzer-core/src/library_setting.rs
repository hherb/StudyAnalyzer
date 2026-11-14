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

use bmlib::db::{transaction_with, Db};
use bmlib::library::{Library, MARKER_FILE};
use serde::Serialize;

use crate::config::LIBRARY_PATH_KEY;
use crate::intake::SourceKind;
use crate::library::{
    configured, installation, opened_before, read_scanned, remember_opened, settle, Configured,
    LibraryAt, LibraryFault,
};
use crate::store::{
    app_to_waiting, bring_back, count_by_place, delete_setting, get_setting, mark_in_library,
    note_left_behind, note_library, paper_title, papers_with_place, set_setting, source_by_id,
    sources_with_place, waiting_sources, waiting_to_app, PaperId, PlaceCounts, SourcePlace,
    StoreError,
};

/// A source still waiting for the library, as the setting lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitingSource {
    /// Its paper.
    pub paper_id: String,
    /// Its paper's title.
    pub title: String,
    /// What kind of source it is.
    pub kind: SourceKind,
    /// Why it is not in the library yet, once a move has failed.
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

/// Why a change of the setting was refused.
#[derive(Debug)]
pub enum SettingError {
    /// The database failed.
    Store(StoreError),
    /// The folder chosen cannot be a library; why, naming it.
    Refused(String),
    /// The change needs a usable library and there is none; why.
    Unreachable(String),
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
        }
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
/// by place and the sources still waiting. `label` names this installation
/// in the library's records (as [`configured`] takes it).
pub fn status(db: &mut dyn Db, label: &str) -> Result<LibraryStatus, StoreError> {
    let unreachable = match configured(db, label)? {
        Configured::Unreachable(why) => Some(why),
        Configured::None | Configured::Ready(_) => None,
    };
    let mut waiting = Vec::new();
    for src in waiting_sources(db, None)? {
        waiting.push(WaitingSource {
            paper_id: src.paper_id.as_str().to_string(),
            title: paper_title(db, &src.paper_id)?,
            kind: src.kind,
            note: src.library_note,
        });
    }
    Ok(LibraryStatus {
        path: get_setting(db, LIBRARY_PATH_KEY)?,
        unreachable,
        counts: count_by_place(db)?,
        waiting,
    })
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
/// copies nothing. The old folder is never changed.
pub fn choose(
    db: &mut dyn Db,
    label: &str,
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
    let library = Library::open(root).map_err(|e| {
        log::warn!("the folder {path} is refused as the library: {e}");
        SettingError::Refused(e.to_string())
    })?;
    let new = LibraryAt {
        library,
        who: installation(db, label)?,
    };
    let old = configured(db, label)?;
    let old_path = get_setting(db, LIBRARY_PATH_KEY)?;
    let mut changed = Changed::default();
    if let Some(old_path) = old_path.as_deref().filter(|&p| p != path) {
        let left = left_note(old_path);
        let n = note_left_behind(db, &left)?;
        log::info!("{n} library source(s) note that their file is in {old_path}");
        if bring {
            match &old {
                Configured::None => {}
                Configured::Ready(old) => copy_out(db, old, Some(&new), &left, &mut changed)?,
                Configured::Unreachable(why) => name_each(db, why, &mut changed)?,
            }
        }
    }
    transaction_with(db, |tx| {
        set_setting(tx, LIBRARY_PATH_KEY, path)?;
        remember_opened(tx, path)
    })?;
    log::info!("the paper library is now {path}");
    if bring && matches!(old, Configured::None) {
        let n = app_to_waiting(db)?;
        log::info!("{n} source(s) kept in the app are to move into the library");
    }
    settle_all(db, &new, &mut changed)?;
    Ok(changed)
}

/// Sets no library. With `bring`, this installation's files in the old
/// library are copied back into the app first; one that cannot be read is
/// named. Without `bring`, or failed, a source in the library reads as
/// unavailable until that folder is chosen again, its row noting the
/// folder. Waiting sources stay in the app for good.
pub fn clear(db: &mut dyn Db, label: &str, bring: bool) -> Result<Changed, SettingError> {
    let mut changed = Changed::default();
    let old = configured(db, label)?;
    if let Some(old_path) = get_setting(db, LIBRARY_PATH_KEY)? {
        let left = left_note(&old_path);
        let n = note_left_behind(db, &left)?;
        log::info!("{n} library source(s) note that their file is in {old_path}");
        if bring {
            match &old {
                Configured::None => {}
                Configured::Ready(old) => copy_out(db, old, None, &left, &mut changed)?,
                Configured::Unreachable(why) => name_each(db, why, &mut changed)?,
            }
        }
    }
    let kept = transaction_with(db, |tx| {
        let kept = waiting_to_app(tx)?;
        delete_setting(tx, LIBRARY_PATH_KEY)?;
        Ok::<_, StoreError>(kept)
    })?;
    log::info!("no paper library is set now; {kept} waiting source(s) stay in the app");
    Ok(changed)
}

/// Moves every source kept in the app into the library set (§4.7, run from
/// the setting). Without a usable library it is [`SettingError::Unreachable`].
pub fn move_in(db: &mut dyn Db, label: &str) -> Result<Changed, SettingError> {
    let lib = match configured(db, label)? {
        Configured::Ready(lib) => lib,
        Configured::None => {
            return Err(SettingError::Unreachable(
                "no library folder is set".to_string(),
            ))
        }
        Configured::Unreachable(why) => return Err(SettingError::Unreachable(why)),
    };
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
/// to `changed`.
fn settle_all(db: &mut dyn Db, lib: &LibraryAt, changed: &mut Changed) -> Result<(), StoreError> {
    let settled = settle(db, lib, None)?;
    changed.moved += settled.moved;
    for (paper, why) in settled.left {
        changed.failed.push(problem(db, &paper, why)?);
    }
    Ok(())
}

/// A [`PaperProblem`] for `paper`, with its title.
fn problem(db: &mut dyn Db, paper: &PaperId, why: String) -> Result<PaperProblem, StoreError> {
    Ok(PaperProblem {
        paper_id: paper.as_str().to_string(),
        title: paper_title(db, paper)?,
        why,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identifiers::Identifiers;
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
        find_digest(&scan, &sha256_hex(bytes)).is_some()
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
        let c = choose(&mut db, LABEL, &a, true).unwrap();
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
        let e = choose(&mut db, LABEL, &dir, true).unwrap_err();
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
        let c = choose(&mut db, LABEL, &dir, false).unwrap();
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
        let c = choose(&mut db, LABEL, &dir, true).unwrap();
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
        let c = choose(&mut db, LABEL, &b, true).unwrap();
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
        let c = choose(&mut db, LABEL, &b, true).unwrap();
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
        let c = choose(&mut db, LABEL, &b, true).unwrap();
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
        let e = choose(&mut db, LABEL, &a, false).unwrap_err();
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
        let c = choose(&mut db, LABEL, &a, true).unwrap();
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
        let two = find_digest(&scan, &sha256_hex(b"two")).unwrap();
        std::fs::remove_file(a.join("papers").join(&two.folder).join(&two.name)).unwrap();
        let c = clear(&mut db, LABEL, true).unwrap();
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
        let c = clear(&mut db, LABEL, false).unwrap();
        assert_eq!(c, Changed::default());
        for s in [s1, s2] {
            let src = source_by_id(&mut db, s).unwrap();
            assert_eq!(src.place, SourcePlace::Library);
            assert!(src.library_note.unwrap().contains(a.to_str().unwrap()));
        }
        assert_eq!(place(&mut db, w), SourcePlace::App);
        let s = status(&mut db, LABEL).unwrap();
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
        let c = choose(&mut db, LABEL, &b, false).unwrap();
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

    /// A switch with `bring` copies what it can read; one file gone from the
    /// old library is named and left in the library, without bytes.
    #[test]
    fn switch_names_a_file_the_old_library_lost() {
        let (mut db, a, [s1, s2]) = in_library_a();
        let scan = Library::open(&a).unwrap().scan().unwrap();
        let two = find_digest(&scan, &sha256_hex(b"two")).unwrap();
        std::fs::remove_file(a.join("papers").join(&two.folder).join(&two.name)).unwrap();
        let b = temp_folder();
        let c = choose(&mut db, LABEL, &b, true).unwrap();
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
        choose(&mut db, LABEL, &b, false).unwrap();
        clear(&mut db, LABEL, false).unwrap();
        for dir in [&a, &b] {
            std::fs::remove_dir_all(dir).unwrap();
            std::fs::create_dir_all(dir).unwrap();
            let e = choose(&mut db, LABEL, dir, false).unwrap_err();
            assert!(matches!(&e, SettingError::Refused(_)), "{e}");
            std::fs::remove_dir_all(dir).ok();
        }
    }

    /// The status names an unreachable library and lists each waiting source.
    #[test]
    fn status_lists_waiting_sources() {
        let mut db = open_memory_store().unwrap();
        let empty = status(&mut db, LABEL).unwrap();
        assert_eq!((empty.path, empty.unreachable), (None, None));
        let (_, w) = paper(&mut db, "1", b"one", SourcePlace::Waiting);
        crate::store::note_waiting(&mut db, w, "unplugged").unwrap();
        let gone = temp_folder().join("missing");
        set_setting(&mut db, LIBRARY_PATH_KEY, gone.to_str().unwrap()).unwrap();
        let s = status(&mut db, LABEL).unwrap();
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
            move_in(&mut db, LABEL),
            Err(SettingError::Unreachable(_))
        ));
        let gone = temp_folder().join("missing");
        set_setting(&mut db, LIBRARY_PATH_KEY, gone.to_str().unwrap()).unwrap();
        assert!(matches!(
            move_in(&mut db, LABEL),
            Err(SettingError::Unreachable(why)) if why.contains("missing")
        ));
        assert_eq!(place(&mut db, s), SourcePlace::App);
        let dir = temp_folder();
        choose(&mut db, LABEL, &dir, false).unwrap();
        let c = move_in(&mut db, LABEL).unwrap();
        assert_eq!((c.moved, c.failed.len()), (1, 0));
        assert_eq!(place(&mut db, s), SourcePlace::Library);
        std::fs::remove_dir_all(dir).ok();
    }
}
