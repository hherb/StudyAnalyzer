//! The manual clear (design `2026-11-13-shared-paper-library-design.md`,
//! §4.8): the library's papers as the setting lists them ([`list`]), each
//! file with its size and the installations whose records list it, and the
//! removal of a paper's folder or of one file ([`remove`]). Nothing here runs
//! on its own: the user asks for each removal, after a confirmation that
//! names the other apps still using it. An app that later misses a removed
//! file reports it and fetches nothing again.

use bmlib::db::Db;
use bmlib::library::{FileRef, LibraryError, PaperFolder, Problem, Record, SourceKind, PAPERS_DIR};
use serde::Serialize;

use crate::library::LibraryAt;
use crate::library_setting::{ready, SettingError};

/// One source file in a paper's folder, as the clear lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedFile {
    /// Its name in the folder (`pdf-3f9a1c2e7b40.pdf`): its kind and a prefix
    /// of its digest.
    pub name: String,
    /// Its size in bytes, as the folder holds it; `None` when it cannot be read.
    pub size: Option<u64>,
    /// The other installations whose records list it, by their labels.
    pub used_by: Vec<String>,
    /// Whether this installation's record lists it.
    pub used_here: bool,
}

/// One paper folder of the library, as the clear lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedPaper {
    /// The folder's name under `papers/` (readable: year, author, title, an
    /// identifier); what [`remove`] takes back.
    pub folder: String,
    /// The title the first record that has one gives, if any.
    pub title: Option<String>,
    /// Its source files, ordered by name. One present only as an online
    /// placeholder is not listed, so it cannot be cleared here.
    pub files: Vec<ListedFile>,
    /// The other installations with a record in it, by their labels, each
    /// once: removing the folder deletes their records, files on disk or not.
    pub used_by: Vec<String>,
    /// Whether this installation has a record in it.
    pub used_here: bool,
    /// Whether the scan could not read some of its records (or the folder
    /// itself): who else uses it is then not known, and the list says so.
    pub users_unknown: bool,
}

/// What the clear shows: the papers, and what the scan found odd.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listing {
    /// Every paper folder, ordered by name.
    pub papers: Vec<ListedPaper>,
    /// Each thing the scan found odd and did not use (a record it cannot
    /// read, a file no record lists…), in the `Debug` form of bmlib's
    /// `Problem` (shortcut: it has no wording yet, shown as is while
    /// developing; give each kind a sentence before a release).
    pub problems: Vec<String>,
}

/// The library's papers, from one scan of the library set. Without a usable
/// library it is [`SettingError::Unreachable`]; a scan that fails is
/// [`SettingError::Library`]. Each problem the scan found is logged and listed.
/// `launch_note` ends an unreachable library's reason (see `ready`).
pub fn list(
    db: &mut dyn Db,
    label: &str,
    launch_note: Option<&str>,
) -> Result<Listing, SettingError> {
    let lib = ready(db, label, launch_note)?;
    let scan = lib.library.scan().map_err(|e| {
        log::warn!("the library cannot be listed: {e}");
        SettingError::Library(format!("The paper library cannot be listed: {e}"))
    })?;
    let problems: Vec<String> = scan.problems.iter().map(|p| format!("{p:?}")).collect();
    for p in &problems {
        log::warn!("the library scan found: {p}");
    }
    let unread: Vec<&str> = scan.problems.iter().filter_map(unread_records).collect();
    let papers = scan
        .folders
        .iter()
        .map(|f| listed(&lib, f, unread.contains(&f.folder.as_str())))
        .collect();
    Ok(Listing { papers, problems })
}

/// The folder whose records `problem` says the scan could not all read (so
/// its users are not all known), if it says that.
fn unread_records(problem: &Problem) -> Option<&str> {
    match problem {
        Problem::UnreadableRecord { folder, .. }
        | Problem::UnreadableFolder { folder, .. }
        | Problem::NewerRecord { folder, .. }
        | Problem::RecordNotDownloaded { folder, .. }
        | Problem::MisnamedRecord { folder, .. } => Some(folder),
        _ => None,
    }
}

/// Whether record `r` is this installation's.
fn is_here(lib: &LibraryAt, r: &Record) -> bool {
    r.installation.app == lib.who.app && r.installation.id == lib.who.id
}

