//! SQLite storage for papers, their original sources and canonical texts.

use std::fmt;
use std::path::Path;

use bmlib::db::{
    execute, fetch_all, fetch_one, get_applied_versions, open_memory, open_path, run_migrations,
    transaction, transaction_with, Db, DbError, Row, Value,
};
use bmlib::embed::store::{delete_paper, ensure_schema, StoreError as EmbedStoreError};
use bmlib::fulltext::canonical::{CanonicalText, FullTextAssessment};
use bmlib::library::sha256_hex;
use rusqlite::Connection;
use serde::Serialize;

use crate::config::{DB_BUSY_TIMEOUT, UNTITLED_PAPER};
use crate::identifiers::Identifiers;
use crate::intake::{Decoded, SourceKind};
use crate::passages::index_key;
use crate::schema::migrations;

/// What can go wrong in storage.
#[derive(Debug)]
pub enum StoreError {
    /// The database refused or failed.
    Db(DbError),
    /// Stored data is not what this code wrote (unknown kind, invalid canonical text).
    Corrupt(String),
    /// The paper asked for does not exist.
    NotFound(String),
    /// An identifier is already held by another paper.
    Conflict(String),
    /// The database was written by a newer app: it holds a schema version
    /// this code does not know, so it is not opened (bmlib's `run_migrations`
    /// would ignore the unknown version and open it as current).
    Newer {
        /// The highest schema version in the database.
        found: i64,
        /// The highest schema version this code knows.
        known: i64,
    },
}

impl fmt::Display for StoreError {
    /// Renders the error for logs and for the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Db(e) => write!(f, "{e}"),
            StoreError::Corrupt(m) => write!(f, "stored data is corrupt: {m}"),
            StoreError::NotFound(m) => write!(f, "not found: {m}"),
            StoreError::Conflict(m) => write!(f, "conflict: {m}"),
            StoreError::Newer { found, known } => write!(
                f,
                "the database was written by a newer StudyAnalyzer (schema {found}; \
                 this version knows up to {known}): update the app"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<DbError> for StoreError {
    /// Wraps a database error.
    fn from(e: DbError) -> Self {
        StoreError::Db(e)
    }
}

/// The id of a stored paper (uuid v4 text). Only this crate builds one, from
/// a row it read or wrote; code outside turns an id it was given into one with
/// [`paper_by_id`], which checks the paper exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperId(pub(crate) String);

impl PaperId {
    /// The id as text, for the frontend and for SQL parameters.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One line of the recent-papers list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaperSummary {
    /// The paper's id.
    pub id: String,
    /// The paper's title.
    pub title: String,
    /// The identifiers known for the paper.
    pub identifiers: Identifiers,
    /// When the paper was last opened (stored text, UTC).
    pub opened_at: String,
}

/// An original file or pasted text to store.
#[derive(Debug, Clone)]
pub struct NewSource {
    /// What kind of source it is.
    pub kind: SourceKind,
    /// `"upload"`, `"paste"` or the name of the full-text provider.
    pub origin: String,
    /// The file name, when there was one.
    pub file_name: Option<String>,
    /// The original bytes.
    pub bytes: Vec<u8>,
    /// How the bytes were decoded; `None` for a PDF.
    pub decoded: Option<Decoded>,
}

/// Where a stored source's bytes are (migration 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePlace {
    /// In the app's own storage (`source_bytes`): stored while no library was
    /// set (every source before migration 11).
    App,
    /// In the app's own storage until the shared library takes them
    /// (`library::settle`); the paper's banner says so.
    Waiting,
    /// In the shared library only; `sha256` and `kind` are the reference.
    Library,
}

impl SourcePlace {
    /// The stable name used in storage.
    pub fn as_str(self) -> &'static str {
        match self {
            SourcePlace::App => "app",
            SourcePlace::Waiting => "waiting",
            SourcePlace::Library => "library",
        }
    }

    /// The place stored as `name`, if it is one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "app" => Some(SourcePlace::App),
            "waiting" => Some(SourcePlace::Waiting),
            "library" => Some(SourcePlace::Library),
            _ => None,
        }
    }
}

/// A stored source. Its bytes are read apart ([`source_bytes`], or
/// `library::read_source` wherever they are).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSource {
    /// Row id.
    pub id: i64,
    /// The paper it belongs to.
    pub paper_id: PaperId,
    /// What kind of source it is.
    pub kind: SourceKind,
    /// `"upload"`, `"paste"`, a hand-off's origin or the full-text provider.
    pub origin: String,
    /// The file name, when there was one (the title hint for re-deriving).
    pub file_name: Option<String>,
    /// The lower-case hex SHA-256 of its bytes: its reference in the library.
    pub sha256: String,
    /// Where its bytes are.
    pub place: SourcePlace,
    /// Why a waiting source is not in the library yet, once a move failed or
    /// the library could not be reached; for a library source, the folder a
    /// change of the setting left its file in.
    pub library_note: Option<String>,
    /// How the bytes were decoded; `None` for a PDF.
    pub decoded: Option<Decoded>,
}

/// A stored canonical text with its assessment.
#[derive(Debug, Clone)]
pub struct StoredCanonical {
    /// Row id.
    pub id: i64,
    /// The source it was derived from.
    pub source_id: i64,
    /// The validated canonical text.
    pub text: CanonicalText,
    /// The full-text assessment, as JSON.
    pub assessment_json: String,
    /// Whether the user confirmed "this is the whole paper".
    pub confirmed_full_text: bool,
    /// The last failed attempt to re-derive this text, if any.
    pub rederive_failure: Option<RederiveFailure>,
}

/// A failed attempt to re-derive a canonical text from its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RederiveFailure {
    /// The converter version that was tried.
    pub converter_version: String,
    /// Why it failed, for the user.
    pub error: String,
}

/// Opens (creating if needed) the database file with WAL and foreign keys, and migrates it,
/// refusing a database a newer app wrote ([`StoreError::Newer`]).
///
/// Transactions begin `IMMEDIATE`: a deferred transaction that reads and then
/// writes fails at once with `SQLITE_BUSY` if another connection wrote in
/// between, ignoring the busy timeout; taking the write lock at begin makes
/// the transaction wait for it instead.
pub fn open_store(path: &Path) -> Result<Connection, StoreError> {
    let mut conn = open_path(path, true, true)?;
    conn.busy_timeout(DB_BUSY_TIMEOUT)
        .map_err(|e| StoreError::Db(DbError::Backend(e.to_string())))?;
    prepare(&mut conn)?;
    Ok(conn)
}

/// Opens a fresh in-memory database, migrated and with `IMMEDIATE`
/// transactions as [`open_store`]'s. For tests.
pub fn open_memory_store() -> Result<Connection, StoreError> {
    let mut conn = open_memory()?;
    prepare(&mut conn)?;
    Ok(conn)
}

/// What every opened connection gets: `IMMEDIATE` transactions, a refusal
/// of a database a newer app wrote ([`StoreError::Newer`]), and the
/// migrations it lacks.
fn prepare(conn: &mut Connection) -> Result<(), StoreError> {
    conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
    let all = migrations();
    let known = all.iter().map(|m| m.version).max().unwrap_or(0);
    if let Some(found) = get_applied_versions(conn)?.into_iter().max() {
        if found > known {
            return Err(StoreError::Newer { found, known });
        }
    }
    run_migrations(conn, all)?;
    // bmlib's chunk tables, which 2b's retrieval stores a paper's chunks in.
    ensure_schema(conn).map_err(chunk_error)?;
    Ok(())
}

/// A failure of bmlib's chunk tables as a [`StoreError`].
fn chunk_error(e: EmbedStoreError) -> StoreError {
    match e {
        EmbedStoreError::Db(e) => StoreError::Db(e),
        other => StoreError::Corrupt(format!("chunk tables: {other}")),
    }
}

/// The three identifier columns, in lookup order.
const ID_COLUMNS: [&str; 3] = ["pmid", "doi", "pmc_id"];

/// The identifiers as `(column, value)` pairs in lookup order.
fn id_pairs(ids: &Identifiers) -> [(&'static str, &Option<String>); 3] {
    [
        (ID_COLUMNS[0], &ids.pmid),
        (ID_COLUMNS[1], &ids.doi),
        (ID_COLUMNS[2], &ids.pmc_id),
    ]
}

/// The paper with this id, if one is stored. The way to turn an id from
/// outside (the frontend) into a [`PaperId`].
pub fn paper_by_id(db: &mut dyn Db, id: &str) -> Result<Option<PaperId>, StoreError> {
    Ok(
        fetch_one(db, "SELECT id FROM papers WHERE id = ?", &[Value::from(id)])?
            .map(|_| PaperId(id.to_string())),
    )
}

/// Finds the paper holding any of the identifiers: PMID first, then DOI, then PMC ID.
pub fn find_paper(db: &mut dyn Db, ids: &Identifiers) -> Result<Option<PaperId>, StoreError> {
    Ok(holders(db, ids)?.into_iter().next())
}

/// Every paper holding one of the identifiers, each once, in the order
/// [`find_paper`] tries them (PMID, DOI, PMC ID).
pub fn holders(db: &mut dyn Db, ids: &Identifiers) -> Result<Vec<PaperId>, StoreError> {
    let mut found = Vec::new();
    for (col, val) in id_pairs(ids) {
        let Some(v) = val else { continue };
        if let Some(p) = holder_of(db, col, v)?.filter(|p| !found.contains(p)) {
            found.push(p);
        }
    }
    Ok(found)
}

/// Creates a paper with the given identifiers; the title defaults to `Untitled paper`.
///
/// Call [`find_paper`] first: an identifier another paper already holds is
/// refused with [`StoreError::Conflict`] naming the field and value.
pub fn create_paper(
    db: &mut dyn Db,
    ids: &Identifiers,
    title: Option<&str>,
) -> Result<PaperId, StoreError> {
    let id = uuid::Uuid::new_v4().to_string();
    for (col, val) in id_pairs(ids) {
        if let Some(v) = val {
            if holder_of(db, col, v)?.is_some() {
                return Err(StoreError::Conflict(format!(
                    "{col} {v} already belongs to another paper"
                )));
            }
        }
    }
    execute(
        db,
        "INSERT INTO papers (id, pmid, doi, pmc_id, title) VALUES (?, ?, ?, ?, ?)",
        &[
            Value::from(id.as_str()),
            Value::from(ids.pmid.clone()),
            Value::from(ids.doi.clone()),
            Value::from(ids.pmc_id.clone()),
            Value::from(title.unwrap_or(UNTITLED_PAPER)),
        ],
    )?;
    Ok(PaperId(id))
}

/// The id of the paper holding `value` in identifier column `col`, if any.
fn holder_of(db: &mut dyn Db, col: &str, value: &str) -> Result<Option<PaperId>, StoreError> {
    fetch_one(
        db,
        &format!("SELECT id FROM papers WHERE {col} = ?"),
        &[Value::from(value)],
    )?
    .map(|r| r.get_str("id").map(|id| PaperId(id.to_string())))
    .transpose()
    .map_err(StoreError::from)
}

/// The paper's identifier columns, or [`StoreError::NotFound`].
fn stored_ids(db: &mut dyn Db, paper: &PaperId) -> Result<Row, StoreError> {
    fetch_one(
        db,
        "SELECT pmid, doi, pmc_id FROM papers WHERE id = ?",
        &[Value::from(paper.0.as_str())],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("no paper {}", paper.0)))
}

