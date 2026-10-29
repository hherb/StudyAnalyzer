//! SQLite storage for papers, their original sources and canonical texts.

use std::fmt;
use std::path::Path;

use bmlib::db::{
    execute, fetch_all, fetch_one, open_memory, open_path, run_migrations, transaction, Db,
    DbError, Row, Value,
};
use bmlib::fulltext::canonical::{CanonicalText, FullTextAssessment};
use rusqlite::Connection;
use serde::Serialize;

use crate::config::UNTITLED_PAPER;
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
}

impl fmt::Display for StoreError {
    /// Renders the error for logs and for the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Db(e) => write!(f, "{e}"),
            StoreError::Corrupt(m) => write!(f, "stored data is corrupt: {m}"),
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
}

/// Opens (creating if needed) the database file with WAL and foreign keys, and migrates it.
pub fn open_store(path: &Path) -> Result<Connection, StoreError> {
    let mut conn = open_path(path, true, true)?;
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

/// Finds the paper holding any of the identifiers: PMID first, then DOI, then PMC ID.
pub fn find_paper(db: &mut dyn Db, ids: &Identifiers) -> Result<Option<PaperId>, StoreError> {
    for (col, val) in id_pairs(ids) {
        if let Some(v) = val {
            let sql = format!("SELECT id FROM papers WHERE {col} = ?");
            if let Some(row) = fetch_one(db, &sql, &[Value::from(v.as_str())])? {
                return Ok(Some(PaperId(row.get_str("id")?.to_string())));
            }
        }
    }
    Ok(None)
}

/// Creates a paper with the given identifiers; the title defaults to `Untitled paper`.
pub fn create_paper(
    db: &mut dyn Db,
    ids: &Identifiers,
    title: Option<&str>,
) -> Result<PaperId, StoreError> {
    let id = uuid::Uuid::new_v4().to_string();
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

/// Fills the paper's absent identifiers. A different stored value is kept, and an
/// identifier another paper already holds is skipped; both are logged.
pub fn add_identifiers(
    db: &mut dyn Db,
    paper: &PaperId,
    ids: &Identifiers,
) -> Result<(), StoreError> {
    let row = fetch_one(
        db,
        "SELECT pmid, doi, pmc_id FROM papers WHERE id = ?",
        &[Value::from(paper.0.as_str())],
    )?
    .ok_or_else(|| StoreError::Corrupt(format!("no paper {}", paper.0)))?;
    for (col, val) in id_pairs(ids) {
        let Some(v) = val else { continue };
        match row.get(col)?.as_str() {
            Some(stored) if stored == v => {}
            Some(stored) => log::warn!(
                "paper {}: keeping stored {col} {stored}, not overwriting with {v}",
                paper.0
            ),
            None => {
                let taken = fetch_one(
                    db,
                    &format!("SELECT id FROM papers WHERE {col} = ?"),
                    &[Value::from(v.as_str())],
                )?;
                if taken.is_some() {
                    log::warn!(
                        "paper {}: {col} {v} already belongs to another paper, skipped",
                        paper.0
                    );
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
    Ok(())
}

/// Sets the paper's title.
pub fn set_title(db: &mut dyn Db, paper: &PaperId, title: &str) -> Result<(), StoreError> {
    execute(
        db,
        "UPDATE papers SET title = ? WHERE id = ?",
        &[Value::from(title), Value::from(paper.0.as_str())],
    )?;
    Ok(())
}

/// Marks the paper as opened now.
pub fn touch_opened(db: &mut dyn Db, paper: &PaperId) -> Result<(), StoreError> {
    execute(
        db,
        "UPDATE papers SET opened_at = datetime('now') WHERE id = ?",
        &[Value::from(paper.0.as_str())],
    )?;
    Ok(())
}

/// Stores an original source for the paper and returns its row id.
pub fn save_source(db: &mut dyn Db, paper: &PaperId, src: &NewSource) -> Result<i64, StoreError> {
    let (enc, guessed) = match &src.decoded {
        Some(d) => (Some(d.encoding.clone()), d.guessed),
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
            Value::from(src.file_name.clone()),
            Value::from(src.bytes.clone()),
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
    let Some(row) = fetch_one(
        db,
        "SELECT id, kind, bytes, encoding, encoding_guessed FROM sources
         WHERE paper_id = ? ORDER BY id DESC LIMIT 1",
        &[Value::from(paper.0.as_str())],
    )?
    else {
        return Ok(None);
    };
    let kind_text = row.get_str("kind")?;
    let kind = SourceKind::parse(kind_text)
        .ok_or_else(|| StoreError::Corrupt(format!("unknown source kind {kind_text:?}")))?;
    let decoded = row.get("encoding")?.as_str().map(|e| Decoded {
        encoding: e.to_string(),
        guessed: row.get("encoding_guessed").ok().and_then(Value::as_i64) == Some(1),
    });
    Ok(Some(StoredSource {
        id: row.get_i64("id")?,
        kind,
        bytes: row.get("bytes")?.as_blob().unwrap_or_default().to_vec(),
        decoded,
    }))
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
        "SELECT id, source_id, canonical_json, assessment_json, confirmed_full_text
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
    }))
}

/// Records that the user confirmed this canonical text is the whole paper.
pub fn confirm_full_text(db: &mut dyn Db, canonical_id: i64) -> Result<(), StoreError> {
    execute(
        db,
        "UPDATE canonical_texts SET confirmed_full_text = 1 WHERE id = ?",
        &[Value::from(canonical_id)],
    )?;
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

    /// Opening the same file twice migrates once.
    #[test]
    fn migrations_apply_once() {
        let dir = std::env::temp_dir().join(format!("sa-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("t.sqlite3");
        drop(open_store(&path).unwrap());
        let mut conn = open_store(&path).unwrap();
        let rows = fetch_all(&mut conn, "SELECT version FROM schema_version", &[]).unwrap();
        assert_eq!(rows.len(), 1);
        drop(conn);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A paper created by PMID is found by a DOI added later (Review Focus 2).
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

    /// A stored identifier is never replaced by a different one.
    #[test]
    fn add_identifiers_never_overwrites() {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &doi("10.1000/a"), None).unwrap();
        add_identifiers(&mut db, &p, &doi("10.1000/b")).unwrap();
        let s = &recent_papers(&mut db).unwrap()[0];
        assert_eq!(s.identifiers.doi.as_deref(), Some("10.1000/a"));
    }

    /// An identifier held by another paper is skipped, not an error.
    #[test]
    fn add_identifiers_skips_other_papers_identifier() {
        let mut db = open_memory_store().unwrap();
        let _a = create_paper(&mut db, &doi("10.1000/a"), None).unwrap();
        let b = create_paper(&mut db, &pmid("5"), None).unwrap();
        add_identifiers(&mut db, &b, &doi("10.1000/a")).unwrap();
        let all = recent_papers(&mut db).unwrap();
        let sb = all.iter().find(|s| s.id == b.0).unwrap();
        assert_eq!(sb.identifiers.doi, None);
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
        let sid = save_source(&mut db, &p, &src).unwrap();
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
        confirm_full_text(&mut db, cid).unwrap();
        assert!(
            latest_canonical(&mut db, &p)
                .unwrap()
                .unwrap()
                .confirmed_full_text
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
        save_source(&mut db, &p, &src).unwrap();
        assert_eq!(latest_source(&mut db, &p).unwrap().unwrap().decoded, None);
        assert!(latest_canonical(&mut db, &p).unwrap().is_none());
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
        let sid = save_source(&mut db, &p, &src).unwrap();
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
}
