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
    find, find_digest, FileRef, Installation, Library, LibraryError, PaperFolder, PaperInfo, Scan,
    SourceKind as LibraryKind, DIGEST_PREFIX_HEX, MARKER_FILE,
};

use crate::config::{
    LIBRARY_APP, LIBRARY_BOOKMARK_FAULT_KEY, LIBRARY_INSTALLATION_KEY, LIBRARY_LABEL_FALLBACK,
    LIBRARY_OPENED_KEY, LIBRARY_PATH_KEY, PREPRINT_PREFIX, UNTITLED_PAPER,
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
/// refuses it). An empty folder becomes a library the first time it is
/// opened ([`opened_before`]); once opened, a folder that has lost its
/// marker (an unmounted drive leaves its empty mount point) is unreachable.
/// One that cannot be used is [`Configured::Unreachable`], logged.
pub fn configured(db: &mut dyn Db, label: &str) -> Result<Configured, StoreError> {
    let Some(path) = get_setting(db, LIBRARY_PATH_KEY)? else {
        return Ok(Configured::None);
    };
    let who = installation(db, label)?;
    let root = Path::new(&path);
    let opened_before = opened_before(db, &path)?;
    if opened_before && root.is_dir() && !root.join(MARKER_FILE).exists() {
        let why = format!(
            "the library folder {path} no longer holds the paper library (is its drive unplugged?)"
        );
        log::warn!("{why}");
        return unreachable(db, why);
    }
    Ok(match Library::open(root) {
        Ok(library) => {
            if !opened_before {
                remember_opened(db, &path)?;
            }
            Configured::Ready(LibraryAt { library, who })
        }
        Err(e) => {
            log::warn!("the library at {path} cannot be used: {e}");
            return unreachable(db, format!("the library folder {path} cannot be used: {e}"));
        }
    })
}

/// [`Configured::Unreachable`] with `why`, ending with why the library
/// could not be reached through its bookmark at launch when it could not
/// ([`LIBRARY_BOOKMARK_FAULT_KEY`]).
fn unreachable(db: &mut dyn Db, why: String) -> Result<Configured, StoreError> {
    Ok(Configured::Unreachable(
        match get_setting(db, LIBRARY_BOOKMARK_FAULT_KEY)? {
            Some(fault) => format!("{why}; at launch: {fault}"),
            None => why,
        },
    ))
}

/// Whether this app has opened the library folder `path` before (one
/// settings key per folder, [`LIBRARY_OPENED_KEY`]`:path`; the key alone is
/// the one folder an older build kept). Such a folder that has lost its
/// marker is not made a new library.
pub(crate) fn opened_before(db: &mut dyn Db, path: &str) -> Result<bool, StoreError> {
    Ok(
        get_setting(db, &format!("{LIBRARY_OPENED_KEY}:{path}"))?.is_some()
            || get_setting(db, LIBRARY_OPENED_KEY)?.as_deref() == Some(path),
    )
}

/// Remembers that this app opened the library folder `path` (see
/// [`opened_before`]); kept when the setting changes, so a folder chosen
/// again is checked too.
pub(crate) fn remember_opened(db: &mut dyn Db, path: &str) -> Result<(), StoreError> {
    set_setting(db, &format!("{LIBRARY_OPENED_KEY}:{path}"), path)
}

/// This installation: its id from the settings, made (a lower-case UUID) and
/// kept on first use, inside a transaction so two commands make one.
pub(crate) fn installation(db: &mut dyn Db, label: &str) -> Result<Installation, StoreError> {
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
/// library, found by digest and kind and checked against the digest. A file
/// the library cannot give (none chosen or reachable, removed, damaged,
/// online-only) is [`LibraryFault::Unavailable`], naming it. `src` may be out
/// of date (a settle may have moved it in since it was read), so a source
/// without bytes here is looked up again before it is called corrupt.
pub fn read_source(
    db: &mut dyn Db,
    configured: &Configured,
    src: &StoredSource,
) -> Result<Vec<u8>, LibraryFault> {
    if let Some(bytes) = source_bytes(db, src.id)? {
        return Ok(bytes);
    }
    if src.place != SourcePlace::Library && source_by_id(db, src.id)?.place != SourcePlace::Library
    {
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
                "{what} is kept in the paper library, and no library folder is set{}",
                left_in(src)
            )))
        }
        Configured::Unreachable(why) => {
            return Err(LibraryFault::Unavailable(format!(
                "{what} is kept in the paper library, and {why}"
            )))
        }
    };
    // shortcut: a scan per read, no index (bmlib measures 0.12 s at 1,000 papers); keep one if reads get slow.
    let scan = lib
        .library
        .scan()
        .map_err(|e| LibraryFault::Unavailable(format!("{what}: {e}")))?;
    read_scanned(lib, &scan, src)
}