/// The paper's stored identifiers, or [`StoreError::NotFound`].
pub fn paper_identifiers(db: &mut dyn Db, paper: &PaperId) -> Result<Identifiers, StoreError> {
    let row = stored_ids(db, paper)?;
    let text = |col: &str| -> Result<Option<String>, StoreError> {
        Ok(row.get(col)?.as_str().map(str::to_string))
    };
    Ok(Identifiers {
        pmid: text("pmid")?,
        doi: text("doi")?,
        pmc_id: text("pmc_id")?,
    })
}

/// Why a document carrying `ids` contradicts `paper`: the first identifier
/// whose value differs from the paper's stored one. `None` when none does
/// (an identifier the paper lacks is no contradiction).
pub fn contradiction(
    db: &mut dyn Db,
    paper: &PaperId,
    ids: &Identifiers,
) -> Result<Option<String>, StoreError> {
    let row = stored_ids(db, paper)?;
    for (col, val) in id_pairs(ids) {
        let Some(v) = val else { continue };
        if let Some(stored) = row.get(col)?.as_str().filter(|s| s != v) {
            let label = id_label(col);
            return Ok(Some(format!(
                "its {label} is {v}, but this paper's {label} is {stored}"
            )));
        }
    }
    Ok(None)
}

/// Why a document carrying `ids` is not `paper`: a [`contradiction`], else
/// the first identifier that another paper already holds (named by title).
/// `None` when every identifier agrees with the paper or is new to the store.
pub fn identifier_conflict(
    db: &mut dyn Db,
    paper: &PaperId,
    ids: &Identifiers,
) -> Result<Option<String>, StoreError> {
    if let Some(why) = contradiction(db, paper, ids)? {
        return Ok(Some(why));
    }
    for (col, val) in id_pairs(ids) {
        let Some(v) = val else { continue };
        if let Some(holder) = holder_of(db, col, v)?.filter(|h| h != paper) {
            return Ok(Some(format!(
                "its {} {v} belongs to another stored paper, {}",
                id_label(col),
                quoted_title(db, &holder)?
            )));
        }
    }
    Ok(None)
}

/// The title of `paper` in quotation marks, as a notice names a paper. A
/// stored notice keeps the title the paper had then; the identifier the
/// notice also names is what tells the paper apart for good.
pub fn quoted_title(db: &mut dyn Db, paper: &PaperId) -> Result<String, StoreError> {
    Ok(format!("\u{201c}{}\u{201d}", paper_title(db, paper)?))
}

/// The name a user knows an identifier column by.
fn id_label(col: &str) -> &'static str {
    match col {
        "pmid" => "PMID",
        "doi" => "DOI",
        _ => "PMC ID",
    }
}

/// Fills the paper's absent identifiers. A different stored value is kept, and an
/// identifier another paper already holds is skipped (that paper named by
/// title); both are logged and returned as notices for the user, one sentence each.
pub fn add_identifiers(
    db: &mut dyn Db,
    paper: &PaperId,
    ids: &Identifiers,
) -> Result<Vec<String>, StoreError> {
    transaction_with(db, |db| {
        let mut notices = Vec::new();
        let row = stored_ids(db, paper)?;
        for (col, val) in id_pairs(ids) {
            let Some(v) = val else { continue };
            let label = id_label(col);
            match row.get(col)?.as_str() {
                Some(stored) if stored == v => {}
                Some(stored) => {
                    log::warn!(
                        "paper {}: keeping stored {col} {stored}, not overwriting with {v}",
                        paper.0
                    );
                    notices.push(format!(
                        "{label} {v} was not recorded: this paper's {label} is {stored}."
                    ));
                }
                None => {
                    if let Some(holder) = holder_of(db, col, v)? {
                        log::warn!(
                            "paper {}: {col} {v} already belongs to paper {}, skipped",
                            paper.0,
                            holder.0
                        );
                        notices.push(format!(
                            "{label} {v} was not recorded: it belongs to another stored paper, {}.",
                            quoted_title(db, &holder)?
                        ));
                    } else {
                        execute(
                            db,
                            &format!("UPDATE papers SET {col} = ? WHERE id = ?"),
                            &[Value::from(v.as_str()), Value::from(paper.0.as_str())],
                        )?;
                    }
                }
            }
        }
        Ok(notices)
    })
}

/// `NotFound` when an UPDATE on a paper touched no row.
fn require_row(affected: u64, paper: &PaperId) -> Result<(), StoreError> {
    if affected == 0 {
        return Err(StoreError::NotFound(format!("no paper {}", paper.0)));
    }
    Ok(())
}

/// Sets the paper's title.
pub fn set_title(db: &mut dyn Db, paper: &PaperId, title: &str) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE papers SET title = ? WHERE id = ?",
        &[Value::from(title), Value::from(paper.0.as_str())],
    )?;
    require_row(n, paper)
}

/// Marks the paper as opened now.
pub fn touch_opened(db: &mut dyn Db, paper: &PaperId) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE papers SET opened_at = datetime('now') WHERE id = ?",
        &[Value::from(paper.0.as_str())],
    )?;
    require_row(n, paper)
}

/// Stores an original source for the paper, its bytes in the app's storage
/// at `place` ([`SourcePlace::App`] or [`SourcePlace::Waiting`]), and returns
/// its row id. Takes the source by value so its bytes, up to the size cap,
/// are moved into the insert rather than copied. Call it inside a
/// transaction: the row and its bytes are two inserts.
pub fn save_source(
    db: &mut dyn Db,
    paper: &PaperId,
    src: NewSource,
    place: SourcePlace,
) -> Result<i64, StoreError> {
    let (enc, guessed) = match src.decoded {
        Some(d) => (Some(d.encoding), d.guessed),
        None => (None, false),
    };
    execute(
        db,
        "INSERT INTO sources (paper_id, kind, origin, file_name, sha256, place, encoding, encoding_guessed)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::from(paper.0.as_str()),
            Value::from(src.kind.as_str()),
            Value::from(src.origin.as_str()),
            Value::from(src.file_name),
            Value::from(sha256_hex(&src.bytes)),
            Value::from(place.as_str()),
            Value::from(enc),
            Value::from(guessed),
        ],
    )?;
    let id = last_id(db)?;
    save_bytes(db, id, src.bytes)?;
    Ok(id)
}

/// Keeps source `id`'s bytes in the app's storage.
fn save_bytes(db: &mut dyn Db, id: i64, bytes: Vec<u8>) -> Result<(), StoreError> {
    execute(
        db,
        "INSERT INTO source_bytes (source_id, bytes) VALUES (?, ?)",
        &[Value::from(id), Value::from(bytes)],
    )?;
    Ok(())
}

/// The bytes of source `id` kept in the app's storage, or `None` when it has
/// none here (it is in the library).
pub fn source_bytes(db: &mut dyn Db, id: i64) -> Result<Option<Vec<u8>>, StoreError> {
    fetch_one(
        db,
        "SELECT bytes FROM source_bytes WHERE source_id = ?",
        &[Value::from(id)],
    )?
    .map(|r| match r.get("bytes")? {
        Value::Blob(bytes) => Ok(bytes.clone()),
        other => Err(StoreError::Corrupt(format!(
            "source {id}'s bytes are not a BLOB: {other:?}"
        ))),
    })
    .transpose()
}

/// The waiting sources ([`SourcePlace::Waiting`]), oldest first: of `paper`
/// only, or of every paper.
pub fn waiting_sources(
    db: &mut dyn Db,
    paper: Option<&PaperId>,
) -> Result<Vec<StoredSource>, StoreError> {
    sources_with_place(db, paper, SourcePlace::Waiting)
}

/// The sources at `place` (of `paper`, or of every paper), oldest first.
pub fn sources_with_place(
    db: &mut dyn Db,
    paper: Option<&PaperId>,
    place: SourcePlace,
) -> Result<Vec<StoredSource>, StoreError> {
    let mut params = vec![Value::from(place.as_str())];
    let filter = match paper {
        Some(p) => {
            params.push(Value::from(p.0.as_str()));
            " AND paper_id = ?"
        }
        None => "",
    };
    fetch_all(
        db,
        &format!("{SOURCE_SELECT} WHERE place = ?{filter} ORDER BY id"),
        &params,
    )?
    .iter()
    .map(source_from_row)
    .collect()
}

/// How many sources are kept at each place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct PlaceCounts {
    /// Bytes in the app for good, not waiting for the library (a library may
    /// still be set: `library_setting::move_in` moves these).
    pub app: usize,
    /// Bytes in the app, to be moved into the library.
    pub waiting: usize,
    /// In the library only.
    pub library: usize,
}

/// Counts the sources at each place.
pub fn count_by_place(db: &mut dyn Db) -> Result<PlaceCounts, StoreError> {
    let mut counts = PlaceCounts::default();
    for row in fetch_all(
        db,
        "SELECT place, COUNT(*) AS n FROM sources GROUP BY place",
        &[],
    )? {
        let place_text = row.get_str("place")?;
        let n = usize::try_from(row.get_i64("n")?)
            .map_err(|_| StoreError::Corrupt("a negative source count".to_string()))?;
        match SourcePlace::parse(place_text) {
            Some(SourcePlace::App) => counts.app = n,
            Some(SourcePlace::Waiting) => counts.waiting = n,
            Some(SourcePlace::Library) => counts.library = n,
            None => {
                return Err(StoreError::Corrupt(format!(
                    "{n} source(s) have an unknown place {place_text:?}"
                )))
            }
        }
    }
    Ok(counts)
}

/// The papers with at least one source at `place`, in the order they were made.
pub fn papers_with_place(db: &mut dyn Db, place: SourcePlace) -> Result<Vec<PaperId>, StoreError> {
    fetch_all(
        db,
        "SELECT paper_id FROM sources WHERE place = ? GROUP BY paper_id ORDER BY MIN(id)",
        &[Value::from(place.as_str())],
    )?
    .iter()
    .map(|r| Ok(PaperId(r.get_str("paper_id")?.to_string())))
    .collect()
}

