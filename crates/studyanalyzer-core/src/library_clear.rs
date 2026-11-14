//! The manual clear (design `2026-11-13-shared-paper-library-design.md`,
//! §4.8): the library's papers as the setting lists them ([`list`]), each
//! file with its size and the installations whose records list it, and the
//! removal of a paper's folder or of one file ([`remove`]). Nothing here runs
//! on its own: the user asks for each removal, after a confirmation that
//! names the other apps still using it. An app that later misses a removed
//! file reports it and fetches nothing again.

use bmlib::db::Db;
use bmlib::library::{FileRef, PaperFolder, SourceKind, PAPERS_DIR};
use serde::Serialize;

use crate::library::LibraryAt;
use crate::library_setting::{ready, SettingError};

/// One source file in a paper's folder, as the clear lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedFile {
    /// Its name in the folder (`pdf-3f9a1c2e7b40.pdf`): its kind and digest.
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
    /// Its source files, by name.
    pub files: Vec<ListedFile>,
}

/// What the clear shows: the papers, and what the scan found odd.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listing {
    /// Every paper folder, ordered by name.
    pub papers: Vec<ListedPaper>,
    /// Each thing the scan found odd and did not use (a record it cannot
    /// read, a file no record lists…), as bmlib describes it.
    pub problems: Vec<String>,
}

/// The library's papers, from one scan of the library set. Without a usable
/// library it is [`SettingError::Unreachable`]; a scan that fails is
/// [`SettingError::Library`]. Each problem the scan found is logged and listed.
pub fn list(db: &mut dyn Db, label: &str) -> Result<Listing, SettingError> {
    let lib = ready(db, label)?;
    let scan = lib.library.scan().map_err(|e| {
        log::warn!("the library cannot be listed: {e}");
        SettingError::Library(format!("The paper library cannot be listed: {e}"))
    })?;
    let problems: Vec<String> = scan.problems.iter().map(|p| format!("{p:?}")).collect();
    for p in &problems {
        log::warn!("the library scan found: {p}");
    }
    let papers = scan.folders.iter().map(|f| listed(&lib, f)).collect();
    Ok(Listing { papers, problems })
}

/// The clear's view of one folder: its title and each file on disk with its
/// size and users.
fn listed(lib: &LibraryAt, folder: &PaperFolder) -> ListedPaper {
    let title = folder.records.iter().find_map(|r| r.paper.title.clone());
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
                if r.installation.app == lib.who.app && r.installation.id == lib.who.id {
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
    }
}

/// Deletes the paper folder `folder` from the library set, records and all,
/// or with `name`, only that file of it (other records are left listing it,
/// so each app reports it missing). A fresh scan must still list the folder
/// (and the file): a list shown earlier may be stale, so anything it no
/// longer holds is [`SettingError::Library`], nothing removed. This
/// installation's sources that used it then read as no longer in the
/// library; a later open of the same file puts it back.
pub fn remove(
    db: &mut dyn Db,
    label: &str,
    folder: &str,
    name: Option<&str>,
) -> Result<(), SettingError> {
    let lib = ready(db, label)?;
    let failed = |e: &dyn std::fmt::Display| {
        log::warn!("\u{201c}{folder}\u{201d} {name:?} was not removed from the library: {e}");
        SettingError::Library(format!(
            "\u{201c}{folder}\u{201d} could not be removed from the paper library: {e}"
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
    let Some(name) = name else {
        lib.library.remove_folder(folder).map_err(|e| failed(&e))?;
        log::info!("the paper folder \u{201c}{folder}\u{201d} was removed from the library");
        return Ok(());
    };
    if !found.files_on_disk.iter().any(|n| n == name) {
        return Err(gone());
    }
    lib.library
        .remove_file(&file_ref(found, name))
        .map_err(|e| failed(&e))?;
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
        choose(&mut db, LABEL, &root, true).unwrap();
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
        let listing = list(&mut db, LABEL).unwrap();
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
        std::fs::remove_dir_all(root).ok();
    }

    /// A file removed is gone from the list, the rest of its folder stays,
    /// and this installation's source then reads as no longer there.
    #[test]
    fn remove_one_file() {
        let (mut db, root, id) = shared();
        let paper = list(&mut db, LABEL).unwrap().papers.remove(0);
        let text = paper.files.iter().find(|f| f.used_here).unwrap();
        remove(&mut db, LABEL, &paper.folder, Some(&text.name)).unwrap();
        let after = list(&mut db, LABEL).unwrap();
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
        std::fs::remove_dir_all(root).ok();
    }

    /// A paper removed takes its folder, records and all.
    #[test]
    fn remove_a_paper() {
        let (mut db, root, _) = shared();
        let folder = list(&mut db, LABEL).unwrap().papers[0].folder.clone();
        remove(&mut db, LABEL, &folder, None).unwrap();
        assert!(list(&mut db, LABEL).unwrap().papers.is_empty());
        assert!(!root.join(PAPERS_DIR).join(&folder).exists());
        std::fs::remove_dir_all(root).ok();
    }

    /// What a fresh scan no longer lists (a stale list, a name the webview
    /// made up) is refused, and nothing is removed.
    #[test]
    fn remove_refuses_what_is_not_listed() {
        let (mut db, root, _) = shared();
        let paper = list(&mut db, LABEL).unwrap().papers.remove(0);
        for (folder, name) in [
            ("no such paper", None),
            (paper.folder.as_str(), Some("pdf-000000000000.pdf")),
            (paper.folder.as_str(), Some("../bmlib-library.json")),
            ("..", None),
        ] {
            let e = remove(&mut db, LABEL, folder, name).unwrap_err();
            assert!(
                matches!(&e, SettingError::Library(why) if why.contains("no longer holds it")),
                "{folder} {name:?}: {e}"
            );
        }
        assert_eq!(list(&mut db, LABEL).unwrap().papers[0], paper);
        std::fs::remove_dir_all(root).ok();
    }

    /// Without a library set, listing and removing say so.
    #[test]
    fn no_library_is_unreachable() {
        let mut db = open_memory_store().unwrap();
        let e = list(&mut db, LABEL).unwrap_err();
        assert!(matches!(&e, SettingError::Unreachable(_)), "{e}");
        let e = remove(&mut db, LABEL, "x", None).unwrap_err();
        assert!(matches!(&e, SettingError::Unreachable(_)), "{e}");
    }
}
