//! The shared paper library (design `2026-11-13-shared-paper-library-design.md`):
//! a folder the user chooses, shared by the bmlib apps, where StudyAnalyzer
//! keeps its papers' sources once one is set. The database then holds a
//! source's SHA-256 and kind, not its bytes (`store::SourcePlace`). A new
//! source is stored as *waiting* and moved in by [`settle`]; one the library
//! cannot take stays waiting, and the paper's banner says so
//! ([`waiting_notice`]). Every read from the library checks the digest.

use std::fmt;
use std::path::Path;

use bmlib::db::{transaction_with, Db};
use bmlib::fulltext::library_ids;
use bmlib::library::{
    find, find_digest, FileRef, Installation, Library, PaperInfo, Scan, SourceKind as LibraryKind,
};

use crate::config::{
    LIBRARY_APP, LIBRARY_INSTALLATION_KEY, LIBRARY_LABEL_FALLBACK, LIBRARY_PATH_KEY,
    PREPRINT_PREFIX, UNTITLED_PAPER,
};
use crate::intake::{jats_identifiers, SourceKind};
use crate::store::{
    contradiction, get_setting, latest_canonical, mark_in_library, note_waiting, paper_identifiers,
    paper_title, pdf_beside_source, set_setting, source_by_id, source_bytes, waiting_sources,
    PaperId, SourcePlace, StoreError, StoredSource,
};

/// A library that opened, and this installation in it.
#[derive(Debug, Clone)]
pub struct LibraryAt {
    /// The library.
    pub library: Library,
    /// This installation, as its records name it.
    pub who: Installation,
}

/// The library as the settings leave it.
#[derive(Debug, Clone)]
pub enum Configured {
    /// No library chosen: sources stay in the app's own storage.
    None,
    /// A library chosen and usable.
    Ready(LibraryAt),
    /// A library chosen that cannot be used now (an unplugged drive, a
    /// folder that is not a library); why, as a phrase for the user.
    Unreachable(String),
}

impl Configured {
    /// Where a new source is stored: waiting for the library whenever one is
    /// chosen, reachable or not, else in the app.
    pub fn new_place(&self) -> SourcePlace {
        match self {
            Configured::None => SourcePlace::App,
            Configured::Ready(_) | Configured::Unreachable(_) => SourcePlace::Waiting,
        }
    }
}

/// Why a source could not be read from, or moved into, the library.
#[derive(Debug)]
pub enum LibraryFault {
    /// The database failed.
    Store(StoreError),
    /// The library cannot give or take the file now; why, as a phrase for the
    /// user that names the file.
    Unavailable(String),
}