/// A changed-row count as a `usize` (it is never more than the table holds).
fn count(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// Marks every source kept in the app as waiting for the library (the move
/// of existing papers, design §4.7); returns how many.
pub fn app_to_waiting(db: &mut dyn Db) -> Result<usize, StoreError> {
    execute(
        db,
        "UPDATE sources SET place = 'waiting' WHERE place = 'app'",
        &[],
    )
    .map(count)
    .map_err(StoreError::from)
}

/// Keeps every waiting source in the app for good, its note cleared (no
/// library is set any more); returns how many.
pub fn waiting_to_app(db: &mut dyn Db) -> Result<usize, StoreError> {
    execute(
        db,
        "UPDATE sources SET place = 'app', library_note = NULL WHERE place = 'waiting'",
        &[],
    )
    .map(count)
    .map_err(StoreError::from)
}

/// Gives source `id`, kept in the library only, its `bytes` back here, now
/// at `place` (waiting for another library, or the app). Returns false,
/// changing nothing, when it is not in the library. Bytes that are not its
/// digest's are [`StoreError::Corrupt`]. Call it inside a transaction.
pub fn bring_back(
    db: &mut dyn Db,
    id: i64,
    bytes: &[u8],
    place: SourcePlace,
) -> Result<bool, StoreError> {
    let src = source_by_id(db, id)?;
    if src.place != SourcePlace::Library {
        return Ok(false);
    }
    let found = sha256_hex(bytes);
    if found != src.sha256 {
        return Err(StoreError::Corrupt(format!(
            "the bytes brought back for source {id} have digest {found}, not its {}",
            src.sha256
        )));
    }
    execute(
        db,
        "INSERT INTO source_bytes (source_id, bytes) VALUES (?, ?)",
        &[Value::from(id), Value::Blob(bytes.to_vec())],
    )?;
    execute(
        db,
        "UPDATE sources SET place = ?, library_note = NULL WHERE id = ?",
        &[Value::from(place.as_str()), Value::from(id)],
    )?;
    Ok(true)
}

/// Marks waiting source `id` as kept in the library only and drops its bytes
/// here. Returns false, changing nothing, when it is not waiting (another
/// thread moved it first). Call it inside a transaction.
pub fn mark_in_library(db: &mut dyn Db, id: i64) -> Result<bool, StoreError> {
    let n = execute(
        db,
        "UPDATE sources SET place = 'library', library_note = NULL WHERE id = ? AND place = 'waiting'",
        &[Value::from(id)],
    )?;
    if n == 0 {
        return Ok(false);
    }
    execute(
        db,
        "DELETE FROM source_bytes WHERE source_id = ?",
        &[Value::from(id)],
    )?;
    Ok(true)
}

/// Records where library source `id`'s file is, when the library set does
/// not hold it (a switch left it in the old folder); a read that misses it
/// names that place. Changes nothing when the source is not in the library.
pub fn note_library(db: &mut dyn Db, id: i64, note: &str) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE sources SET library_note = ? WHERE id = ? AND place = 'library'",
        &[Value::from(note), Value::from(id)],
    )?;
    if n == 0 {
        log::info!("source {id} is no longer in the library; its note is not kept");
    }
    Ok(())
}

/// Gives every library source without a note `note`: a change of the setting
/// is leaving them in the old folder. Returns how many.
pub fn note_left_behind(db: &mut dyn Db, note: &str) -> Result<usize, StoreError> {
    execute(
        db,
        "UPDATE sources SET library_note = ? WHERE place = 'library' AND library_note IS NULL",
        &[Value::from(note)],
    )
    .map(count)
    .map_err(StoreError::from)
}

/// Clears `note` from every library source holding it: the folder it
/// names is the library again. Returns how many.
pub fn clear_library_note(db: &mut dyn Db, note: &str) -> Result<usize, StoreError> {
    execute(
        db,
        "UPDATE sources SET library_note = NULL WHERE place = 'library' AND library_note = ?",
        &[Value::from(note)],
    )
    .map(count)
    .map_err(StoreError::from)
}

/// Records why waiting source `id` could not be moved into the library.
pub fn note_waiting(db: &mut dyn Db, id: i64, note: &str) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE sources SET library_note = ? WHERE id = ? AND place = 'waiting'",
        &[Value::from(note), Value::from(id)],
    )?;
    if n == 0 {
        log::info!("source {id} is no longer waiting; its library note is not kept");
    }
    Ok(())
}

/// The row id of the insert just made on this connection.
pub(crate) fn last_id(db: &mut dyn Db) -> Result<i64, StoreError> {
    db.last_insert_rowid()
        .ok_or_else(|| StoreError::Corrupt("insert returned no row id".to_string()))
}

/// The paper's most recently stored source, if any.
pub fn latest_source(db: &mut dyn Db, paper: &PaperId) -> Result<Option<StoredSource>, StoreError> {
    fetch_one(
        db,
        &format!("{SOURCE_SELECT} WHERE paper_id = ? ORDER BY id DESC LIMIT 1"),
        &[Value::from(paper.0.as_str())],
    )?
    .as_ref()
    .map(source_from_row)
    .transpose()
}

/// The stored source with this row id (the one a canonical text was derived
/// from), or [`StoreError::NotFound`].
pub fn source_by_id(db: &mut dyn Db, id: i64) -> Result<StoredSource, StoreError> {
    fetch_one(
        db,
        &format!("{SOURCE_SELECT} WHERE id = ?"),
        &[Value::from(id)],
    )?
    .as_ref()
    .map(source_from_row)
    .transpose()?
    .ok_or_else(|| StoreError::NotFound(format!("source {id}")))
}

/// The paper that most recently stored a source of this kind whose bytes
/// have this SHA-256 ([`sha256_hex`]), and that source's file name, if any:
/// how a document carrying no identifiers finds its paper again (#44),
/// wherever its bytes are kept.
pub fn holder_of_source(
    db: &mut dyn Db,
    kind: SourceKind,
    sha256: &str,
) -> Result<Option<(PaperId, Option<String>)>, StoreError> {
    fetch_one(
        db,
        "SELECT paper_id, file_name FROM sources
         WHERE kind = ? AND sha256 = ?
         ORDER BY id DESC LIMIT 1",
        &[Value::from(kind.as_str()), Value::from(sha256)],
    )?
    .map(|r| {
        Ok::<_, StoreError>((
            PaperId(r.get_str("paper_id")?.to_string()),
            r.get("file_name")?.as_str().map(str::to_string),
        ))
    })
    .transpose()
}

/// Stores the notices given when source `id` was stored, replacing any earlier
/// ones; [`StoreError::NotFound`] when no source has this id.
pub fn record_source_notices(
    db: &mut dyn Db,
    id: i64,
    notices: &[String],
) -> Result<(), StoreError> {
    let json = serde_json::to_string(notices)
        .map_err(|e| StoreError::Corrupt(format!("cannot serialise notices: {e}")))?;
    let n = execute(
        db,
        "UPDATE sources SET notices_json = ? WHERE id = ?",
        &[Value::from(json), Value::from(id)],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("source {id}")));
    }
    Ok(())
}

/// The notices stored with source `id` (empty for a source stored before
/// migration 3), or [`StoreError::NotFound`].
pub fn source_notices(db: &mut dyn Db, id: i64) -> Result<Vec<String>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT notices_json FROM sources WHERE id = ?",
        &[Value::from(id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("source {id}")))?;
    serde_json::from_str(row.get_str("notices_json")?)
        .map_err(|e| StoreError::Corrupt(format!("source {id}'s notices are invalid: {e}")))
}

/// The kind of the stored source with this row id and how it was decoded
/// (`None` for a PDF), without loading its bytes.
pub fn source_info(db: &mut dyn Db, id: i64) -> Result<(SourceKind, Option<Decoded>), StoreError> {
    let row = fetch_one(
        db,
        "SELECT kind, encoding, encoding_guessed FROM sources WHERE id = ?",
        &[Value::from(id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("source {id}")))?;
    Ok((kind_from_row(&row)?, decoded_from_row(&row)?))
}

/// The PDF source for the *Original* tab of the paper's latest canonical
/// text: the PDF it was derived from, else the PDF kept beside its source
/// ([`save_pdf_beside`]); `None` when there is neither or the paper has no
/// text yet. Its bytes are read apart (`library::read_source`).
pub fn original_pdf_source(
    db: &mut dyn Db,
    paper: &PaperId,
) -> Result<Option<StoredSource>, StoreError> {
    let Some(canonical) = latest_canonical(db, paper)? else {
        return Ok(None);
    };
    let source = source_by_id(db, canonical.source_id)?;
    if source.kind == SourceKind::Pdf {
        return Ok(Some(source));
    }
    pdf_beside_source(db, canonical.source_id)
}

/// The bytes of [`original_pdf_source`], read from the app's storage: for
/// tests, whose sources are never in the library.
#[cfg(test)]
pub(crate) fn original_pdf(
    db: &mut dyn Db,
    paper: &PaperId,
) -> Result<Option<Vec<u8>>, StoreError> {
    original_pdf_source(db, paper)?
        .map(|s| {
            source_bytes(db, s.id)?
                .ok_or_else(|| StoreError::Corrupt(format!("source {} has no bytes here", s.id)))
        })
        .transpose()
}

/// The PDF kept beside source `beside` ([`save_pdf_beside`]), if any.
pub fn pdf_beside_source(db: &mut dyn Db, beside: i64) -> Result<Option<StoredSource>, StoreError> {
    fetch_one(
        db,
        &format!("{SOURCE_SELECT} WHERE pdf_beside = ? ORDER BY id DESC LIMIT 1"),
        &[Value::from(beside)],
    )?
    .as_ref()
    .map(source_from_row)
    .transpose()
}

/// Whether the text derived from source `source_id` has a PDF for the
/// *Original* tab: the source is one, or a PDF is kept beside it. Reads no
/// bytes.
pub fn has_original_pdf(db: &mut dyn Db, source_id: i64) -> Result<bool, StoreError> {
    let row = fetch_one(
        db,
        "SELECT kind = ? OR EXISTS (SELECT 1 FROM sources WHERE pdf_beside = s.id) AS has
         FROM sources s WHERE s.id = ?",
        &[
            Value::from(SourceKind::Pdf.as_str()),
            Value::from(source_id),
        ],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("source {source_id}")))?;
    Ok(row.get_i64("has")? != 0)
}

/// Keeps `bytes`, a PDF of the paper, beside its source `beside` (the JATS
/// its text was read from) for the *Original* tab, at `place` (as
/// [`save_source`] takes it), unless one is kept there already. Returns
/// whether it was stored. Call it inside a transaction.
pub fn save_pdf_beside(
    db: &mut dyn Db,
    paper: &PaperId,
    beside: i64,
    origin: &str,
    bytes: Vec<u8>,
    place: SourcePlace,
) -> Result<bool, StoreError> {
    if fetch_one(
        db,
        "SELECT 1 FROM sources WHERE pdf_beside = ?",
        &[Value::from(beside)],
    )?
    .is_some()
    {
        return Ok(false);
    }
    execute(
        db,
        "INSERT INTO sources (paper_id, kind, origin, sha256, place, pdf_beside) VALUES (?, ?, ?, ?, ?, ?)",
        &[
            Value::from(paper.0.as_str()),
            Value::from(SourceKind::Pdf.as_str()),
            Value::from(origin),
            Value::from(sha256_hex(&bytes)),
            Value::from(place.as_str()),
            Value::from(beside),
        ],
    )?;
    let id = last_id(db)?;
    save_bytes(db, id, bytes)?;
    Ok(true)
}

