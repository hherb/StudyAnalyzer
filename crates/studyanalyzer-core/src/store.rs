//! SQLite storage for papers, their original sources and canonical texts.

use std::fmt;
use std::path::Path;

use bmlib::db::{
    execute, fetch_all, fetch_one, open_memory, open_path, run_migrations, transaction,
    transaction_with, Db, DbError, Row, Value,
};
use bmlib::fulltext::canonical::{CanonicalText, FullTextAssessment};
use rusqlite::Connection;
use serde::Serialize;

use crate::config::{DB_BUSY_TIMEOUT, UNTITLED_PAPER};
use crate::identifiers::Identifiers;
use crate::intake::{Decoded, SourceKind};
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
}

impl fmt::Display for StoreError {
    /// Renders the error for logs and for the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Db(e) => write!(f, "{e}"),
            StoreError::Corrupt(m) => write!(f, "stored data is corrupt: {m}"),
            StoreError::NotFound(m) => write!(f, "not found: {m}"),
            StoreError::Conflict(m) => write!(f, "conflict: {m}"),
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

/// The id of a stored paper (uuid v4 text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperId(pub String);

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

/// A stored source.
#[derive(Debug, Clone)]
pub struct StoredSource {
    /// Row id.
    pub id: i64,
    /// What kind of source it is.
    pub kind: SourceKind,
    /// The file name, when there was one (the title hint for re-deriving).
    pub file_name: Option<String>,
    /// The original bytes.
    pub bytes: Vec<u8>,
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

/// Opens (creating if needed) the database file with WAL and foreign keys, and migrates it.
///
/// Transactions begin `IMMEDIATE`: a deferred transaction that reads and then
/// writes fails at once with `SQLITE_BUSY` if another connection wrote in
/// between, ignoring the busy timeout; taking the write lock at begin makes
/// the transaction wait for it instead.
pub fn open_store(path: &Path) -> Result<Connection, StoreError> {
    let mut conn = open_path(path, true, true)?;
    conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
    conn.busy_timeout(DB_BUSY_TIMEOUT)
        .map_err(|e| StoreError::Db(DbError::Backend(e.to_string())))?;
    run_migrations(&mut conn, migrations())?;
    Ok(conn)
}

/// Opens a fresh in-memory database, migrated. For tests.
pub fn open_memory_store() -> Result<Connection, StoreError> {
    let mut conn = open_memory()?;
    run_migrations(&mut conn, migrations())?;
    Ok(conn)
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

/// Stores an original source for the paper and returns its row id.
/// Takes the source by value so its bytes, up to the size cap, are moved into
/// the insert rather than copied.
pub fn save_source(db: &mut dyn Db, paper: &PaperId, src: NewSource) -> Result<i64, StoreError> {
    let (enc, guessed) = match src.decoded {
        Some(d) => (Some(d.encoding), d.guessed),
        None => (None, false),
    };
    execute(
        db,
        "INSERT INTO sources (paper_id, kind, origin, file_name, bytes, encoding, encoding_guessed)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::from(paper.0.as_str()),
            Value::from(src.kind.as_str()),
            Value::from(src.origin.as_str()),
            Value::from(src.file_name),
            Value::from(src.bytes),
            Value::from(enc),
            Value::from(guessed),
        ],
    )?;
    last_id(db)
}

/// The row id of the insert just made on this connection.
fn last_id(db: &mut dyn Db) -> Result<i64, StoreError> {
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

/// How the stored source with this row id was decoded (`None` for a PDF),
/// without loading its bytes.
pub fn source_decoding(db: &mut dyn Db, id: i64) -> Result<Option<Decoded>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT encoding, encoding_guessed FROM sources WHERE id = ?",
        &[Value::from(id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("source {id}")))?;
    decoded_from_row(&row)
}

/// The columns every source query reads.
const SOURCE_SELECT: &str =
    "SELECT id, kind, file_name, bytes, encoding, encoding_guessed FROM sources";

/// The `encoding` and `encoding_guessed` columns of a `sources` row.
fn decoded_from_row(row: &Row) -> Result<Option<Decoded>, StoreError> {
    let guessed = row.get_i64("encoding_guessed")? != 0;
    Ok(row.get("encoding")?.as_str().map(|e| Decoded {
        encoding: e.to_string(),
        guessed,
    }))
}

/// Decodes a `sources` row selected with [`SOURCE_SELECT`].
fn source_from_row(row: &Row) -> Result<StoredSource, StoreError> {
    let kind_text = row.get_str("kind")?;
    let kind = SourceKind::parse(kind_text)
        .ok_or_else(|| StoreError::Corrupt(format!("unknown source kind {kind_text:?}")))?;
    let decoded = decoded_from_row(row)?;
    let bytes = row
        .get("bytes")?
        .as_blob()
        .ok_or_else(|| StoreError::Corrupt("source bytes are not a BLOB".to_string()))?
        .to_vec();
    Ok(StoredSource {
        id: row.get_i64("id")?,
        kind,
        file_name: row.get("file_name")?.as_str().map(str::to_string),
        bytes,
        decoded,
    })
}

/// Stores a canonical text and its assessment, derived from `source_id`.
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
    last_id(db)
}

/// The paper's most recently stored canonical text, validated on the way out.
pub fn latest_canonical(
    db: &mut dyn Db,
    paper: &PaperId,
) -> Result<Option<StoredCanonical>, StoreError> {
    let Some(row) = fetch_one(
        db,
        "SELECT id, source_id, canonical_json, assessment_json, confirmed_full_text,
                rederive_failed_version, rederive_error
         FROM canonical_texts WHERE paper_id = ? ORDER BY id DESC LIMIT 1",
        &[Value::from(paper.0.as_str())],
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

/// Records that the user confirmed this canonical text is the whole paper;
/// [`StoreError::NotFound`] when no canonical text has this id.
pub fn confirm_full_text(db: &mut dyn Db, canonical_id: i64) -> Result<(), StoreError> {
    let n = execute(
        db,
        "UPDATE canonical_texts SET confirmed_full_text = 1 WHERE id = ?",
        &[Value::from(canonical_id)],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!(
            "canonical text {canonical_id}"
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

/// Records why the document the paper's last lookup fetched was not stored
/// on it (`None` clears it); [`StoreError::NotFound`] when no such paper exists.
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

#[cfg(test)]
mod tests {
    use super::*;
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_plain_text};

    fn doi(d: &str) -> Identifiers {
        Identifiers {
            doi: Some(d.to_string()),
            ..Default::default()
        }
    }

    fn pmid(p: &str) -> Identifiers {
        Identifiers {
            pmid: Some(p.to_string()),
            ..Default::default()
        }
    }

    /// A transaction from `open_store` holds the write lock from its start.
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

    /// A database written under migration 1 upgrades: its canonical texts read
    /// back with no failed re-derive, and one can then be recorded.
    #[test]
    fn version_1_database_upgrades() {
        let dir = std::env::temp_dir().join(format!("sa-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.sqlite3");
        let mut old = open_path(&path, true, true).unwrap();
        run_migrations(&mut old, migrations().into_iter().take(1).collect()).unwrap();
        let p = create_paper(&mut old, &pmid("1"), None).unwrap();
        let src = NewSource {
            kind: SourceKind::PlainText,
            origin: "paste".into(),
            file_name: None,
            bytes: b"Para one.".to_vec(),
            decoded: None,
        };
        let sid = save_source(&mut old, &p, src).unwrap();
        let text = canonical_from_plain_text("Para one.", "T");
        let a = assess_full_text(&text).unwrap();
        let cid = save_canonical(&mut old, &p, sid, &text, &a).unwrap();
        drop(old);
        let mut db = open_store(&path).unwrap();
        let c = latest_canonical(&mut db, &p).unwrap().unwrap();
        assert_eq!((c.id, c.rederive_failure), (cid, None));
        assert!(source_notices(&mut db, sid).unwrap().is_empty());
        assert_eq!(refusal(&mut db, &p).unwrap(), None);
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
        let sid = save_source(&mut db, &p, src.clone()).unwrap();
        let got = latest_source(&mut db, &p).unwrap().unwrap();
        assert_eq!(
            (got.id, got.kind, got.bytes),
            (sid, src.kind, src.bytes.clone())
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
        confirm_full_text(&mut db, cid).unwrap();
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
        save_source(&mut db, &p, src).unwrap();
        let stored = latest_source(&mut db, &p).unwrap().unwrap();
        assert_eq!(stored.decoded, None);
        assert_eq!(stored.file_name.as_deref(), Some("a.pdf"));
        assert_eq!(source_decoding(&mut db, stored.id).unwrap(), None);
        assert!(matches!(
            source_decoding(&mut db, stored.id + 1),
            Err(StoreError::NotFound(_))
        ));
        assert!(latest_canonical(&mut db, &p).unwrap().is_none());
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
        let a = save_source(&mut db, &p, mk(1)).unwrap();
        let b = save_source(&mut db, &p, mk(2)).unwrap();
        assert_eq!(source_by_id(&mut db, a).unwrap().bytes, vec![1]);
        assert_eq!(source_by_id(&mut db, b).unwrap().bytes, vec![2]);
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
            "INSERT INTO sources (paper_id, kind, origin, bytes) VALUES (?, 'zip', 'x', x'00')",
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
        let sid = save_source(&mut db, &p, src).unwrap();
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
            "INSERT INTO sources (paper_id, kind, origin, bytes) VALUES (?, 'pdf', 'x', 'text')",
            &[Value::from(p.0.as_str())],
        )
        .unwrap();
        assert!(matches!(
            latest_source(&mut db, &p),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// A stored paper is found by id; an unknown id is `None`.
    #[test]
    fn paper_by_id_finds_stored_only() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &pmid("1"), None).unwrap();
        assert_eq!(paper_by_id(&mut db, &p.0).unwrap(), Some(p));
        assert_eq!(paper_by_id(&mut db, "nope").unwrap(), None);
    }

    /// Confirming a canonical text that does not exist is `NotFound`, not a silent success.
    #[test]
    fn confirm_unknown_canonical_is_not_found() {
        let mut db = open_memory_store().unwrap();
        assert!(matches!(
            confirm_full_text(&mut db, 42),
            Err(StoreError::NotFound(_))
        ));
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
        let sid = save_source(&mut db, &p, src).unwrap();
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