impl fmt::Display for LibraryFault {
    /// A message the user can read.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryFault::Store(e) => write!(f, "{e}"),
            LibraryFault::Unavailable(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for LibraryFault {}

impl From<StoreError> for LibraryFault {
    /// Wraps a store error.
    fn from(e: StoreError) -> Self {
        LibraryFault::Store(e)
    }
}

/// The library the settings name ([`LIBRARY_PATH_KEY`]), opened, with this
/// installation (its id made on first use, [`LIBRARY_INSTALLATION_KEY`]) and
/// `label` (the app and the machine, [`LIBRARY_LABEL_FALLBACK`] when bmlib
/// refuses it). An empty folder becomes a library; one that cannot be used
/// is [`Configured::Unreachable`], logged.
pub fn configured(db: &mut dyn Db, label: &str) -> Result<Configured, StoreError> {
    let Some(path) = get_setting(db, LIBRARY_PATH_KEY)? else {
        return Ok(Configured::None);
    };
    let who = installation(db, label)?;
    Ok(match Library::open(Path::new(&path)) {
        Ok(library) => Configured::Ready(LibraryAt { library, who }),
        Err(e) => {
            log::warn!("the library at {path} cannot be used: {e}");
            Configured::Unreachable(format!("the library folder {path} cannot be used: {e}"))
        }
    })
}

/// This installation: its id from the settings, made (a lower-case UUID) and
/// kept on first use, inside a transaction so two commands make one.
fn installation(db: &mut dyn Db, label: &str) -> Result<Installation, StoreError> {
    let id = transaction_with(db, |tx| {
        if let Some(id) = get_setting(tx, LIBRARY_INSTALLATION_KEY)? {
            return Ok::<_, StoreError>(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        set_setting(tx, LIBRARY_INSTALLATION_KEY, &id)?;
        log::info!("this installation is {id} in the library's records");
        Ok(id)
    })?;
    Installation::new(LIBRARY_APP, &id, label)
        .or_else(|e| {
            log::warn!(
                "the library label {label:?} is refused ({e}); using {LIBRARY_LABEL_FALLBACK:?}"
            );
            Installation::new(LIBRARY_APP, &id, LIBRARY_LABEL_FALLBACK)
        })
        .map_err(|e| {
            StoreError::Corrupt(format!(
                "the stored library installation id {id:?} is not usable: {e}"
            ))
        })
}

/// The library's name for a source kind: `plain_text` is its `text`.
pub fn library_kind(kind: SourceKind) -> LibraryKind {
    match kind {
        SourceKind::Pdf => LibraryKind::Pdf,
        SourceKind::Jats => LibraryKind::Jats,
        SourceKind::Html => LibraryKind::Html,
        SourceKind::Markdown => LibraryKind::Markdown,
        SourceKind::PlainText => LibraryKind::Text,
    }
}

/// The paper as the library's records describe it: its identifiers in the
/// library's canonical form (a preprint accession left out, one bmlib refuses
/// dropped with a warning) and its title. The store knows no year or first
/// author, so the folder is named by the title and an identifier.
pub fn paper_info(db: &mut dyn Db, paper: &PaperId) -> Result<PaperInfo, StoreError> {
    let ids = paper_identifiers(db, paper)?;
    let pmid = ids
        .pmid
        .as_deref()
        .filter(|p| !p.starts_with(PREPRINT_PREFIX))
        .unwrap_or("");
    let (lib_ids, dropped) = library_ids(pmid, ids.doi.as_deref(), ids.pmc_id.as_deref());
    for field in dropped {
        log::warn!(
            "paper {}: the library refuses its {field}, so it is left out of the record",
            paper.0
        );
    }
    let title = Some(paper_title(db, paper)?).filter(|t| t != UNTITLED_PAPER);
    Ok(PaperInfo {
        ids: lib_ids,
        title,
        year: None,
        first_author: None,
    })
}

/// The bytes of `src`, wherever they are: the app's storage, else the
/// library, found by digest and checked against it. A file the library
/// cannot give (none chosen or reachable, removed, damaged, online-only) is
/// [`LibraryFault::Unavailable`], naming it.
pub fn read_source(
    db: &mut dyn Db,
    configured: &Configured,
    src: &StoredSource,
) -> Result<Vec<u8>, LibraryFault> {
    if let Some(bytes) = source_bytes(db, src.id)? {
        return Ok(bytes);
    }
    if src.place != SourcePlace::Library {
        return Err(StoreError::Corrupt(format!(
            "source {} is kept in the app, but its bytes are gone",
            src.id
        ))
        .into());
    }
    let what = describe(src);
    let lib = match configured {
        Configured::Ready(lib) => lib,
        Configured::None => {
            return Err(LibraryFault::Unavailable(format!(
                "{what} is kept in the paper library, and no library folder is set"
            )))
        }
        Configured::Unreachable(why) => {
            return Err(LibraryFault::Unavailable(format!(
                "{what} is kept in the paper library, and {why}"
            )))
        }
    };
    let unavailable = |e: &dyn fmt::Display| LibraryFault::Unavailable(format!("{what}: {e}"));
    // shortcut: a scan per read, no index (bmlib measures 0.12 s at 1,000 papers); keep one if reads get slow.
    let scan = lib.library.scan().map_err(|e| unavailable(&e))?;
    let Some(file) = find_digest(&scan, &src.sha256) else {
        return Err(LibraryFault::Unavailable(format!(
            "{what} is no longer in the paper library"
        )));
    };
    if file.kind != library_kind(src.kind) {
        return Err(unavailable(&format!(
            "the library holds those bytes as {}",
            file.kind.as_str()
        )));
    }
    lib.library.read(&file).map_err(|e| unavailable(&e))
}

/// The source as a phrase for the user: its kind and its digest's start.
fn describe(src: &StoredSource) -> String {
    let short: String = src.sha256.chars().take(12).collect();
    format!(
        "This paper's {} file ({short}…)",
        library_kind(src.kind).as_str()
    )
}

/// What [`settle`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settled {
    /// How many waiting sources went into the library.
    pub moved: usize,
    /// The papers whose sources still wait, each with why.
    pub left: Vec<(PaperId, String)>,
}

/// Moves the waiting sources (of `paper`, or of every paper) into the
/// library: each is saved there (bytes kept once by digest) and recorded as
/// this installation's, then marked as in the library and its bytes dropped
/// here. A source the library cannot take stays waiting, with why in its
/// note, logged; so does one whose paper's library folder holds another
/// paper's JATS ([`foreign_jats`]).
pub fn settle(
    db: &mut dyn Db,
    lib: &LibraryAt,
    paper: Option<&PaperId>,
) -> Result<Settled, StoreError> {
    let mut settled = Settled::default();
    for src in waiting_sources(db, paper)? {
        match move_in(db, lib, &src) {
            Ok(()) => settled.moved += 1,
            Err(LibraryFault::Store(e)) => return Err(e),
            Err(LibraryFault::Unavailable(why)) => {
                log::warn!(
                    "source {} of paper {} stays waiting for the library: {why}",
                    src.id,
                    src.paper_id.0
                );
                note_waiting(db, src.id, &why)?;
                settled.left.push((src.paper_id.clone(), why));
            }
        }
    }
    if settled.moved > 0 {
        log::info!("{} source(s) moved into the library", settled.moved);
    }
    Ok(settled)
}

/// [`settle`] for `paper` when the library is usable; when it is chosen but
/// cannot be reached, each of the paper's waiting sources notes why.
pub fn settle_paper(
    db: &mut dyn Db,
    configured: &Configured,
    paper: &PaperId,
) -> Result<(), StoreError> {
    match configured {
        Configured::None => Ok(()),
        Configured::Ready(lib) => settle(db, lib, Some(paper)).map(|_| ()),
        Configured::Unreachable(why) => {
            for src in waiting_sources(db, Some(paper))? {
                note_waiting(db, src.id, why)?;
            }
            Ok(())
        }
    }
}

/// Saves waiting source `src` into the library and marks it so.
fn move_in(db: &mut dyn Db, lib: &LibraryAt, src: &StoredSource) -> Result<(), LibraryFault> {
    let bytes = source_bytes(db, src.id)?
        .ok_or_else(|| StoreError::Corrupt(format!("waiting source {} has no bytes", src.id)))?;
    let info = paper_info(db, &src.paper_id)?;
    let scan = lib
        .library
        .scan()
        .map_err(|e| LibraryFault::Unavailable(e.to_string()))?;
    if let Some((folder, why)) = foreign_jats(db, lib, &scan, src, &info)? {
        return Err(LibraryFault::Unavailable(format!(
            "the library's folder \u{201c}{folder}\u{201d} for this paper holds the JATS of another paper ({why}), so nothing is put there until that folder is put right"
        )));
    }
    let file = lib
        .library
        .save(&lib.who, &info, &bytes, library_kind(src.kind), &src.origin)
        .map_err(|e| LibraryFault::Unavailable(e.to_string()))?;
    if file.sha256 != src.sha256 {
        return Err(StoreError::Corrupt(format!(
            "source {}'s stored digest {} is not its bytes' {}",
            src.id, src.sha256, file.sha256
        ))
        .into());
    }
    if transaction_with(db, |tx| mark_in_library(tx, src.id))? {
        log::info!(
            "source {} of paper {} moved into the library as {}/{}",
            src.id,
            src.paper_id.0,
            file.folder,
            file.name
        );
    }
    Ok(())
}

/// The first of the paper's library folders (those `find` gives for `info`)
/// holding a JATS whose own identifiers contradict the paper's, with why. A
/// record is another app's claim, not proof (design §4.4): bmlib's
/// `FullTextService` makes this check for a lookup, `Library::save` does not
/// (shortcut: checked here until bmlib #766 puts it in `save` for every app).
/// A JATS that cannot be read is skipped, logged: it names no paper.
fn foreign_jats(
    db: &mut dyn Db,
    lib: &LibraryAt,
    scan: &Scan,
    src: &StoredSource,
    info: &PaperInfo,
) -> Result<Option<(String, String)>, StoreError> {
    for folder in find(scan, &info.ids) {
        let mut seen = Vec::new();
        let entries = folder.records.iter().flat_map(|r| r.files.iter());
        for entry in entries.filter(|e| e.kind == LibraryKind::Jats) {
            if seen.contains(&&entry.sha256) {
                continue;
            }
            seen.push(&entry.sha256);
            let file = FileRef {
                folder: folder.folder.clone(),
                name: entry.name.clone(),
                sha256: entry.sha256.clone(),
                kind: entry.kind,
            };
            let bytes = match lib.library.read(&file) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!(
                        "the library's {}/{} is not checked: {e}",
                        file.folder,
                        file.name
                    );
                    continue;
                }
            };
            if let Some(why) = contradiction(db, &src.paper_id, &jats_identifiers(&bytes).0)? {
                log::warn!(
                    "the library's {}/{} is another paper's JATS: {why}",
                    file.folder,
                    file.name
                );
                return Ok(Some((folder.folder.clone(), why)));
            }
        }
    }
    Ok(None)
}

/// The banner's sentence while the source of the paper's latest text, or the
/// PDF kept beside it, waits for the library; `None` when nothing waits.
pub fn waiting_notice(db: &mut dyn Db, paper: &PaperId) -> Result<Option<String>, StoreError> {
    let Some(text) = latest_canonical(db, paper)? else {
        return Ok(None);
    };
    let src = source_by_id(db, text.source_id)?;
    let beside = pdf_beside_source(db, src.id)?;
    let waiting: Vec<&StoredSource> = std::iter::once(&src)
        .chain(beside.as_ref())
        .filter(|s| s.place == SourcePlace::Waiting)
        .collect();
    let Some(first) = waiting.first() else {
        return Ok(None);
    };
    let why = waiting
        .iter()
        .find_map(|s| s.library_note.as_deref())
        .unwrap_or("it has not been put there yet");
    Ok(Some(format!(
        "{} is kept in the app for now, not in the paper library: {why}. It moves there once the library can take it.",
        describe(first)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identifiers::Identifiers;
    use crate::store::{
        create_paper, open_memory_store, save_canonical, save_pdf_beside, save_source, NewSource,
    };
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_plain_text};
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// A fresh, empty folder under the temp dir.
    fn temp_folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sa-lib-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A database whose settings name a new library folder, and that folder.
    fn with_library() -> (Connection, PathBuf) {
        let mut db = open_memory_store().unwrap();
        let dir = temp_folder();
        set_setting(&mut db, LIBRARY_PATH_KEY, dir.to_str().unwrap()).unwrap();
        (db, dir)
    }

    /// The usable library `db`'s settings name.
    fn ready(db: &mut Connection) -> LibraryAt {
        match configured(db, "StudyAnalyzer on a test").unwrap() {
            Configured::Ready(lib) => lib,
            other => panic!("not ready: {other:?}"),
        }
    }

    /// A paper with PMID `pmid` and a text read from a waiting plain-text
    /// source of `bytes`; returns the paper and the source's id.
    fn waiting_paper(db: &mut Connection, pmid: &str, bytes: &[u8]) -> (PaperId, i64) {
        let ids = Identifiers {
            pmid: Some(pmid.into()),
            ..Default::default()
        };
        let p = create_paper(db, &ids, Some("A trial")).unwrap();
        let src = NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: bytes.to_vec(),
            decoded: None,
        };
        let sid = save_source(db, &p, src, SourcePlace::Waiting).unwrap();
        let text = canonical_from_plain_text("Para one.", "T");
        let a = assess_full_text(&text).unwrap();
        save_canonical(db, &p, sid, &text, &a).unwrap();
        (p, sid)
    }

    /// A JATS naming PMID `pmid`.
    fn jats(pmid: &str) -> Vec<u8> {
        format!(
            "<article><front><article-meta><article-id pub-id-type=\"pmid\">{pmid}</article-id>\
             <title-group><article-title>A trial</article-title></title-group></article-meta>\
             </front><body><sec><title>Results</title><p>Mortality fell.</p></sec></body></article>"
        )
        .into_bytes()
    }

    /// No setting is no library; a folder is opened (made a library when
    /// empty), the installation id kept; a missing folder is unreachable.
    #[test]
    fn configured_follows_the_setting() {
        let mut db = open_memory_store().unwrap();
        assert!(matches!(
            configured(&mut db, "x").unwrap(),
            Configured::None
        ));
        assert_eq!(Configured::None.new_place(), SourcePlace::App);
        let dir = temp_folder();
        set_setting(&mut db, LIBRARY_PATH_KEY, dir.to_str().unwrap()).unwrap();
        let first = ready(&mut db);
        assert!(dir.join(bmlib::library::MARKER_FILE).is_file());
        assert_eq!(first.who.id, ready(&mut db).who.id);
        assert_eq!(first.who.id, first.who.id.to_lowercase());
        assert_eq!(first.who.label, "StudyAnalyzer on a test");
        let gone = dir.join("missing");
        set_setting(&mut db, LIBRARY_PATH_KEY, gone.to_str().unwrap()).unwrap();
        let c = configured(&mut db, "x").unwrap();
        assert!(matches!(&c, Configured::Unreachable(why) if why.contains("missing")));
        assert_eq!(c.new_place(), SourcePlace::Waiting);
        std::fs::remove_dir_all(dir).ok();
    }

    /// A label bmlib refuses falls back to the app's name.
    #[test]
    fn a_refused_label_falls_back() {
        let (mut db, dir) = with_library();
        let lib = match configured(&mut db, "").unwrap() {
            Configured::Ready(lib) => lib,
            other => panic!("not ready: {other:?}"),
        };
        assert_eq!(lib.who.label, LIBRARY_LABEL_FALLBACK);
        std::fs::remove_dir_all(dir).ok();
    }

    /// Plain text is the library's `text`; the others keep their names.
    #[test]
    fn kinds_map_to_the_librarys() {
        assert_eq!(library_kind(SourceKind::PlainText), LibraryKind::Text);
        assert_eq!(library_kind(SourceKind::Pdf), LibraryKind::Pdf);
        assert_eq!(library_kind(SourceKind::Jats), LibraryKind::Jats);
        assert_eq!(library_kind(SourceKind::Html), LibraryKind::Html);
        assert_eq!(library_kind(SourceKind::Markdown), LibraryKind::Markdown);
    }

    /// A preprint accession is left out of the record; the title goes in,
    /// unless the paper is untitled.
    #[test]
    fn paper_info_leaves_a_preprint_accession_out() {
        let mut db = open_memory_store().unwrap();
        let ids = Identifiers {
            pmid: Some("PPR123".into()),
            doi: Some("10.1/x".into()),
            pmc_id: None,
        };
        let p = create_paper(&mut db, &ids, None).unwrap();
        let info = paper_info(&mut db, &p).unwrap();
        assert_eq!(info.ids.pmid, None);
        assert_eq!(info.ids.doi.as_deref(), Some("10.1/x"));
        assert_eq!(info.title, None);
    }

    /// Settling moves a waiting source in: the bytes go from the database,
    /// the file reads back by digest, the banner has nothing to say.
    #[test]
    fn settle_moves_a_waiting_source_in() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let (p, sid) = waiting_paper(&mut db, "1", b"Para one.");
        assert!(waiting_notice(&mut db, &p).unwrap().is_some());
        let s = settle(&mut db, &lib, None).unwrap();
        assert_eq!(
            s,
            Settled {
                moved: 1,
                left: vec![]
            }
        );
        let src = source_by_id(&mut db, sid).unwrap();
        assert_eq!(src.place, SourcePlace::Library);
        assert_eq!(source_bytes(&mut db, sid).unwrap(), None);
        let c = Configured::Ready(lib.clone());
        assert_eq!(read_source(&mut db, &c, &src).unwrap(), b"Para one.");
        assert_eq!(waiting_notice(&mut db, &p).unwrap(), None);
        // Saved as the library's `text`.
        let file = find_digest(&lib.library.scan().unwrap(), &src.sha256).unwrap();
        assert_eq!(file.kind, LibraryKind::Text);
        // Nothing waits, so a second settle moves nothing.
        assert_eq!(settle(&mut db, &lib, None).unwrap(), Settled::default());
        std::fs::remove_dir_all(dir).ok();
    }

    /// The same bytes on two papers are one file in the library.
    #[test]
    fn the_same_bytes_are_one_file() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        waiting_paper(&mut db, "1", b"Same bytes.");
        waiting_paper(&mut db, "2", b"Same bytes.");
        assert_eq!(settle(&mut db, &lib, None).unwrap().moved, 2);
        let scan = lib.library.scan().unwrap();
        let files: usize = scan.folders.iter().map(|f| f.files_on_disk.len()).sum();
        assert_eq!(files, 1);
        std::fs::remove_dir_all(dir).ok();
    }