/// The columns every source query reads.
const SOURCE_SELECT: &str =
    "SELECT id, paper_id, kind, origin, file_name, sha256, place, library_note,
     encoding, encoding_guessed FROM sources";

/// The `encoding` and `encoding_guessed` columns of a `sources` row.
fn decoded_from_row(row: &Row) -> Result<Option<Decoded>, StoreError> {
    let guessed = row.get_i64("encoding_guessed")? != 0;
    Ok(row.get("encoding")?.as_str().map(|e| Decoded {
        encoding: e.to_string(),
        guessed,
    }))
}

/// The `kind` column of a `sources` row; an unknown kind is corrupt.
fn kind_from_row(row: &Row) -> Result<SourceKind, StoreError> {
    let kind_text = row.get_str("kind")?;
    SourceKind::parse(kind_text)
        .ok_or_else(|| StoreError::Corrupt(format!("unknown source kind {kind_text:?}")))
}

/// Decodes a `sources` row selected with [`SOURCE_SELECT`].
fn source_from_row(row: &Row) -> Result<StoredSource, StoreError> {
    let id = row.get_i64("id")?;
    let kind = kind_from_row(row)?;
    let decoded = decoded_from_row(row)?;
    let place_text = row.get_str("place")?;
    let place = SourcePlace::parse(place_text).ok_or_else(|| {
        StoreError::Corrupt(format!("source {id} has an unknown place {place_text:?}"))
    })?;
    let sha256 = row
        .get("sha256")?
        .as_str()
        .ok_or_else(|| StoreError::Corrupt(format!("source {id} has no digest")))?
        .to_string();
    let text = |name: &str| -> Result<Option<String>, StoreError> {
        Ok(row.get(name)?.as_str().map(str::to_string))
    };
    Ok(StoredSource {
        id,
        paper_id: PaperId(row.get_str("paper_id")?.to_string()),
        kind,
        origin: row.get_str("origin")?.to_string(),
        file_name: text("file_name")?,
        sha256,
        place,
        library_note: text("library_note")?,
        decoded,
    })
}

/// Stores a canonical text and its assessment, derived from `source_id`, and
/// drops the chunk sets of the paper's older texts: only the latest text is
/// ever searched, and a set can always be rebuilt. Call it inside a
/// transaction, so the new text and the dropped sets land together.
pub fn save_canonical(
    db: &mut dyn Db,
    paper: &PaperId,
    source_id: i64,
    text: &CanonicalText,
    assessment: &FullTextAssessment,
) -> Result<i64, StoreError> {
    let canonical_json = serde_json::to_string(text)
        .map_err(|e| StoreError::Corrupt(format!("cannot serialise canonical text: {e}")))?;
    let assessment_json = serde_json::to_string(assessment)
        .map_err(|e| StoreError::Corrupt(format!("cannot serialise assessment: {e}")))?;
    execute(
        db,
        "INSERT INTO canonical_texts (paper_id, source_id, converter_version, canonical_json, assessment_json)
         VALUES (?, ?, ?, ?, ?)",
        &[
            Value::from(paper.0.as_str()),
            Value::from(source_id),
            Value::from(text.converter_version.as_str()),
            Value::from(canonical_json),
            Value::from(assessment_json),
        ],
    )?;
    let id = last_id(db)?;
    let older = fetch_all(
        db,
        "SELECT id FROM canonical_texts WHERE paper_id = ? AND id <> ?",
        &[Value::from(paper.0.as_str()), Value::from(id)],
    )?;
    // A failed drop fails the save on purpose: a stale set left behind would go unnoticed.
    for row in older {
        delete_paper(db, &index_key(row.get_i64("id")?)).map_err(chunk_error)?;
    }
    Ok(id)
}

/// Drops the chunk set of canonical text `canonical_id` if its paper has a
/// newer text, and says whether it did. An ask indexes on its own connection,
/// so a newer text saved meanwhile may drop the older sets before the ask
/// writes its own (#55). Calling this after the write closes that gap:
/// a newer text saved before the call is seen here, and one saved after it
/// drops the set itself.
pub fn drop_superseded_chunk_set(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError> {
    let newer = fetch_one(
        db,
        "SELECT newer.id FROM canonical_texts AS this
         JOIN canonical_texts AS newer ON newer.paper_id = this.paper_id AND newer.id > this.id
         WHERE this.id = ? LIMIT 1",
        &[Value::from(canonical_id)],
    )?;
    if newer.is_none() {
        return Ok(false);
    }
    delete_paper(db, &index_key(canonical_id)).map_err(chunk_error)?;
    log::info!(
        "dropped the chunk set of canonical text {canonical_id}: its paper has a newer text"
    );
    Ok(true)
}

/// The paper's most recently stored canonical text, validated on the way out.
pub fn latest_canonical(
    db: &mut dyn Db,
    paper: &PaperId,
) -> Result<Option<StoredCanonical>, StoreError> {
    read_canonical(db, paper, None)
}

/// The id of the paper's newest canonical text, without reading the text
/// itself; `None` when it has none.
pub fn latest_canonical_id(db: &mut dyn Db, paper: &PaperId) -> Result<Option<i64>, StoreError> {
    fetch_one(
        db,
        "SELECT id FROM canonical_texts WHERE paper_id = ? ORDER BY id DESC LIMIT 1",
        &[Value::from(paper.0.as_str())],
    )?
    .map(|r| r.get_i64("id"))
    .transpose()
    .map_err(Into::into)
}

/// The paper's canonical text `canonical_id`, validated on the way out;
/// `None` when the paper has no text with that id (another paper's included).
pub fn canonical_by_id(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<Option<StoredCanonical>, StoreError> {
    read_canonical(db, paper, Some(canonical_id))
}

/// The paper's canonical text `canonical_id`, or its newest one for `None`.
fn read_canonical(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: Option<i64>,
) -> Result<Option<StoredCanonical>, StoreError> {
    let Some(row) = fetch_one(
        db,
        "SELECT id, source_id, canonical_json, assessment_json, confirmed_full_text,
                rederive_failed_version, rederive_error
         FROM canonical_texts WHERE paper_id = ? AND (? IS NULL OR id = ?)
         ORDER BY id DESC LIMIT 1",
        &[
            Value::from(paper.0.as_str()),
            Value::from(canonical_id),
            Value::from(canonical_id),
        ],
    )?
    else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(row.get_str("canonical_json")?)
        .map_err(|e| StoreError::Corrupt(format!("canonical text is not JSON: {e}")))?;
    let text = CanonicalText::from_value(value)
        .map_err(|e| StoreError::Corrupt(format!("canonical text is invalid: {e}")))?;
    Ok(Some(StoredCanonical {
        id: row.get_i64("id")?,
        source_id: row.get_i64("source_id")?,
        text,
        assessment_json: row.get_str("assessment_json")?.to_string(),
        confirmed_full_text: row.get_i64("confirmed_full_text")? != 0,
        rederive_failure: match row.get("rederive_failed_version")?.as_str() {
            Some(version) => Some(RederiveFailure {
                converter_version: version.to_string(),
                error: match row.get("rederive_error")?.as_str() {
                    Some(e) => e.to_string(),
                    // Only a damaged row gets here: record_rederive_failure writes both.
                    None => {
                        log::warn!(
                            "paper {}'s canonical text records a failed version but no error",
                            paper.0
                        );
                        "the cause was not recorded".to_string()
                    }
                },
            }),
            None => None,
        },
    }))
}

/// Records that re-deriving canonical text `canonical_id` with `failure`'s
/// converter version failed; [`StoreError::NotFound`] when no text has this id.
pub fn record_rederive_failure(
    db: &mut dyn Db,
    canonical_id: i64,
    failure: &RederiveFailure,
) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE canonical_texts SET rederive_failed_version = ?, rederive_error = ? WHERE id = ?",
        &[
            Value::from(failure.converter_version.as_str()),
            Value::from(failure.error.as_str()),
            Value::from(canonical_id),
        ],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!(
            "canonical text {canonical_id}"
        )));
    }
    Ok(())
}

/// Records that the user confirmed this canonical text of `paper` is the whole
/// paper; [`StoreError::NotFound`] when `paper` has no canonical text with this id.
pub fn confirm_full_text(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE canonical_texts SET confirmed_full_text = 1 WHERE id = ? AND paper_id = ?",
        &[Value::from(canonical_id), Value::from(paper.as_str())],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!(
            "canonical text {canonical_id} of paper {}",
            paper.0
        )));
    }
    Ok(())
}

/// Builds a summary from a `papers` row.
fn summary_from_row(row: &Row) -> Result<PaperSummary, StoreError> {
    let opt = |c: &str| -> Result<Option<String>, StoreError> {
        Ok(row.get(c)?.as_str().map(str::to_string))
    };
    Ok(PaperSummary {
        id: row.get_str("id")?.to_string(),
        title: row.get_str("title")?.to_string(),
        identifiers: Identifiers {
            pmid: opt("pmid")?,
            doi: opt("doi")?,
            pmc_id: opt("pmc_id")?,
        },
        opened_at: row.get_str("opened_at")?.to_string(),
    })
}

/// Records the note of the paper's last lookup that stored no text on it: why
/// the document fetched was not stored, and what the lookup noted (`None`
/// clears it). A reopen without text shows it as the reason.
/// [`StoreError::NotFound`] when no such paper exists.
pub fn set_refusal(
    db: &mut dyn Db,
    paper: &PaperId,
    refusal: Option<&str>,
) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE papers SET refusal = ? WHERE id = ?",
        &[
            Value::from(refusal.map(str::to_string)),
            Value::from(paper.0.as_str()),
        ],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("paper {}", paper.0)));
    }
    Ok(())
}

/// What [`set_refusal`] last recorded for the paper, or [`StoreError::NotFound`].
pub fn refusal(db: &mut dyn Db, paper: &PaperId) -> Result<Option<String>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT refusal FROM papers WHERE id = ?",
        &[Value::from(paper.0.as_str())],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("paper {}", paper.0)))?;
    Ok(row.get("refusal")?.as_str().map(str::to_string))
}

/// The paper's title, or [`StoreError::NotFound`] if no such paper exists.
pub fn paper_title(db: &mut dyn Db, paper: &PaperId) -> Result<String, StoreError> {
    let row = fetch_one(
        db,
        "SELECT title FROM papers WHERE id = ?",
        &[Value::from(paper.0.as_str())],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("paper {}", paper.0)))?;
    Ok(row.get_str("title")?.to_string())
}