/// The bytes of `src`, kept in `lib`, found by digest and kind in `scan`
/// (one scan for many reads) and checked against the digest. A file the
/// library cannot give is [`LibraryFault::Unavailable`], naming it.
pub(crate) fn read_scanned(
    lib: &LibraryAt,
    scan: &Scan,
    src: &StoredSource,
) -> Result<Vec<u8>, LibraryFault> {
    let what = describe(src);
    let unavailable = |e: &dyn fmt::Display| LibraryFault::Unavailable(format!("{what}: {e}"));
    let Some(file) = find_digest(scan, &src.sha256, Some(library_kind(src.kind))) else {
        return Err(LibraryFault::Unavailable(format!(
            "{what} is no longer in the paper library{}",
            left_in(src)
        )));
    };
    lib.library.read(&file).map_err(|e| unavailable(&e))
}

/// Whether the library at hand still holds `src`'s file, found by digest and
/// kind in a fresh scan. No library, an unreachable one or a failed scan
/// (logged) is false, so a caller holding the bytes keeps them (#124).
pub fn holds(configured: &Configured, src: &StoredSource) -> bool {
    let Configured::Ready(lib) = configured else {
        return false;
    };
    match lib.library.scan() {
        Ok(scan) => find_digest(&scan, &src.sha256, Some(library_kind(src.kind))).is_some(),
        Err(e) => {
            log::warn!(
                "source {}: the paper library could not be scanned ({e}); keeping its bytes here",
                src.id
            );
            false
        }
    }
}

/// Where a change of the setting left library source `src`'s file, as a
/// clause to end a sentence ("; its file was left in …"), or nothing.
fn left_in(src: &StoredSource) -> String {
    match (&src.place, &src.library_note) {
        (SourcePlace::Library, Some(note)) => format!("; {note}"),
        _ => String::new(),
    }
}

/// The source as a phrase for the user: its kind and its digest's start.
fn describe(src: &StoredSource) -> String {
    let short: String = src.sha256.chars().take(DIGEST_PREFIX_HEX).collect();
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
    /// One entry per source still waiting: its paper and why.
    pub left: Vec<(PaperId, String)>,
}