/// The clear's view of one folder: its title, its users (`users_unknown`
/// when some of its records could not be read) and each file on disk with
/// its size and users.
fn listed(lib: &LibraryAt, folder: &PaperFolder, users_unknown: bool) -> ListedPaper {
    let title = folder.records.iter().find_map(|r| r.paper.title.clone());
    let mut used_by: Vec<String> = Vec::new();
    for r in folder.records.iter().filter(|r| !is_here(lib, r)) {
        if !used_by.contains(&r.installation.label) {
            used_by.push(r.installation.label.clone());
        }
    }
    let used_here = folder.records.iter().any(|r| is_here(lib, r));
    let files = folder
        .files_on_disk
        .iter()
        .map(|name| {
            let mut used_by = Vec::new();
            let mut used_here = false;
            for r in &folder.records {
                if !r.files.iter().any(|f| f.name == *name) {
                    continue;
                }
                if is_here(lib, r) {
                    used_here = true;
                } else {
                    used_by.push(r.installation.label.clone());
                }
            }
            let path = lib
                .library
                .root()
                .join(PAPERS_DIR)
                .join(&folder.folder)
                .join(name);
            let size = match std::fs::metadata(&path) {
                Ok(m) => Some(m.len()),
                Err(e) => {
                    log::warn!("the size of {} cannot be read: {e}", path.display());
                    None
                }
            };
            ListedFile {
                name: name.clone(),
                size,
                used_by,
                used_here,
            }
        })
        .collect();
    ListedPaper {
        folder: folder.folder.clone(),
        title,
        files,
        used_by,
        used_here,
        users_unknown,
    }
}

/// Deletes the paper folder `folder` from the library set, records and all,
/// or with `name`, only that file of it (other records are left listing it,
/// so each app reports it missing). A fresh scan must still list the folder
/// (and the file): a list shown earlier may be stale, so anything it no
/// longer holds is [`SettingError::Library`], nothing removed (shortcut: who
/// uses it is not checked again, so the confirmation names the users of the
/// list as read). A folder whose deletion fails partway may be partly gone,
/// and the error says so. A folder or file whose name bmlib refuses (#789),
/// or a file name that is not UTF-8, is not touched, and the error asks for a
/// rename. This installation's sources that used it then read as no longer in
/// the library; opening again the file a paper's text was read from puts it
/// back. `launch_note` is as [`list`] takes it.
pub fn remove(
    db: &mut dyn Db,
    label: &str,
    launch_note: Option<&str>,
    folder: &str,
    name: Option<&str>,
) -> Result<(), SettingError> {
    let lib = ready(db, label, launch_note)?;
    let what = match name {
        Some(n) => format!("{folder}/{n}"),
        None => folder.to_string(),
    };
    let failed = |e: &dyn std::fmt::Display| {
        log::warn!("\u{201c}{what}\u{201d} was not removed from the library: {e}");
        SettingError::Library(format!(
            "\u{201c}{what}\u{201d} could not be removed from the paper library: {e}"
        ))
    };
    let scan = lib.library.scan().map_err(|e| failed(&e))?;
    let gone =
        || failed(&"the library no longer holds it as listed; the list now shows what it holds");
    let found = scan
        .folders
        .iter()
        .find(|f| f.folder == folder)
        .ok_or_else(gone)?;
    let rename = |why: &dyn std::fmt::Display| {
        log::warn!("\u{201c}{what}\u{201d} was not removed, its name is refused: {why}");
        SettingError::Library(format!(
            "\u{201c}{what}\u{201d} was not removed: {why}. Rename it in the file manager, then \
             read the list again"
        ))
    };
    // bmlib raises these before it deletes anything: a name it refuses (#789),
    // or what went between the scan and here.
    let untouched = |e: &LibraryError| match e {
        LibraryError::BadName { .. } => Some(rename(e)),
        LibraryError::Missing { .. } => Some(gone()),
        _ => None,
    };
    let Some(name) = name else {
        lib.library.remove_folder(folder).map_err(|e| {
            untouched(&e).unwrap_or_else(|| {
                log::error!("the paper folder \u{201c}{folder}\u{201d} may be partly removed: {e}");
                SettingError::Library(format!(
                    "\u{201c}{folder}\u{201d} was not removed from the paper library, or only in \
                     part (the list now shows what is left): {e}"
                ))
            })
        })?;
        log::info!("the paper folder \u{201c}{folder}\u{201d} was removed from the library");
        return Ok(());
    };
    if !found.files_on_disk.iter().any(|n| n == name) {
        return Err(gone());
    }
    // shortcut: bmlib lists a name that is not UTF-8 lossily, as one with a
    // U+FFFD, and a join would reach a sibling of that very spelling; a real
    // U+FFFD is refused too until bmlib tells them apart (bmlib#796).
    if name.contains(char::REPLACEMENT_CHARACTER) {
        return Err(rename(&"its name is not readable text"));
    }
    lib.library
        .remove_file(&file_ref(found, name))
        .map_err(|e| untouched(&e).unwrap_or_else(|| failed(&e)))?;
    log::info!("{folder}/{name} was removed from the library");
    Ok(())
}