/// Every paper, most recently opened first (ties: most recently created first).
pub fn recent_papers(db: &mut dyn Db) -> Result<Vec<PaperSummary>, StoreError> {
    fetch_all(
        db,
        "SELECT id, title, pmid, doi, pmc_id, opened_at FROM papers
         ORDER BY opened_at DESC, rowid DESC",
        &[],
    )?
    .iter()
    .map(summary_from_row)
    .collect()
}

/// Reads a setting, `None` if never set.
pub fn get_setting(db: &mut dyn Db, key: &str) -> Result<Option<String>, StoreError> {
    Ok(fetch_one(
        db,
        "SELECT value FROM settings WHERE key = ?",
        &[Value::from(key)],
    )?
    .map(|r| r.get_str("value").map(str::to_string))
    .transpose()?)
}

/// Reads an on/off setting written by [`set_flag`]: off when never set, and
/// [`StoreError::Corrupt`] for any value but `"true"` or `"false"`.
pub fn get_flag(db: &mut dyn Db, key: &str) -> Result<bool, StoreError> {
    match get_setting(db, key)?.as_deref() {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        Some(other) => Err(StoreError::Corrupt(format!(
            "setting {key} is {other:?}, not true or false"
        ))),
    }
}

/// Writes an on/off setting (see [`get_flag`]).
pub fn set_flag(db: &mut dyn Db, key: &str, on: bool) -> Result<(), StoreError> {
    set_setting(db, key, if on { "true" } else { "false" })
}

/// Writes a setting, replacing any earlier value.
pub fn set_setting(db: &mut dyn Db, key: &str, value: &str) -> Result<(), StoreError> {
    transaction(db, |tx| {
        execute(
            tx,
            "INSERT INTO settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            &[Value::from(key), Value::from(value)],
        )?;
        Ok(())
    })?;
    Ok(())
}

