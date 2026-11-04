//! Received hand-offs, kept in SQLite from the moment they arrive until the
//! user opens or discards them, so nothing a sender handed over lives only in
//! memory.

use bmlib::db::{execute, fetch_all, fetch_one, Db, Row, Value};
use serde::Serialize;

use crate::handoff::{HandedDocument, Handoff};
use crate::identifiers::Identifiers;
use crate::intake::SourceKind;
use crate::store::{last_id, StoreError};

/// One line of the list of waiting hand-offs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HandoffSummary {
    /// The hand-off's row id.
    pub id: i64,
    /// When it arrived, as SQLite writes it (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub received_at: String,
    /// The sending app, if the hand-off named one.
    pub sender: Option<String>,
    /// The title the sender had, if any.
    pub title: Option<String>,
    /// The identifiers that passed their checks.
    pub identifiers: Identifiers,
    /// True when the hand-off carried the full text.
    pub has_full_text: bool,
    /// Why the hand-off was refused, if it was (such a row has nothing else).
    pub refusal: Option<String>,
}

/// Stores `h` and returns its row id.
pub fn save_handoff(db: &mut dyn Db, h: &Handoff) -> Result<i64, StoreError> {
    let notices = serde_json::to_string(&h.notices)
        .map_err(|e| StoreError::Corrupt(format!("notices cannot be stored: {e}")))?;
    let doc = h.document.as_ref();
    execute(
        db,
        "INSERT INTO handoffs
            (sender, title, pmid, doi, pmc_id, notices_json, kind, file_name, source, bytes)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::from(h.sender.clone()),
            Value::from(h.title.clone()),
            Value::from(h.identifiers.pmid.clone()),
            Value::from(h.identifiers.doi.clone()),
            Value::from(h.identifiers.pmc_id.clone()),
            Value::from(notices),
            Value::from(doc.map(|d| d.kind.as_str())),
            Value::from(doc.and_then(|d| d.file_name.clone())),
            Value::from(doc.and_then(|d| d.source.clone())),
            Value::from(doc.map(|d| d.bytes.clone())),
        ],
    )?;
    last_id(db)
}

/// Stores a hand-off that was refused, with the reason, so the user sees
/// that something arrived and why it was not taken. Returns its row id.
pub fn save_refused_handoff(db: &mut dyn Db, refusal: &str) -> Result<i64, StoreError> {
    execute(
        db,
        "INSERT INTO handoffs (refusal) VALUES (?)",
        &[Value::from(refusal)],
    )?;
    last_id(db)
}

/// Every waiting hand-off, newest first.
pub fn pending_handoffs(db: &mut dyn Db) -> Result<Vec<HandoffSummary>, StoreError> {
    fetch_all(
        db,
        "SELECT id, received_at, sender, title, pmid, doi, pmc_id,
                (bytes IS NOT NULL) AS has_full_text, refusal
         FROM handoffs ORDER BY id DESC",
        &[],
    )?
    .iter()
    .map(summary_from_row)
    .collect()
}

/// The hand-off with this id, bytes included. `None` for an unknown id or a
/// refusal row; a row with a kind but no bytes (or the reverse) is corrupt.
pub fn load_handoff(db: &mut dyn Db, id: i64) -> Result<Option<Handoff>, StoreError> {
    let Some(row) = fetch_one(
        db,
        "SELECT sender, title, pmid, doi, pmc_id, notices_json, kind, file_name, source,
                bytes, refusal
         FROM handoffs WHERE id = ?",
        &[Value::from(id)],
    )?
    else {
        return Ok(None);
    };
    if row.get("refusal")?.as_str().is_some() {
        return Ok(None);
    }
    let notices: Vec<String> = serde_json::from_str(row.get_str("notices_json")?)
        .map_err(|e| StoreError::Corrupt(format!("hand-off notices are not a list: {e}")))?;
    let document = match (row.get("kind")?.as_str(), row.get("bytes")?.as_blob()) {
        (None, None) => None,
        (Some(kind), Some(bytes)) => Some(HandedDocument {
            kind: SourceKind::parse(kind)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown source kind {kind:?}")))?,
            file_name: opt_string(&row, "file_name")?,
            source: opt_string(&row, "source")?,
            bytes: bytes.to_vec(),
        }),
        _ => {
            return Err(StoreError::Corrupt(
                "a hand-off has a kind or bytes but not both".to_string(),
            ))
        }
    };
    Ok(Some(Handoff {
        sender: opt_string(&row, "sender")?,
        title: opt_string(&row, "title")?,
        identifiers: identifiers_of(&row)?,
        notices,
        document,
    }))
}

/// Removes a hand-off; false when there was none with this id.
pub fn delete_handoff(db: &mut dyn Db, id: i64) -> Result<bool, StoreError> {
    Ok(execute(db, "DELETE FROM handoffs WHERE id = ?", &[Value::from(id)])? > 0)
}

/// A nullable text column.
fn opt_string(row: &Row, column: &str) -> Result<Option<String>, StoreError> {
    Ok(row.get(column)?.as_str().map(str::to_string))
}

/// The three identifier columns.
fn identifiers_of(row: &Row) -> Result<Identifiers, StoreError> {
    Ok(Identifiers {
        pmid: opt_string(row, "pmid")?,
        doi: opt_string(row, "doi")?,
        pmc_id: opt_string(row, "pmc_id")?,
    })
}