/// Moves the waiting sources (of `paper`, or of every paper) into the
/// library: each is saved there (bytes kept once by digest) and recorded as
/// this installation's, then marked as in the library and its bytes dropped
/// here. A source the library cannot take stays waiting, with why in its
/// note, logged; so does one whose paper's library folder holds another
/// paper's JATS ([`foreign_jats`]), and one the database fails on (logged as
/// an error), so one bad source does not hold up the others. A source
/// another thread moved first is passed over.
pub fn settle(
    db: &mut dyn Db,
    lib: &LibraryAt,
    paper: Option<&PaperId>,
) -> Result<Settled, StoreError> {
    let mut settled = Settled::default();
    for src in waiting_sources(db, paper)? {
        let why = match move_in(db, lib, &src) {
            Ok(true) => {
                settled.moved += 1;
                continue;
            }
            Ok(false) => continue,
            Err(LibraryFault::Store(e)) => {
                log::error!(
                    "source {} of paper {} cannot be moved into the library: {e}",
                    src.id,
                    src.paper_id.0
                );
                format!("the app's storage failed on it ({e})")
            }
            Err(LibraryFault::Unavailable(why)) => {
                log::warn!(
                    "source {} of paper {} stays waiting for the library: {why}",
                    src.id,
                    src.paper_id.0
                );
                why
            }
        };
        note_waiting(db, src.id, &why)?;
        settled.left.push((src.paper_id.clone(), why));
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

/// Saves waiting source `src` into the library and marks it so; true when
/// this call moved it, false when another thread had moved it first.
fn move_in(db: &mut dyn Db, lib: &LibraryAt, src: &StoredSource) -> Result<bool, LibraryFault> {
    let Some(bytes) = source_bytes(db, src.id)? else {
        if source_by_id(db, src.id)?.place != SourcePlace::Waiting {
            log::info!(
                "source {} was moved into the library by another thread",
                src.id
            );
            return Ok(false);
        }
        return Err(StoreError::Corrupt(format!("waiting source {} has no bytes", src.id)).into());
    };
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
    let moved = transaction_with(db, |tx| mark_in_library(tx, src.id))?;
    if moved {
        log::info!(
            "source {} of paper {} moved into the library as {}/{}",
            src.id,
            src.paper_id.0,
            file.folder,
            file.name
        );
    }
    Ok(moved)
}

/// The first of the folders `Library::save` may put `src` in (the one
/// already holding its bytes, and those `find` gives for `info`) holding a
/// JATS whose own identifiers contradict the paper's, with why. A record is
/// another app's claim, not proof (design §4.4): bmlib's `FullTextService`
/// makes this check for a lookup, `Library::save` does not (shortcut: checked
/// here until bmlib #766 puts it in `save` for every app). A JATS that cannot
/// be read now (online-only, a busy drive) is [`LibraryFault::Unavailable`],
/// so the source waits and is checked again later; one a record lists that
/// is missing, with no placeholder (removed by a manual clear), is passed
/// over.
fn foreign_jats(
    db: &mut dyn Db,
    lib: &LibraryAt,
    scan: &Scan,
    src: &StoredSource,
    info: &PaperInfo,
) -> Result<Option<(String, String)>, LibraryFault> {
    let holder = find_digest(scan, &src.sha256, None)
        .and_then(|r| scan.folders.iter().find(|f| f.folder == r.folder));
    let mut folders: Vec<&PaperFolder> = holder.into_iter().collect();
    for f in find(scan, &info.ids) {
        if !folders.iter().any(|g| g.folder == f.folder) {
            folders.push(f);
        }
    }
    for folder in folders {
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
                // A file a record still lists after a clear removed it is no paper's JATS.
                Err(LibraryError::Missing { .. }) => {
                    log::info!(
                        "the library's {}/{} is listed but gone; not checked for another paper's JATS",
                        file.folder,
                        file.name
                    );
                    continue;
                }
                Err(e) => {
                    return Err(LibraryFault::Unavailable(format!(
                        "the library's {}/{} cannot be checked for another paper's JATS now: {e}",
                        file.folder, file.name
                    )))
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
        let file = find_digest(&lib.library.scan().unwrap(), &src.sha256, None).unwrap();
        assert_eq!(file.kind, LibraryKind::Text);
        // Nothing waits, so a second settle moves nothing.
        assert_eq!(settle(&mut db, &lib, None).unwrap(), Settled::default());
        std::fs::remove_dir_all(dir).ok();
    }

    /// `holds` is true for a file the library has, by digest and kind, and
    /// false once it is removed, for another kind, and without a library.
    #[test]
    fn holds_finds_the_file_by_digest_and_kind() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let (_, sid) = waiting_paper(&mut db, "1", b"Para one.");
        settle(&mut db, &lib, None).unwrap();
        let src = source_by_id(&mut db, sid).unwrap();
        let c = Configured::Ready(lib.clone());
        assert!(holds(&c, &src));
        let as_md = StoredSource {
            kind: SourceKind::Markdown,
            ..src.clone()
        };
        assert!(!holds(&c, &as_md));
        assert!(!holds(&Configured::None, &src));
        assert!(!holds(&Configured::Unreachable("gone".into()), &src));
        let file = find_digest(&lib.library.scan().unwrap(), &src.sha256, None).unwrap();
        lib.library.remove_file(&file).unwrap();
        assert!(!holds(&c, &src));
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
        let file = find_digest(&lib.library.scan().unwrap(), &src.sha256, None).unwrap();
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

    /// A waiting source of `kind` and `bytes` on paper `p`, no text read from it.
    fn waiting_source(db: &mut Connection, p: &PaperId, kind: SourceKind, bytes: &[u8]) -> i64 {
        let src = NewSource {
            kind,
            origin: "upload".into(),
            file_name: None,
            bytes: bytes.to_vec(),
            decoded: None,
        };
        save_source(db, p, src, SourcePlace::Waiting).unwrap()
    }

    /// The same bytes kept as two kinds are two files, and each source reads
    /// its own kind's (`find_digest` with the source's kind).
    #[test]
    fn the_same_bytes_as_two_kinds_each_read_back() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let (p, text) = waiting_paper(&mut db, "1", b"# Notes");
        let md = waiting_source(&mut db, &p, SourceKind::Markdown, b"# Notes");
        assert_eq!(settle(&mut db, &lib, None).unwrap().moved, 2);
        let c = Configured::Ready(lib);
        for id in [text, md] {
            let src = source_by_id(&mut db, id).unwrap();
            assert_eq!(src.place, SourcePlace::Library);
            assert_eq!(read_source(&mut db, &c, &src).unwrap(), b"# Notes");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// A source whose stored digest is not its bytes' keeps its bytes and
    /// waits, noted, and the sources after it still move in.
    #[test]
    fn a_wrong_digest_keeps_the_bytes_and_settling_goes_on() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let (_, bad) = waiting_paper(&mut db, "1", b"Para one.");
        let (_, good) = waiting_paper(&mut db, "2", b"Para two.");
        Connection::execute(
            &db,
            "UPDATE sources SET sha256 = ?1 WHERE id = ?2",
            rusqlite::params!["0".repeat(64), bad],
        )
        .unwrap();
        let s = settle(&mut db, &lib, None).unwrap();
        assert_eq!(s.moved, 1);
        assert_eq!(s.left.len(), 1);
        assert!(s.left[0].1.contains("is not its bytes'"), "{s:?}");
        let src = source_by_id(&mut db, bad).unwrap();
        assert_eq!(src.place, SourcePlace::Waiting);
        assert!(src.library_note.unwrap().contains("storage failed"));
        assert_eq!(source_bytes(&mut db, bad).unwrap().unwrap(), b"Para one.");
        assert_eq!(
            source_by_id(&mut db, good).unwrap().place,
            SourcePlace::Library
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A source another thread moved in after it was listed is passed over by
    /// `move_in` and read from the library by `read_source`, not called corrupt.
    #[test]
    fn a_source_moved_by_another_thread_is_not_corrupt() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let (_, sid) = waiting_paper(&mut db, "1", b"Para one.");
        let stale = source_by_id(&mut db, sid).unwrap();
        assert!(move_in(&mut db, &lib, &stale).unwrap());
        assert_eq!(stale.place, SourcePlace::Waiting);
        assert!(!move_in(&mut db, &lib, &stale).unwrap());
        let c = Configured::Ready(lib);
        assert_eq!(read_source(&mut db, &c, &stale).unwrap(), b"Para one.");
        std::fs::remove_dir_all(dir).ok();
    }

    /// A source kept in the app whose bytes are gone is corrupt, and the
    /// library is not asked.
    #[test]
    fn an_app_source_without_bytes_is_corrupt() {
        let mut db = open_memory_store().unwrap();
        let ids = Identifiers {
            pmid: Some("1".into()),
            ..Default::default()
        };
        let p = create_paper(&mut db, &ids, None).unwrap();
        let src = NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: b"x".to_vec(),
            decoded: None,
        };
        let sid = save_source(&mut db, &p, src, SourcePlace::App).unwrap();
        Connection::execute(&db, "DELETE FROM source_bytes WHERE source_id = ?1", [sid]).unwrap();
        let src = source_by_id(&mut db, sid).unwrap();
        let e = read_source(&mut db, &Configured::None, &src).unwrap_err();
        assert!(
            matches!(e, LibraryFault::Store(StoreError::Corrupt(_))),
            "{e:?}"
        );
    }

    /// Once opened, a library folder that comes back empty (an unmounted
    /// drive's mount point) is unreachable, not made a library again; a
    /// newly chosen empty folder still becomes one.
    #[test]
    fn an_opened_library_that_comes_back_empty_is_unreachable() {
        let (mut db, dir) = with_library();
        ready(&mut db);
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let c = configured(&mut db, "x").unwrap();
        assert!(
            matches!(&c, Configured::Unreachable(why) if why.contains("no longer holds")),
            "{c:?}"
        );
        assert!(!dir.join(MARKER_FILE).exists());
        let other = temp_folder();
        set_setting(&mut db, LIBRARY_PATH_KEY, other.to_str().unwrap()).unwrap();
        ready(&mut db);
        assert!(other.join(MARKER_FILE).is_file());
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(other).ok();
    }

    /// A JATS of another installation's in folder `"1"`'s paper, then
    /// `harm` done to its file in the library folder `dir`; whether the
    /// waiting source of paper 1 then moved in, and the settle.
    fn settle_after(harm: impl Fn(&Path, &FileRef)) -> (bool, Settled) {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let claim = PaperInfo {
            ids: bmlib::library::PaperIds::canonical(Some("1"), None, None).unwrap(),
            title: Some("A trial".into()),
            year: None,
            first_author: None,
        };
        let other = Installation::new("medicalfactchecker", "bbbb", "MFC").unwrap();
        let file = lib
            .library
            .save(&other, &claim, &jats("1"), LibraryKind::Jats, "x")
            .unwrap();
        harm(&dir, &file);
        let (_, sid) = waiting_paper(&mut db, "1", b"Para one.");
        let s = settle(&mut db, &lib, None).unwrap();
        let moved = source_by_id(&mut db, sid).unwrap().place == SourcePlace::Library;
        std::fs::remove_dir_all(dir).ok();
        (moved, s)
    }

    /// A JATS in the paper's folder that cannot be read now (damaged,
    /// online-only) leaves the source waiting, so it is checked again later.
    #[test]
    fn an_unreadable_jats_in_the_folder_leaves_the_source_waiting() {
        let damaged = |dir: &Path, f: &FileRef| {
            std::fs::write(
                dir.join("papers").join(&f.folder).join(&f.name),
                b"<article/>",
            )
            .unwrap();
        };
        let online_only = |dir: &Path, f: &FileRef| {
            let folder = dir.join("papers").join(&f.folder);
            std::fs::remove_file(folder.join(&f.name)).unwrap();
            let placeholder = bmlib::library::ICLOUD_PLACEHOLDER.replace("{name}", &f.name);
            std::fs::write(folder.join(placeholder), b"").unwrap();
        };
        for (what, harm) in [
            ("damaged", &damaged as &dyn Fn(&Path, &FileRef)),
            ("online-only", &online_only),
        ] {
            let (moved, s) = settle_after(harm);
            assert!(!moved, "{what}: {s:?}");
            assert_eq!(s.moved, 0, "{what}");
            assert!(s.left[0].1.contains("cannot be checked"), "{what}: {s:?}");
        }
    }

    /// A JATS a record still lists that the manual clear removed is passed
    /// over: the source moves in.
    #[test]
    fn a_removed_jats_in_the_folder_is_passed_over() {
        let (moved, s) = settle_after(|dir, f| {
            std::fs::remove_file(dir.join("papers").join(&f.folder).join(&f.name)).unwrap();
        });
        assert!(moved, "{s:?}");
        assert_eq!(s.moved, 1, "{s:?}");
        assert!(s.left.is_empty(), "{s:?}");
    }

    /// The folder already holding the bytes, where `save` puts them, is
    /// checked too, though the paper's identifiers do not name it.
    #[test]
    fn the_folder_holding_the_bytes_is_checked_for_another_papers_jats() {
        let (mut db, dir) = with_library();
        let lib = ready(&mut db);
        let claim = PaperInfo {
            ids: bmlib::library::PaperIds::canonical(Some("2"), None, None).unwrap(),
            title: Some("Another trial".into()),
            year: None,
            first_author: None,
        };
        let other = Installation::new("medicalfactchecker", "bbbb", "MFC").unwrap();
        for (bytes, kind) in [
            (jats("2"), LibraryKind::Jats),
            (b"Para one.".to_vec(), LibraryKind::Text),
        ] {
            lib.library.save(&other, &claim, &bytes, kind, "x").unwrap();
        }
        let (_, sid) = waiting_paper(&mut db, "1", b"Para one.");
        let s = settle(&mut db, &lib, None).unwrap();
        assert_eq!(s.moved, 0);
        assert!(
            s.left[0].1.contains("holds the JATS of another paper"),
            "{s:?}"
        );
        assert_eq!(
            source_by_id(&mut db, sid).unwrap().place,
            SourcePlace::Waiting
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