/// Removes a setting; removing one that was never set is not an error.
pub fn delete_setting(db: &mut dyn Db, key: &str) -> Result<(), StoreError> {
    transaction(db, |tx| {
        execute(
            tx,
            "DELETE FROM settings WHERE key = ?",
            &[Value::from(key)],
        )?;
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_plain_text};

    /// Identifiers holding only the DOI `d`.
    fn doi(d: &str) -> Identifiers {
        Identifiers {
            doi: Some(d.to_string()),
            ..Default::default()
        }
    }

    /// Identifiers holding only the PMID `p`.
    fn pmid(p: &str) -> Identifiers {
        Identifiers {
            pmid: Some(p.to_string()),
            ..Default::default()
        }
    }

    /// A transaction from `open_store` holds the write lock from its start
    /// (`prepare` sets it, for `open_memory_store` too).
    #[test]
    fn transactions_begin_immediate() {
        let dir = std::env::temp_dir().join(format!("sa-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.sqlite3");
        let mut a = open_store(&path).unwrap();
        let b = open_store(&path).unwrap();
        b.busy_timeout(std::time::Duration::ZERO).unwrap();
        let blocked = transaction_with(&mut a, |_tx| {
            Ok::<_, DbError>(b.execute_batch("BEGIN IMMEDIATE").is_err())
        })
        .unwrap();
        assert!(
            blocked,
            "the second writer should be refused while A is open"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Opening a store also creates bmlib's chunk tables.
    #[test]
    fn chunk_schema_exists() {
        let mut db = open_memory_store().unwrap();
        let row = fetch_one(
            &mut db,
            "SELECT count(*) AS n FROM sqlite_master WHERE name = 'embedded_chunks'",
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(row.get_i64("n").unwrap(), 1);
    }

    /// Opening the same file twice migrates once.
    #[test]
    fn migrations_apply_once() {
        let dir = std::env::temp_dir().join(format!("sa-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.sqlite3");
        drop(open_store(&path).unwrap());
        let mut conn = open_store(&path).unwrap();
        let rows = fetch_all(&mut conn, "SELECT version FROM schema_version", &[]).unwrap();
        assert_eq!(rows.len(), migrations().len());
        drop(conn);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A database a newer app wrote (a schema version past ours) is refused,
    /// and left as it was.
    #[test]
    fn newer_database_is_refused() {
        let dir = std::env::temp_dir().join(format!("sa-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.sqlite3");
        let mut conn = open_store(&path).unwrap();
        let known = migrations().iter().map(|m| m.version).max().unwrap();
        execute(
            &mut conn,
            "INSERT INTO schema_version (version, name) VALUES (?, ?)",
            &[Value::from(known + 1), Value::from("from the future")],
        )
        .unwrap();
        drop(conn);
        match open_store(&path) {
            Err(StoreError::Newer { found, known: k }) => {
                assert_eq!((found, k), (known + 1, known))
            }
            other => panic!("expected Newer, got {other:?}"),
        }
        // Read without the guard: no migration ran over the newer schema.
        let raw = rusqlite::Connection::open(&path).unwrap();
        let rows: i64 = raw
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, migrations().len() as i64 + 1);
        drop(raw);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A database written under migration 1 upgrades: its canonical texts read
    /// back with no failed re-derive, and one can then be recorded; migration
    /// 11 moves each source's bytes to `source_bytes` (a TEXT value hashed as
    /// its UTF-8), backfills its digest, places it in the app, and
    /// `holder_of_source` finds it by that digest.
    #[test]
    fn version_1_database_upgrades() {
        let dir = std::env::temp_dir().join(format!("sa-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.sqlite3");
        let mut old = open_path(&path, true, true).unwrap();
        run_migrations(&mut old, migrations().into_iter().take(1).collect()).unwrap();
        let p = create_paper(&mut old, &pmid("1"), None).unwrap();
        // Migration 1's columns, as an app of that time stored a source.
        execute(
            &mut old,
            "INSERT INTO sources (paper_id, kind, origin, bytes) VALUES (?, 'plain_text', 'paste', ?)",
            &[Value::from(p.0.as_str()), Value::from(b"Para one.".to_vec())],
        )
        .unwrap();
        let sid = last_id(&mut old).unwrap();
        // SQLite let a TEXT value into `bytes BLOB NOT NULL`; migration 11 hashes its UTF-8.
        execute(
            &mut old,
            "INSERT INTO sources (paper_id, kind, origin, bytes) VALUES (?, 'pdf', 'x', 'text')",
            &[Value::from(p.0.as_str())],
        )
        .unwrap();
        let text_id = last_id(&mut old).unwrap();
        let text = canonical_from_plain_text("Para one.", "T");
        let a = assess_full_text(&text).unwrap();
        let cid = save_canonical(&mut old, &p, sid, &text, &a).unwrap();
        drop(old);
        let mut db = open_store(&path).unwrap();
        let c = latest_canonical(&mut db, &p).unwrap().unwrap();
        assert_eq!((c.id, c.rederive_failure), (cid, None));
        assert!(source_notices(&mut db, sid).unwrap().is_empty());
        assert_eq!(refusal(&mut db, &p).unwrap(), None);
        // Migration 11 kept the bytes apart and gave each source its digest.
        let s = source_by_id(&mut db, sid).unwrap();
        assert_eq!(
            (s.sha256, s.place),
            (sha256_hex(b"Para one."), SourcePlace::App)
        );
        assert_eq!(
            source_bytes(&mut db, sid).unwrap(),
            Some(b"Para one.".to_vec())
        );
        let digest = sha256_hex(b"Para one.");
        let held = holder_of_source(&mut db, SourceKind::PlainText, &digest).unwrap();
        assert_eq!(held.map(|h| h.0), Some(p.clone()));
        assert_eq!(
            source_by_id(&mut db, text_id).unwrap().sha256,
            sha256_hex(b"text")
        );
        let failure = RederiveFailure {
            converter_version: "plain-2".into(),
            error: "bad bytes".into(),
        };
        record_rederive_failure(&mut db, cid, &failure).unwrap();
        let c = latest_canonical(&mut db, &p).unwrap().unwrap();
        assert_eq!(c.rederive_failure, Some(failure));
        drop(db);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two connections to one file can each write, and the busy timeout is set.
    #[test]
    fn two_connections_each_write() {
        let dir = std::env::temp_dir().join(format!("sa-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.sqlite3");
        let mut a = open_store(&path).unwrap();
        let mut b = open_store(&path).unwrap();
        let ms: i64 = a
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ms, DB_BUSY_TIMEOUT.as_millis() as i64);
        create_paper(&mut a, &pmid("1"), None).unwrap();
        create_paper(&mut b, &pmid("2"), None).unwrap();
        assert_eq!(recent_papers(&mut a).unwrap().len(), 2);
        drop((a, b));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A paper created by PMID is found by a DOI added later.
    #[test]
    fn same_paper_by_other_identifier() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("123"), None).unwrap();
        add_identifiers(&mut db, &p, &doi("10.1000/x")).unwrap();
        assert_eq!(
            find_paper(&mut db, &doi("10.1000/x")).unwrap(),
            Some(p.clone())
        );
        let pmc = Identifiers {
            pmc_id: Some("PMC9".into()),
            ..Default::default()
        };
        assert_eq!(find_paper(&mut db, &pmc).unwrap(), None);
        assert_eq!(recent_papers(&mut db).unwrap()[0].title, UNTITLED_PAPER);
    }

    /// The title is read back; an unknown paper is `NotFound`.
    #[test]
    fn paper_title_reads_or_not_found() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), Some("A title")).unwrap();
        assert_eq!(paper_title(&mut db, &p).unwrap(), "A title");
        let missing = PaperId("nope".into());
        assert!(matches!(
            paper_title(&mut db, &missing),
            Err(StoreError::NotFound(_))
        ));
    }

    /// A stored identifier is never replaced by a different one.
    #[test]
    fn add_identifiers_never_overwrites() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &doi("10.1000/a"), None).unwrap();
        let notices = add_identifiers(&mut db, &p, &doi("10.1000/b")).unwrap();
        let s = &recent_papers(&mut db).unwrap()[0];
        assert_eq!(s.identifiers.doi.as_deref(), Some("10.1000/a"));
        assert_eq!(
            notices,
            ["DOI 10.1000/b was not recorded: this paper's DOI is 10.1000/a."]
        );
        // The same value again, or a new one, says nothing.
        let mut both = pmid("7");
        both.doi = Some("10.1000/a".into());
        assert!(add_identifiers(&mut db, &p, &both).unwrap().is_empty());
    }

    /// An identifier held by another paper is skipped with a notice, not an error.
    #[test]
    fn add_identifiers_skips_other_papers_identifier() {
        let mut db = open_memory_store().unwrap();
        let _a = create_paper(&mut db, &doi("10.1000/a"), Some("Paper A")).unwrap();
        let b = create_paper(&mut db, &pmid("5"), None).unwrap();
        let notices = add_identifiers(&mut db, &b, &doi("10.1000/a")).unwrap();
        let all = recent_papers(&mut db).unwrap();
        let sb = all.iter().find(|s| s.id == b.0).unwrap();
        assert_eq!(sb.identifiers.doi, None);
        assert_eq!(
            notices,
            ["DOI 10.1000/a was not recorded: it belongs to another stored paper, \u{201c}Paper A\u{201d}."]
        );
    }

    /// A text is found by id for its own paper only; the latest is the newest,
    /// and `latest_canonical_id` names it (none for a paper without text).
    #[test]
    fn canonical_by_id_is_per_paper() {
        let mut db = open_memory_store().unwrap();
        let text_of = |db: &mut Connection, p: &PaperId, body: &str| {
            let src = NewSource {
                kind: SourceKind::PlainText,
                origin: "paste".into(),
                file_name: None,
                bytes: body.as_bytes().to_vec(),
                decoded: None,
            };
            let sid = save_source(db, p, src, SourcePlace::App).unwrap();
            let text = canonical_from_plain_text(body, "T");
            let a = assess_full_text(&text).unwrap();
            save_canonical(db, p, sid, &text, &a).unwrap()
        };
        let a = create_paper(&mut db, &pmid("1"), None).unwrap();
        let b = create_paper(&mut db, &pmid("2"), None).unwrap();
        assert_eq!(latest_canonical_id(&mut db, &a).unwrap(), None);
        let old = text_of(&mut db, &a, "Old.");
        let new = text_of(&mut db, &a, "New.");
        let other = text_of(&mut db, &b, "Other.");
        let got = canonical_by_id(&mut db, &a, old).unwrap().unwrap();
        assert_eq!(got.id, old);
        assert_eq!(got.text, canonical_from_plain_text("Old.", "T"));
        assert_eq!(latest_canonical(&mut db, &a).unwrap().unwrap().id, new);
        assert_eq!(latest_canonical_id(&mut db, &a).unwrap(), Some(new));
        assert_eq!(latest_canonical_id(&mut db, &b).unwrap(), Some(other));
        assert!(canonical_by_id(&mut db, &a, other).unwrap().is_none());
        assert!(canonical_by_id(&mut db, &a, other + 1).unwrap().is_none());
    }

    /// A newer text drops the older texts' chunk sets, and only the paper's.
    #[test]
    fn newer_text_drops_older_chunk_sets() {
        use crate::passages::tests::FakeEmbedder;
        use bmlib::embed::store::index_paper;
        let mut db = open_memory_store().unwrap();
        let text_of = |db: &mut Connection, p: &PaperId, body: &str| {
            let src = NewSource {
                kind: SourceKind::PlainText,
                origin: "paste".into(),
                file_name: None,
                bytes: body.as_bytes().to_vec(),
                decoded: None,
            };
            let sid = save_source(db, p, src, SourcePlace::App).unwrap();
            let text = canonical_from_plain_text(body, "T");
            let a = assess_full_text(&text).unwrap();
            let id = save_canonical(db, p, sid, &text, &a).unwrap();
            index_paper(db, &index_key(id), &text, &mut FakeEmbedder, None).unwrap();
            id
        };
        let sets = |db: &mut Connection, id: i64| {
            fetch_one(
                db,
                "SELECT count(*) AS n FROM embedded_chunk_sets WHERE paper_key = ?",
                &[Value::from(index_key(id))],
            )
            .unwrap()
            .unwrap()
            .get_i64("n")
            .unwrap()
        };
        let a = create_paper(&mut db, &pmid("1"), None).unwrap();
        let b = create_paper(&mut db, &pmid("2"), None).unwrap();
        let other = text_of(&mut db, &b, "Other paper.");
        let old = text_of(&mut db, &a, "Old text.");
        assert_eq!(sets(&mut db, old), 1);
        let new = text_of(&mut db, &a, "New text.");
        assert_eq!(sets(&mut db, old), 0);
        assert_eq!(sets(&mut db, new), 1);
        assert_eq!(sets(&mut db, other), 1);

        // #55: an ask that writes the old text's set after the drop drops it again.
        let old_text = canonical_from_plain_text("Old text.", "T");
        index_paper(&mut db, &index_key(old), &old_text, &mut FakeEmbedder, None).unwrap();
        assert_eq!(sets(&mut db, old), 1);
        assert!(drop_superseded_chunk_set(&mut db, old).unwrap());
        assert_eq!(sets(&mut db, old), 0);
        // The latest texts keep theirs, and an unknown id drops nothing.
        assert!(!drop_superseded_chunk_set(&mut db, new).unwrap());
        assert!(!drop_superseded_chunk_set(&mut db, other).unwrap());
        assert!(!drop_superseded_chunk_set(&mut db, new + 100).unwrap());
        assert_eq!(sets(&mut db, new), 1);
        assert_eq!(sets(&mut db, other), 1);
    }

    /// Source and canonical text survive a round trip; confirmation is stored.
    #[test]
    fn canonical_round_trip() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), Some("T")).unwrap();
        let src = NewSource {
            kind: SourceKind::Markdown,
            origin: "paste".into(),
            file_name: None,
            bytes: b"Para one.\n\nPara two.".to_vec(),
            decoded: Some(Decoded {
                encoding: "UTF-8".into(),
                guessed: true,
            }),
        };
        let sid = save_source(&mut db, &p, src.clone(), SourcePlace::App).unwrap();
        let got = latest_source(&mut db, &p).unwrap().unwrap();
        assert_eq!(
            (got.id, got.kind, source_bytes(&mut db, sid).unwrap()),
            (sid, src.kind, Some(src.bytes.clone()))
        );
        assert_eq!(
            (got.sha256, got.place, got.library_note),
            (sha256_hex(&src.bytes), SourcePlace::App, None)
        );
        assert_eq!(got.decoded, src.decoded);
        let text = canonical_from_plain_text("Para one.\n\nPara two.", "T");
        let a = assess_full_text(&text).unwrap();
        let cid = save_canonical(&mut db, &p, sid, &text, &a).unwrap();
        let c = latest_canonical(&mut db, &p).unwrap().unwrap();
        assert_eq!(c.text, text);
        assert_eq!(c.source_id, sid);
        assert!(!c.confirmed_full_text);
        assert_eq!(c.rederive_failure, None);
        confirm_full_text(&mut db, &p, cid).unwrap();
        let failure = RederiveFailure {
            converter_version: "plain-2".into(),
            error: "bad bytes".into(),
        };
        record_rederive_failure(&mut db, cid, &failure).unwrap();
        let c = latest_canonical(&mut db, &p).unwrap().unwrap();
        assert!(c.confirmed_full_text);
        assert_eq!(c.rederive_failure, Some(failure.clone()));
        assert!(matches!(
            record_rederive_failure(&mut db, 42, &failure),
            Err(StoreError::NotFound(_))
        ));
        // A damaged row (version, no error) still names a cause.
        execute(
            &mut db,
            "UPDATE canonical_texts SET rederive_error = NULL",
            &[],
        )
        .unwrap();
        let c = latest_canonical(&mut db, &p).unwrap().unwrap();
        assert_eq!(
            c.rederive_failure.map(|f| f.error),
            Some("the cause was not recorded".to_string())
        );
    }

    /// A PDF source has no encoding and reads back as `None`.
    #[test]
    fn pdf_source_has_no_decoding() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let src = NewSource {
            kind: SourceKind::Pdf,
            origin: "upload".into(),
            file_name: Some("a.pdf".into()),
            bytes: vec![0, 1, 2],
            decoded: None,
        };
        save_source(&mut db, &p, src, SourcePlace::App).unwrap();
        let stored = latest_source(&mut db, &p).unwrap().unwrap();
        assert_eq!(stored.decoded, None);
        assert_eq!(stored.file_name.as_deref(), Some("a.pdf"));
        assert_eq!(
            source_info(&mut db, stored.id).unwrap(),
            (SourceKind::Pdf, None)
        );
        assert!(matches!(
            source_info(&mut db, stored.id + 1),
            Err(StoreError::NotFound(_))
        ));
        assert!(latest_canonical(&mut db, &p).unwrap().is_none());
    }

    /// The original PDF is the source of the latest text, and only when that is a PDF.
    #[test]
    fn original_pdf_is_the_latest_texts_pdf_source() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        assert_eq!(original_pdf(&mut db, &p).unwrap(), None);
        let text = canonical_from_plain_text("Para one.\n\nPara two.", "T");
        let a = assess_full_text(&text).unwrap();
        let mut keep = |kind, bytes: &[u8], decoded| {
            let src = NewSource {
                kind,
                origin: "upload".into(),
                file_name: None,
                bytes: bytes.to_vec(),
                decoded,
            };
            let sid = save_source(&mut db, &p, src, SourcePlace::App).unwrap();
            save_canonical(&mut db, &p, sid, &text, &a).unwrap();
            original_pdf(&mut db, &p).unwrap()
        };
        assert_eq!(keep(SourceKind::Pdf, b"%PDF", None), Some(b"%PDF".to_vec()));
        let utf8 = Some(Decoded {
            encoding: "UTF-8".into(),
            guessed: false,
        });
        assert_eq!(keep(SourceKind::PlainText, b"Para one.", utf8), None);
    }

    /// A PDF kept beside a JATS source is the Original of a text derived from
    /// that source, once, and of no other text (#739).
    #[test]
    fn a_pdf_beside_the_texts_source_is_its_original() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let text = canonical_from_plain_text("Para one.\n\nPara two.", "T");
        let a = assess_full_text(&text).unwrap();
        let jats = NewSource {
            kind: SourceKind::Jats,
            origin: "europepmc".into(),
            file_name: None,
            bytes: b"<article/>".to_vec(),
            decoded: None,
        };
        let sid = save_source(&mut db, &p, jats, SourcePlace::App).unwrap();
        save_canonical(&mut db, &p, sid, &text, &a).unwrap();
        assert!(!has_original_pdf(&mut db, sid).unwrap());
        assert_eq!(original_pdf(&mut db, &p).unwrap(), None);

        assert!(save_pdf_beside(
            &mut db,
            &p,
            sid,
            "europepmc_pdf",
            b"%PDF-1".to_vec(),
            SourcePlace::App
        )
        .unwrap());
        assert!(!save_pdf_beside(
            &mut db,
            &p,
            sid,
            "unpaywall",
            b"%PDF-2".to_vec(),
            SourcePlace::App
        )
        .unwrap());
        assert!(has_original_pdf(&mut db, sid).unwrap());
        assert_eq!(original_pdf(&mut db, &p).unwrap(), Some(b"%PDF-1".to_vec()));

        // A later text from another source has no Original.
        let plain = NewSource {
            kind: SourceKind::PlainText,
            origin: "upload".into(),
            file_name: None,
            bytes: b"Para one.".to_vec(),
            decoded: Some(Decoded {
                encoding: "UTF-8".into(),
                guessed: false,
            }),
        };
        let other = save_source(&mut db, &p, plain, SourcePlace::App).unwrap();
        save_canonical(&mut db, &p, other, &text, &a).unwrap();
        assert!(!has_original_pdf(&mut db, other).unwrap());
        assert_eq!(original_pdf(&mut db, &p).unwrap(), None);
        assert!(matches!(
            has_original_pdf(&mut db, 9999),
            Err(StoreError::NotFound(_))
        ));
    }

    /// A source is fetched by its row id; an absent id is `NotFound`.
    #[test]
    fn source_by_id_finds_or_reports_missing() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let mk = |b: u8| NewSource {
            kind: SourceKind::Pdf,
            origin: "upload".into(),
            file_name: None,
            bytes: vec![b],
            decoded: None,
        };
        let a = save_source(&mut db, &p, mk(1), SourcePlace::App).unwrap();
        let b = save_source(&mut db, &p, mk(2), SourcePlace::App).unwrap();
        assert_eq!(source_by_id(&mut db, a).unwrap().sha256, sha256_hex(&[1]));
        assert_eq!(source_bytes(&mut db, b).unwrap(), Some(vec![2]));
        assert!(matches!(
            source_by_id(&mut db, b + 100),
            Err(StoreError::NotFound(_))
        ));
    }

    /// An unknown source kind is reported as corrupt.
    #[test]
    fn unknown_kind_is_corrupt() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        execute(
            &mut db,
            "INSERT INTO sources (paper_id, kind, origin, sha256) VALUES (?, 'zip', 'x', '00')",
            &[Value::from(p.0.as_str())],
        )
        .unwrap();
        assert!(matches!(
            latest_source(&mut db, &p),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// Invalid stored canonical JSON is reported, not returned.
    #[test]
    fn corrupt_canonical_is_reported() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let src = NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: b"x".to_vec(),
            decoded: None,
        };
        let sid = save_source(&mut db, &p, src, SourcePlace::App).unwrap();
        let text = canonical_from_plain_text("Para one.", "T");
        let a = assess_full_text(&text).unwrap();
        save_canonical(&mut db, &p, sid, &text, &a).unwrap();
        execute(
            &mut db,
            "UPDATE canonical_texts SET canonical_json = '{}'",
            &[],
        )
        .unwrap();
        assert!(matches!(
            latest_canonical(&mut db, &p),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// All papers come back, newest-opened first. Order is set through explicit
    /// `opened_at` values, so no wall-clock sleeps are needed.
    #[test]
    fn recent_is_complete_and_ordered() {
        let mut db = open_memory_store().unwrap();
        let mut ids = Vec::new();
        for i in 0..30 {
            let p = create_paper(&mut db, &pmid(&(i + 1).to_string()), None).unwrap();
            // Creation order is shuffled against the opened order.
            let rank = (i * 7) % 30;
            execute(
                &mut db,
                "UPDATE papers SET opened_at = ? WHERE id = ?",
                &[
                    Value::from(format!("2026-01-01 00:00:{rank:02}")),
                    Value::from(p.0.as_str()),
                ],
            )
            .unwrap();
            ids.push((rank, p.0));
        }
        ids.sort_by_key(|a| std::cmp::Reverse(a.0));
        let got: Vec<String> = recent_papers(&mut db)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(got, ids.into_iter().map(|x| x.1).collect::<Vec<_>>());
    }

    /// Equal `opened_at` values order by insertion, newest first; touch works.
    #[test]
    fn ties_order_by_rowid_and_touch_updates() {
        let mut db = open_memory_store().unwrap();
        let a = create_paper(&mut db, &pmid("1"), None).unwrap();
        let b = create_paper(&mut db, &pmid("2"), None).unwrap();
        execute(
            &mut db,
            "UPDATE papers SET opened_at = '2026-01-01 00:00:00'",
            &[],
        )
        .unwrap();
        assert_eq!(recent_papers(&mut db).unwrap()[0].id, b.0);
        touch_opened(&mut db, &a).unwrap();
        set_title(&mut db, &a, "New").unwrap();
        let first = &recent_papers(&mut db).unwrap()[0];
        assert_eq!(
            (first.id.as_str(), first.title.as_str()),
            (a.0.as_str(), "New")
        );
    }

    /// Settings round-trip and overwrite; unknown keys are `None`.
    #[test]
    fn settings_round_trip() {
        let mut db = open_memory_store().unwrap();
        assert_eq!(get_setting(&mut db, "contact_email").unwrap(), None);
        set_setting(&mut db, "contact_email", "a@b.c").unwrap();
        assert_eq!(
            get_setting(&mut db, "contact_email").unwrap().as_deref(),
            Some("a@b.c")
        );
        set_setting(&mut db, "contact_email", "d@e.f").unwrap();
        assert_eq!(
            get_setting(&mut db, "contact_email").unwrap().as_deref(),
            Some("d@e.f")
        );
    }

    /// delete_setting removes a value and tolerates an absent one.
    #[test]
    fn delete_setting_removes() {
        let mut db = open_memory_store().unwrap();
        set_setting(&mut db, "k", "v").unwrap();
        delete_setting(&mut db, "k").unwrap();
        assert_eq!(get_setting(&mut db, "k").unwrap(), None);
        delete_setting(&mut db, "k").unwrap();
    }

    /// create_paper refuses an identifier another paper holds.
    #[test]
    fn create_paper_conflict() {
        let mut db = open_memory_store().unwrap();
        create_paper(&mut db, &doi("10.1000/a"), None).unwrap();
        let err = create_paper(&mut db, &doi("10.1000/a"), None).unwrap_err();
        assert!(
            matches!(&err, StoreError::Conflict(m) if m.contains("doi") && m.contains("10.1000/a"))
        );
        assert_eq!(recent_papers(&mut db).unwrap().len(), 1);
    }

    /// Unknown paper ids are NotFound for add_identifiers, set_title and touch_opened.
    #[test]
    fn unknown_paper_is_not_found() {
        let mut db = open_memory_store().unwrap();
        let p = PaperId("nope".into());
        assert!(matches!(
            add_identifiers(&mut db, &p, &doi("10.1000/a")),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            set_title(&mut db, &p, "x"),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            touch_opened(&mut db, &p),
            Err(StoreError::NotFound(_))
        ));
    }

    /// A non-BLOB `bytes` value is Corrupt, not empty bytes (SQLite is dynamically typed).
    #[test]
    fn non_blob_bytes_is_corrupt() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        execute(
            &mut db,
            "INSERT INTO sources (paper_id, kind, origin, sha256) VALUES (?, 'pdf', 'x', '00')",
            &[Value::from(p.0.as_str())],
        )
        .unwrap();
        let id = last_id(&mut db).unwrap();
        execute(
            &mut db,
            "INSERT INTO source_bytes (source_id, bytes) VALUES (?, 'text')",
            &[Value::from(id)],
        )
        .unwrap();
        assert!(matches!(
            source_bytes(&mut db, id),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// An unknown place, or no digest, is corrupt.
    #[test]
    fn unknown_place_or_no_digest_is_corrupt() {
        for (place, sha) in [("elsewhere", Some("00")), ("app", None)] {
            let mut db = open_memory_store().unwrap();
            let p = create_paper(&mut db, &pmid("1"), None).unwrap();
            execute(
                &mut db,
                "INSERT INTO sources (paper_id, kind, origin, sha256, place) VALUES (?, 'pdf', 'x', ?, ?)",
                &[Value::from(p.0.as_str()), Value::from(sha), Value::from(place)],
            )
            .unwrap();
            assert!(matches!(
                latest_source(&mut db, &p),
                Err(StoreError::Corrupt(_))
            ));
        }
        for name in ["app", "waiting", "library"] {
            assert_eq!(SourcePlace::parse(name).unwrap().as_str(), name);
        }
    }

    /// Waiting sources are listed, oldest first; marking one as in the
    /// library drops its bytes and its note, once; a note only lands on a
    /// waiting source.
    #[test]
    fn waiting_sources_move_to_the_library() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let q = create_paper(&mut db, &pmid("2"), None).unwrap();
        let mk = |b: u8| NewSource {
            kind: SourceKind::Pdf,
            origin: "upload".into(),
            file_name: None,
            bytes: vec![b],
            decoded: None,
        };
        let a = save_source(&mut db, &p, mk(1), SourcePlace::Waiting).unwrap();
        let b = save_source(&mut db, &q, mk(2), SourcePlace::Waiting).unwrap();
        save_source(&mut db, &p, mk(3), SourcePlace::App).unwrap();
        let ids = |v: Vec<StoredSource>| v.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(ids(waiting_sources(&mut db, None).unwrap()), vec![a, b]);
        assert_eq!(ids(waiting_sources(&mut db, Some(&q)).unwrap()), vec![b]);
        note_waiting(&mut db, a, "unplugged").unwrap();
        assert_eq!(
            source_by_id(&mut db, a).unwrap().library_note.as_deref(),
            Some("unplugged")
        );
        assert!(mark_in_library(&mut db, a).unwrap());
        assert!(!mark_in_library(&mut db, a).unwrap());
        let s = source_by_id(&mut db, a).unwrap();
        assert_eq!((s.place, s.library_note), (SourcePlace::Library, None));
        assert_eq!(source_bytes(&mut db, a).unwrap(), None);
        note_waiting(&mut db, a, "late").unwrap();
        assert_eq!(source_by_id(&mut db, a).unwrap().library_note, None);
        assert_eq!(ids(waiting_sources(&mut db, None).unwrap()), vec![b]);
    }

    /// Rows are counted by place; every app row can wait, and every waiting
    /// row can go back to the app, its note cleared.
    #[test]
    fn place_counts_and_moves() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let q = create_paper(&mut db, &pmid("2"), None).unwrap();
        let mk = |b: u8| NewSource {
            kind: SourceKind::Pdf,
            origin: "upload".into(),
            file_name: None,
            bytes: vec![b],
            decoded: None,
        };
        save_source(&mut db, &p, mk(1), SourcePlace::App).unwrap();
        save_source(&mut db, &q, mk(2), SourcePlace::App).unwrap();
        let w = save_source(&mut db, &p, mk(3), SourcePlace::Waiting).unwrap();
        note_waiting(&mut db, w, "unplugged").unwrap();
        let counts = |db: &mut dyn Db| count_by_place(db).unwrap();
        assert_eq!(
            counts(&mut db),
            PlaceCounts {
                app: 2,
                waiting: 1,
                library: 0
            }
        );
        assert_eq!(
            papers_with_place(&mut db, SourcePlace::App).unwrap(),
            vec![p.clone(), q.clone()]
        );
        assert_eq!(app_to_waiting(&mut db).unwrap(), 2);
        assert_eq!(counts(&mut db).waiting, 3);
        assert_eq!(
            sources_with_place(&mut db, Some(&p), SourcePlace::Waiting)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(waiting_to_app(&mut db).unwrap(), 3);
        assert_eq!(counts(&mut db).app, 3);
        assert_eq!(source_by_id(&mut db, w).unwrap().library_note, None);
    }

    /// A source kept in the library gets its bytes back here, once, as
    /// waiting or app; bytes that are not its digest's are refused.
    #[test]
    fn bring_back_only_from_library() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let mk = |b: u8| NewSource {
            kind: SourceKind::Pdf,
            origin: "upload".into(),
            file_name: None,
            bytes: vec![b],
            decoded: None,
        };
        let a = save_source(&mut db, &p, mk(1), SourcePlace::Waiting).unwrap();
        let app = save_source(&mut db, &p, mk(2), SourcePlace::App).unwrap();
        assert!(mark_in_library(&mut db, a).unwrap());
        note_library(&mut db, a, "left in /old").unwrap();
        assert!(matches!(
            bring_back(&mut db, a, &[9], SourcePlace::App),
            Err(StoreError::Corrupt(_))
        ));
        assert!(bring_back(&mut db, a, &[1], SourcePlace::App).unwrap());
        let got = source_by_id(&mut db, a).unwrap();
        assert_eq!((got.place, got.library_note), (SourcePlace::App, None));
        assert_eq!(source_bytes(&mut db, a).unwrap(), Some(vec![1]));
        assert!(!bring_back(&mut db, a, &[1], SourcePlace::Waiting).unwrap());
        assert!(!bring_back(&mut db, app, &[2], SourcePlace::Waiting).unwrap());
    }

    /// Library sources take a note of where their file was left: one at a
    /// time, or every one without a note; others are not touched.
    #[test]
    fn library_sources_take_a_note() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let mk = |b: u8| NewSource {
            kind: SourceKind::Pdf,
            origin: "upload".into(),
            file_name: None,
            bytes: vec![b],
            decoded: None,
        };
        let [a, b, w] =
            [1, 2, 3].map(|n| save_source(&mut db, &p, mk(n), SourcePlace::Waiting).unwrap());
        assert!(mark_in_library(&mut db, a).unwrap());
        assert!(mark_in_library(&mut db, b).unwrap());
        note_library(&mut db, a, "in A").unwrap();
        note_library(&mut db, w, "not kept").unwrap();
        assert_eq!(note_left_behind(&mut db, "in B").unwrap(), 1);
        let note = |db: &mut Connection, id| source_by_id(db, id).unwrap().library_note;
        assert_eq!(note(&mut db, a).as_deref(), Some("in A"));
        assert_eq!(note(&mut db, b).as_deref(), Some("in B"));
        assert_eq!(note(&mut db, w), None);
    }

    /// A stored paper is found by id; an unknown id is `None`.
    #[test]
    fn paper_by_id_finds_stored_only() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        assert_eq!(paper_by_id(&mut db, &p.0).unwrap(), Some(p));
        assert_eq!(paper_by_id(&mut db, "nope").unwrap(), None);
    }

    /// A source is found by its kind and exact bytes only, and the paper that
    /// stored it last wins, with that source's file name.
    #[test]
    fn holder_of_source_matches_kind_and_bytes() {
        let mut db = open_memory_store().unwrap();
        let src = |name: Option<&str>| NewSource {
            kind: SourceKind::PlainText,
            origin: "upload".into(),
            file_name: name.map(str::to_string),
            bytes: b"same".to_vec(),
            decoded: None,
        };
        let a = create_paper(&mut db, &Identifiers::default(), None).unwrap();
        let b = create_paper(&mut db, &Identifiers::default(), None).unwrap();
        save_source(&mut db, &a, src(None), SourcePlace::App).unwrap();
        save_source(&mut db, &b, src(Some("b.txt")), SourcePlace::App).unwrap();
        for (kind, bytes, want) in [
            (
                SourceKind::PlainText,
                &b"same"[..],
                Some((b.clone(), Some("b.txt".to_string()))),
            ),
            (SourceKind::Markdown, b"same", None),
            (SourceKind::PlainText, b"sane", None),
            (SourceKind::PlainText, b"same ", None),
        ] {
            assert_eq!(
                holder_of_source(&mut db, kind, &sha256_hex(bytes)).unwrap(),
                want
            );
        }
    }

    /// Confirming a canonical text that does not exist, or one of another
    /// paper, is `NotFound`, not a silent success; confirming an older text
    /// leaves a newer one unconfirmed.
    #[test]
    fn confirm_unknown_canonical_is_not_found() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let other = create_paper(&mut db, &pmid("2"), None).unwrap();
        assert!(matches!(
            confirm_full_text(&mut db, &p, 42),
            Err(StoreError::NotFound(_))
        ));
        let src = NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: b"Para one.".to_vec(),
            decoded: None,
        };
        let sid = save_source(&mut db, &p, src, SourcePlace::App).unwrap();
        let text = canonical_from_plain_text("Para one.", "T");
        let cid =
            save_canonical(&mut db, &p, sid, &text, &assess_full_text(&text).unwrap()).unwrap();
        assert!(matches!(
            confirm_full_text(&mut db, &other, cid),
            Err(StoreError::NotFound(_))
        ));
        assert!(
            !latest_canonical(&mut db, &p)
                .unwrap()
                .unwrap()
                .confirmed_full_text
        );
        // The text seen is confirmed, not a newer one a re-derive saved meanwhile.
        let newer =
            save_canonical(&mut db, &p, sid, &text, &assess_full_text(&text).unwrap()).unwrap();
        assert_ne!(newer, cid);
        confirm_full_text(&mut db, &p, cid).unwrap();
        assert!(
            !latest_canonical(&mut db, &p)
                .unwrap()
                .unwrap()
                .confirmed_full_text
        );
    }

    /// Agreeing and new identifiers are fine; a contradicting one, or one
    /// another paper holds, is named.
    #[test]
    fn identifier_conflict_names_the_mismatch() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        create_paper(&mut db, &doi("10.1000/b"), Some("Paper B")).unwrap();
        assert_eq!(identifier_conflict(&mut db, &p, &pmid("1")).unwrap(), None);
        assert_eq!(
            identifier_conflict(&mut db, &p, &doi("10.1000/new")).unwrap(),
            None
        );
        let other = identifier_conflict(&mut db, &p, &pmid("2"))
            .unwrap()
            .unwrap();
        assert_eq!(other, "its PMID is 2, but this paper's PMID is 1");
        let held = identifier_conflict(&mut db, &p, &doi("10.1000/b"))
            .unwrap()
            .unwrap();
        assert_eq!(
            held,
            "its DOI 10.1000/b belongs to another stored paper, \u{201c}Paper B\u{201d}"
        );
        let gone = PaperId("nope".into());
        assert!(matches!(
            identifier_conflict(&mut db, &gone, &pmid("1")),
            Err(StoreError::NotFound(_))
        ));
    }

    /// Only a value differing from the paper's stored one contradicts it; an
    /// identifier the paper lacks, even one another paper holds, does not.
    #[test]
    fn contradiction_is_a_differing_stored_value() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        create_paper(&mut db, &doi("10.1000/b"), None).unwrap();
        assert_eq!(contradiction(&mut db, &p, &pmid("1")).unwrap(), None);
        assert_eq!(contradiction(&mut db, &p, &doi("10.1000/b")).unwrap(), None);
        let both = Identifiers {
            pmid: Some("2".into()),
            doi: Some("10.1000/b".into()),
            pmc_id: None,
        };
        assert_eq!(
            contradiction(&mut db, &p, &both).unwrap().as_deref(),
            Some("its PMID is 2, but this paper's PMID is 1")
        );
    }

    /// A source's notices are stored and read back; a source without any reads empty.
    #[test]
    fn source_notices_round_trip() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        let src = NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: b"Para one.".to_vec(),
            decoded: None,
        };
        let sid = save_source(&mut db, &p, src, SourcePlace::App).unwrap();
        assert!(source_notices(&mut db, sid).unwrap().is_empty());
        let notices = vec!["one".to_string(), "two".to_string()];
        record_source_notices(&mut db, sid, &notices).unwrap();
        assert_eq!(source_notices(&mut db, sid).unwrap(), notices);
        assert!(matches!(
            record_source_notices(&mut db, 42, &notices),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            source_notices(&mut db, 42),
            Err(StoreError::NotFound(_))
        ));
        execute(&mut db, "UPDATE sources SET notices_json = 'x'", &[]).unwrap();
        assert!(matches!(
            source_notices(&mut db, sid),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// A refusal is stored, replaced and cleared; an unknown paper is NotFound.
    #[test]
    fn refusal_round_trip() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        assert_eq!(refusal(&mut db, &p).unwrap(), None);
        set_refusal(&mut db, &p, Some("not this one")).unwrap();
        assert_eq!(
            refusal(&mut db, &p).unwrap().as_deref(),
            Some("not this one")
        );
        set_refusal(&mut db, &p, None).unwrap();
        assert_eq!(refusal(&mut db, &p).unwrap(), None);
        let gone = PaperId("nope".into());
        assert!(matches!(
            set_refusal(&mut db, &gone, None),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            refusal(&mut db, &gone),
            Err(StoreError::NotFound(_))
        ));
    }

    /// Every paper holding one of the identifiers, once each, in lookup order.
    #[test]
    fn holders_lists_each_paper_once() {
        let mut db = open_memory_store().unwrap();
        let a = create_paper(&mut db, &pmid("1"), None).unwrap();
        let b = create_paper(&mut db, &doi("10.1000/b"), None).unwrap();
        let ids = Identifiers {
            pmid: Some("1".into()),
            doi: Some("10.1000/b".into()),
            pmc_id: None,
        };
        assert_eq!(holders(&mut db, &ids).unwrap(), [a.clone(), b]);
        assert_eq!(holders(&mut db, &pmid("1")).unwrap(), [a]);
        assert!(holders(&mut db, &pmid("2")).unwrap().is_empty());
    }

    /// A flag is off until set, and reads back what was set.
    #[test]
    fn flag_round_trip() {
        let mut db = open_memory_store().unwrap();
        assert!(!get_flag(&mut db, "f").unwrap());
        set_flag(&mut db, "f", true).unwrap();
        assert!(get_flag(&mut db, "f").unwrap());
        set_flag(&mut db, "f", false).unwrap();
        assert!(!get_flag(&mut db, "f").unwrap());
        set_setting(&mut db, "f", "1").unwrap();
        assert!(matches!(
            get_flag(&mut db, "f"),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// PMID is matched before DOI, so a clash resolves to the PMID's paper.
    #[test]
    fn find_paper_prefers_pmid() {
        let mut db = open_memory_store().unwrap();
        let a = create_paper(&mut db, &pmid("1"), None).unwrap();
        create_paper(&mut db, &doi("10.1000/b"), None).unwrap();
        let both = Identifiers {
            pmid: Some("1".into()),
            doi: Some("10.1000/b".into()),
            pmc_id: None,
        };
        assert_eq!(find_paper(&mut db, &both).unwrap(), Some(a));
    }
}