/// A row of [`pending_handoffs`]' query.
fn summary_from_row(row: &Row) -> Result<HandoffSummary, StoreError> {
    Ok(HandoffSummary {
        id: row.get_i64("id")?,
        received_at: row.get_str("received_at")?.to_string(),
        sender: opt_string(row, "sender")?,
        title: opt_string(row, "title")?,
        identifiers: identifiers_of(row)?,
        has_full_text: row.get_i64("has_full_text")? != 0,
        refusal: opt_string(row, "refusal")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::migrations;
    use crate::store::open_memory_store;
    use bmlib::db::{open_memory, run_migrations};

    fn handoff(title: &str, with_text: bool) -> Handoff {
        Handoff {
            sender: Some("MedicalFactChecker".into()),
            title: Some(title.into()),
            identifiers: Identifiers {
                pmid: Some("123".into()),
                doi: Some("10.1000/x".into()),
                pmc_id: None,
            },
            notices: vec!["A DOI was dropped.".into()],
            document: with_text.then(|| HandedDocument {
                kind: SourceKind::PlainText,
                file_name: Some("p.txt".into()),
                source: Some("Europe PMC".into()),
                bytes: vec![0, 1, 2, 255],
            }),
        }
    }

    /// A saved hand-off is listed with its fields.
    #[test]
    fn saved_handoff_is_pending() {
        let mut db = open_memory_store().unwrap();
        let id = save_handoff(&mut db, &handoff("T", true)).unwrap();
        let list = pending_handoffs(&mut db).unwrap();
        assert_eq!(list.len(), 1);
        let s = &list[0];
        assert_eq!(s.id, id);
        assert_eq!(s.sender.as_deref(), Some("MedicalFactChecker"));
        assert_eq!(s.title.as_deref(), Some("T"));
        assert_eq!(s.identifiers.pmid.as_deref(), Some("123"));
        assert!(s.has_full_text && s.refusal.is_none());
        assert!(!s.received_at.is_empty());
        save_handoff(&mut db, &handoff("U", false)).unwrap();
        assert!(!pending_handoffs(&mut db).unwrap()[0].has_full_text);
    }

    /// What is loaded equals what was saved, with and without text.
    #[test]
    fn load_round_trips() {
        let mut db = open_memory_store().unwrap();
        for with_text in [true, false] {
            let h = handoff("T", with_text);
            let id = save_handoff(&mut db, &h).unwrap();
            assert_eq!(load_handoff(&mut db, id).unwrap(), Some(h));
        }
        assert_eq!(load_handoff(&mut db, 999).unwrap(), None);
    }

    /// A refusal is listed with its reason and cannot be opened.
    #[test]
    fn refused_handoff_is_listed() {
        let mut db = open_memory_store().unwrap();
        let id = save_refused_handoff(&mut db, "too large").unwrap();
        let s = &pending_handoffs(&mut db).unwrap()[0];
        assert_eq!(s.refusal.as_deref(), Some("too large"));
        assert!(!s.has_full_text && s.sender.is_none());
        assert_eq!(load_handoff(&mut db, id).unwrap(), None);
    }

    /// The list is newest first.
    #[test]
    fn pending_newest_first() {
        let mut db = open_memory_store().unwrap();
        let a = save_handoff(&mut db, &handoff("A", false)).unwrap();
        let b = save_handoff(&mut db, &handoff("B", false)).unwrap();
        let ids: Vec<i64> = pending_handoffs(&mut db)
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec![b, a]);
    }

    /// Deleting removes the row once.
    #[test]
    fn delete_handoff_removes_it() {
        let mut db = open_memory_store().unwrap();
        let id = save_handoff(&mut db, &handoff("T", true)).unwrap();
        assert!(delete_handoff(&mut db, id).unwrap());
        assert!(!delete_handoff(&mut db, id).unwrap());
        assert!(pending_handoffs(&mut db).unwrap().is_empty());
    }

    /// A kind without bytes or bytes without a kind, an unknown kind, and
    /// notices that are not a list are corrupt, never defaulted (#67).
    #[test]
    fn corrupt_rows_are_errors() {
        let mut db = open_memory_store().unwrap();
        for insert in [
            "INSERT INTO handoffs (kind) VALUES ('pdf')",
            "INSERT INTO handoffs (bytes) VALUES (x'00')",
            "INSERT INTO handoffs (kind, bytes) VALUES ('docx', x'00')",
            "INSERT INTO handoffs (notices_json) VALUES ('{\"a\": 1}')",
        ] {
            execute(&mut db, insert, &[]).unwrap();
            let id = last_id(&mut db).unwrap();
            assert!(
                matches!(load_handoff(&mut db, id), Err(StoreError::Corrupt(_))),
                "{insert}"
            );
        }
    }

    /// A database at version 7 gains the table, and keeps its data: a paper
    /// stored before the migration is still there after it (#67).
    #[test]
    fn migration_8_on_a_v7_database() {
        let mut db = open_memory().unwrap();
        run_migrations(&mut db, migrations().into_iter().take(7).collect()).unwrap();
        assert!(execute(&mut db, "SELECT 1 FROM handoffs", &[]).is_err());
        execute(
            &mut db,
            "INSERT INTO papers (id, title) VALUES ('p1', 'Kept')",
            &[],
        )
        .unwrap();
        run_migrations(&mut db, migrations()).unwrap();
        let kept = fetch_one(&mut db, "SELECT title FROM papers WHERE id = 'p1'", &[])
            .unwrap()
            .unwrap();
        assert_eq!(kept.get_str("title").unwrap(), "Kept");
        let id = save_handoff(&mut db, &handoff("T", true)).unwrap();
        assert!(load_handoff(&mut db, id).unwrap().is_some());
    }
}