    /// An unreachable library leaves the source waiting with why, and the
    /// banner says so; once reachable, it moves in.
    #[test]
    fn an_unreachable_library_leaves_the_source_waiting() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let (p, sid) = waiting_paper(&mut db, "1", b"Para one.");
        let gone = Configured::Unreachable("the library folder /x cannot be used: gone".into());
        settle_paper(&mut db, &gone, &p).unwrap();
        let notice = waiting_notice(&mut db, &p).unwrap().unwrap();
        assert!(notice.contains("kept in the app for now"), "{notice}");
        assert!(notice.contains("/x cannot be used"), "{notice}");
        assert!(source_bytes(&mut db, sid).unwrap().is_some());
        settle_paper(&mut db, &Configured::Ready(lib), &p).unwrap();
        assert_eq!(waiting_notice(&mut db, &p).unwrap(), None);
        std::fs::remove_dir_all(dir).ok();
    }

    /// A paper whose library folder holds another paper's JATS puts nothing
    /// there: its source waits, the note naming the folder.
    #[test]
    fn a_folder_holding_another_papers_jats_is_refused() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        // Another app filed PMID 2's JATS under PMID 1.
        let claim = PaperInfo {
            ids: bmlib::library::PaperIds::canonical(Some("1"), None, None).unwrap(),
            title: Some("A trial".into()),
            year: None,
            first_author: None,
        };
        let other = Installation::new("medicalfactchecker", "bbbb", "MFC").unwrap();
        lib.library
            .save(&other, &claim, &jats("2"), LibraryKind::Jats, "x")
            .unwrap();
        let (p, sid) = waiting_paper(&mut db, "1", b"Para one.");
        let s = settle(&mut db, &lib, None).unwrap();
        assert_eq!(s.moved, 0);
        assert_eq!(s.left.len(), 1);
        assert!(
            s.left[0].1.contains("holds the JATS of another paper"),
            "{s:?}"
        );
        let src = source_by_id(&mut db, sid).unwrap();
        assert_eq!(src.place, SourcePlace::Waiting);
        assert!(waiting_notice(&mut db, &p)
            .unwrap()
            .unwrap()
            .contains("PMID"));
        std::fs::remove_dir_all(dir).ok();
    }

    /// A folder whose JATS is the paper's own takes the source.
    #[test]
    fn a_folder_holding_its_own_jats_takes_the_source() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let claim = PaperInfo {
            ids: bmlib::library::PaperIds::canonical(Some("1"), None, None).unwrap(),
            title: Some("A trial".into()),
            year: None,
            first_author: None,
        };
        let other = Installation::new("medicalfactchecker", "bbbb", "MFC").unwrap();
        lib.library
            .save(&other, &claim, &jats("1"), LibraryKind::Jats, "x")
            .unwrap();
        waiting_paper(&mut db, "1", b"Para one.");
        assert_eq!(settle(&mut db, &lib, None).unwrap().moved, 1);
        assert_eq!(lib.library.scan().unwrap().folders.len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    /// A read from the library checks the digest: a changed file and a
    /// removed one are unavailable, named; so is any library file when no
    /// library is set or it cannot be reached.
    #[test]
    fn a_read_checks_the_digest() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let (_, sid) = waiting_paper(&mut db, "1", b"Para one.");
        settle(&mut db, &lib, None).unwrap();
        let src = source_by_id(&mut db, sid).unwrap();
        let file = find_digest(&lib.library.scan().unwrap(), &src.sha256).unwrap();
        let path = dir.join("papers").join(&file.folder).join(&file.name);
        let c = Configured::Ready(lib);
        for (configured, want) in [
            (Configured::None, "no library folder is set"),
            (
                Configured::Unreachable("the library folder /x cannot be used: gone".into()),
                "/x cannot be used",
            ),
        ] {
            let e = read_source(&mut db, &configured, &src).unwrap_err();
            assert!(e.to_string().contains(want), "{e}");
        }
        std::fs::write(&path, b"Changed.").unwrap();
        let e = read_source(&mut db, &c, &src).unwrap_err();
        assert!(matches!(e, LibraryFault::Unavailable(_)), "{e:?}");
        assert!(e.to_string().starts_with("This paper's text file"), "{e}");
        std::fs::remove_file(&path).unwrap();
        let e = read_source(&mut db, &c, &src).unwrap_err();
        assert!(
            e.to_string().contains("no longer in the paper library"),
            "{e}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A PDF kept beside the text's source that waits shows in the banner too.
    #[test]
    fn a_waiting_pdf_beside_shows_in_the_banner() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let (p, sid) = waiting_paper(&mut db, "1", b"Para one.");
        settle(&mut db, &lib, None).unwrap();
        assert!(save_pdf_beside(
            &mut db,
            &p,
            sid,
            "x",
            b"%PDF-1".to_vec(),
            SourcePlace::Waiting
        )
        .unwrap());
        let notice = waiting_notice(&mut db, &p).unwrap().unwrap();
        assert!(notice.starts_with("This paper's pdf file"), "{notice}");
        settle(&mut db, &lib, Some(&p)).unwrap();
        assert_eq!(waiting_notice(&mut db, &p).unwrap(), None);
        std::fs::remove_dir_all(dir).ok();
    }
}