/// The [`FileRef`] of file `name` in `folder`, with the digest and kind a
/// record gives it. shortcut: a file no record lists gets no digest and the
/// kind `text`, since `Library::remove_file` locates by folder and name only;
/// hash it here if bmlib starts checking either.
fn file_ref(folder: &PaperFolder, name: &str) -> FileRef {
    let entry = folder
        .records
        .iter()
        .flat_map(|r| r.files.iter())
        .find(|f| f.name == name);
    FileRef {
        folder: folder.folder.clone(),
        name: name.to_string(),
        sha256: entry.map(|e| e.sha256.clone()).unwrap_or_default(),
        kind: entry.map_or(SourceKind::Text, |e| e.kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identifiers::Identifiers;
    use crate::intake::SourceKind as Kind;
    use crate::library::{configured, paper_info, read_source, Configured};
    use crate::library_setting::choose;
    use crate::store::{
        create_paper, open_memory_store, save_source, source_by_id, NewSource, SourcePlace,
    };
    use bmlib::library::Installation;
    use rusqlite::Connection;
    use std::path::PathBuf;

    const LABEL: &str = "StudyAnalyzer on a test";
    const OTHER: &str = "MedicalFactChecker on a test";

    /// A database whose new library (a fresh temp folder) holds one paper's
    /// text file, used here and, recorded by another installation, there.
    fn shared() -> (Connection, PathBuf, i64) {
        let mut db = open_memory_store().unwrap();
        let root = std::env::temp_dir().join(format!("sa-clear-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let ids = Identifiers {
            pmid: Some("7".into()),
            ..Default::default()
        };
        let p = create_paper(&mut db, &ids, Some("Trial 7")).unwrap();
        let src = NewSource {
            kind: Kind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: b"seven".to_vec(),
            decoded: None,
        };
        let id = save_source(&mut db, &p, src, SourcePlace::App).unwrap();
        choose(&mut db, LABEL, None, &root, true).unwrap();
        let Configured::Ready(lib) = configured(&mut db, LABEL).unwrap() else {
            panic!("the library is not ready");
        };
        let other = Installation::new("medicalfactchecker", "0-other", OTHER).unwrap();
        let info = paper_info(&mut db, &p).unwrap();
        lib.library
            .save(&other, &info, b"seven", SourceKind::Text, "pasted")
            .unwrap();
        lib.library
            .save(&other, &info, b"theirs", SourceKind::Pdf, "Europe PMC")
            .unwrap();
        (db, root, id)
    }

    /// The listing names each file's users, this installation apart, and its size.
    #[test]
    fn list_names_users_and_sizes() {
        let (mut db, root, _) = shared();
        let listing = list(&mut db, LABEL, None).unwrap();
        assert_eq!(listing.papers.len(), 1, "{listing:?}");
        let paper = &listing.papers[0];
        assert_eq!(paper.title.as_deref(), Some("Trial 7"));
        let text = paper
            .files
            .iter()
            .find(|f| f.name.starts_with("text-"))
            .unwrap();
        assert_eq!(
            (text.size, text.used_here, text.used_by.clone()),
            (Some(5), true, vec![OTHER.to_string()])
        );
        let pdf = paper
            .files
            .iter()
            .find(|f| f.name.starts_with("pdf-"))
            .unwrap();
        assert_eq!(
            (pdf.size, pdf.used_here, pdf.used_by.clone()),
            (Some(6), false, vec![OTHER.to_string()])
        );
        assert!(listing.problems.is_empty(), "{:?}", listing.problems);
        assert_eq!(
            (paper.used_here, paper.used_by.clone(), paper.users_unknown),
            (true, vec![OTHER.to_string()], false)
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// A paper's users come from all its records, so an installation whose
    /// files are gone from the folder still counts: removing the folder
    /// deletes its record. A record the scan cannot read makes its users
    /// unknown.
    #[test]
    fn paper_users_come_from_every_record() {
        let (mut db, root, id) = shared();
        const THIRD: &str = "BioMedNews on a test";
        let third = Installation::new("biomednews", "0-third", THIRD).unwrap();
        let paper_id = source_by_id(&mut db, id).unwrap().paper_id;
        let info = paper_info(&mut db, &paper_id).unwrap();
        let Configured::Ready(lib) = configured(&mut db, LABEL).unwrap() else {
            panic!("the library is not ready");
        };
        let saved = lib
            .library
            .save(&third, &info, b"third's", SourceKind::Pdf, "CORE")
            .unwrap();
        let folder = root.join(PAPERS_DIR).join(&saved.folder);
        std::fs::remove_file(folder.join(&saved.name)).unwrap();
        let paper = list(&mut db, LABEL, None).unwrap().papers.remove(0);
        assert!(
            paper.files.iter().all(|f| f.name != saved.name),
            "{paper:?}"
        );
        assert_eq!(
            (paper.used_by.clone(), paper.users_unknown),
            (vec![THIRD.to_string(), OTHER.to_string()], false)
        );
        std::fs::write(folder.join("records").join("zz-broken.json"), b"{").unwrap();
        let listing = list(&mut db, LABEL, None).unwrap();
        assert!(listing.papers[0].users_unknown, "{listing:?}");
        std::fs::remove_dir_all(root).ok();
    }

    /// A file removed is gone from the list, the rest of its folder stays,
    /// and this installation's source then reads as no longer there.
    #[test]
    fn remove_one_file() {
        let (mut db, root, id) = shared();
        let paper = list(&mut db, LABEL, None).unwrap().papers.remove(0);
        let text = paper.files.iter().find(|f| f.used_here).unwrap();
        remove(&mut db, LABEL, None, &paper.folder, Some(&text.name)).unwrap();
        let after = list(&mut db, LABEL, None).unwrap();
        let names: Vec<&str> = after.papers[0]
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert!(names.iter().all(|n| n.starts_with("pdf-")), "{names:?}");
        let src = source_by_id(&mut db, id).unwrap();
        let lib = configured(&mut db, LABEL).unwrap();
        let e = read_source(&mut db, &lib, &src).unwrap_err();
        assert!(
            e.to_string().contains("no longer in the paper library"),
            "{e}"
        );
        // The other installation's record still lists it, so that app reports it missing.
        let Configured::Ready(at) = &lib else {
            panic!("the library is not ready");
        };
        let scan = at.library.scan().unwrap();
        let theirs = scan.folders[0]
            .records
            .iter()
            .find(|r| r.installation.label == OTHER)
            .unwrap();
        assert!(
            theirs.files.iter().any(|f| f.name == text.name),
            "{theirs:?}"
        );
        assert!(after.problems.is_empty(), "{:?}", after.problems);
        std::fs::remove_dir_all(root).ok();
    }

    /// A paper removed takes its folder, records and all, and this
    /// installation's source then reads as no longer there.
    #[test]
    fn remove_a_paper() {
        let (mut db, root, id) = shared();
        let folder = list(&mut db, LABEL, None).unwrap().papers[0].folder.clone();
        remove(&mut db, LABEL, None, &folder, None).unwrap();
        assert!(list(&mut db, LABEL, None).unwrap().papers.is_empty());
        assert!(!root.join(PAPERS_DIR).join(&folder).exists());
        let src = source_by_id(&mut db, id).unwrap();
        let lib = configured(&mut db, LABEL).unwrap();
        let e = read_source(&mut db, &lib, &src).unwrap_err();
        assert!(
            e.to_string().contains("no longer in the paper library"),
            "{e}"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// What a fresh scan no longer lists (a stale list, a name the webview
    /// made up) is refused, and nothing is removed.
    #[test]
    fn remove_refuses_what_is_not_listed() {
        let (mut db, root, _) = shared();
        let paper = list(&mut db, LABEL, None).unwrap().papers.remove(0);
        for (folder, name) in [
            ("no such paper", None),
            (paper.folder.as_str(), Some("pdf-000000000000.pdf")),
            (paper.folder.as_str(), Some("../bmlib-library.json")),
            ("..", None),
        ] {
            let e = remove(&mut db, LABEL, None, folder, name).unwrap_err();
            assert!(
                matches!(&e, SettingError::Library(why) if why.contains("no longer holds it")),
                "{folder} {name:?}: {e}"
            );
        }
        assert_eq!(list(&mut db, LABEL, None).unwrap().papers[0], paper);
        std::fs::remove_dir_all(root).ok();
    }

    /// Whether `e` is the refusal that asks for a rename, nothing removed.
    fn asks_for_a_rename(e: &SettingError) -> bool {
        matches!(e, SettingError::Library(why) if why.contains("Rename it") && !why.contains("in part"))
    }

    /// A paper folder whose name bmlib cannot use (`papers/con`, one named
    /// before bmlib #788's device rule, here one with a leading space, which
    /// every file system takes) is still listed, with its problem. Removing it
    /// or one of its files is refused, nothing removed, asking for a rename
    /// (#789).
    #[test]
    fn a_folder_bmlib_cannot_name_is_listed_and_not_removed() {
        let (mut db, root, _) = shared();
        let before = list(&mut db, LABEL, None).unwrap().papers.remove(0);
        let papers = root.join(PAPERS_DIR);
        let bad = format!(" {}", before.folder);
        std::fs::rename(papers.join(&before.folder), papers.join(&bad)).unwrap();
        let listing = list(&mut db, LABEL, None).unwrap();
        assert_eq!(listing.papers.len(), 1, "{listing:?}");
        assert_eq!(listing.papers[0].folder, bad);
        assert!(
            listing.problems.iter().any(|p| p.contains("BadFolderName")),
            "{:?}",
            listing.problems
        );
        let e = remove(&mut db, LABEL, None, &bad, None).unwrap_err();
        assert!(asks_for_a_rename(&e), "{e}");
        let file = &before.files[0].name;
        let e = remove(&mut db, LABEL, None, &bad, Some(file)).unwrap_err();
        assert!(asks_for_a_rename(&e), "{e}");
        let after = list(&mut db, LABEL, None).unwrap().papers.remove(0);
        assert_eq!(after.files, before.files);
        std::fs::remove_dir_all(root).ok();
    }

    /// A file name holding U+FFFD, as bmlib lists one that is not UTF-8, is
    /// refused, never joined to a path (bmlib#796), and the file stays.
    #[test]
    fn a_file_name_that_may_be_lossy_is_not_removed() {
        let (mut db, root, _) = shared();
        let folder = list(&mut db, LABEL, None).unwrap().papers[0].folder.clone();
        let odd = "caf\u{fffd}.pdf";
        let path = root.join(PAPERS_DIR).join(&folder).join(odd);
        std::fs::write(&path, b"odd").unwrap();
        let e = remove(&mut db, LABEL, None, &folder, Some(odd)).unwrap_err();
        assert!(asks_for_a_rename(&e), "{e}");
        assert!(path.exists());
        std::fs::remove_dir_all(root).ok();
    }

    /// Without a library set, listing and removing say so.
    #[test]
    fn no_library_is_unreachable() {
        let mut db = open_memory_store().unwrap();
        let e = list(&mut db, LABEL, None).unwrap_err();
        assert!(matches!(&e, SettingError::Unreachable(_)), "{e}");
        let e = remove(&mut db, LABEL, None, "x", None).unwrap_err();
        assert!(matches!(&e, SettingError::Unreachable(_)), "{e}");
    }
}
