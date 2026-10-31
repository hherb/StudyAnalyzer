//! The database schema, as bmlib migrations.

use bmlib::db::{execute, Db, DbError, Migration};

/// The DDL of migration 1: papers, their original sources, canonical texts, settings.
const SCHEMA_V1: [&str; 4] = [
    "CREATE TABLE papers (id TEXT PRIMARY KEY, pmid TEXT UNIQUE, doi TEXT UNIQUE, pmc_id TEXT UNIQUE,
  title TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')), opened_at TEXT NOT NULL DEFAULT (datetime('now')))",
    "CREATE TABLE sources (id INTEGER PRIMARY KEY, paper_id TEXT NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, origin TEXT NOT NULL, file_name TEXT, bytes BLOB NOT NULL,
  encoding TEXT, encoding_guessed INTEGER NOT NULL DEFAULT 0, received_at TEXT NOT NULL DEFAULT (datetime('now')))",
    "CREATE TABLE canonical_texts (id INTEGER PRIMARY KEY, paper_id TEXT NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  source_id INTEGER NOT NULL REFERENCES sources(id), converter_version TEXT NOT NULL, canonical_json TEXT NOT NULL,
  assessment_json TEXT NOT NULL, confirmed_full_text INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
    "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
];

/// The DDL of migration 2: a canonical text remembers a failed attempt to
/// re-derive it (the converter version tried and the error), so the attempt
/// is not repeated on every open (#22).
const SCHEMA_V2: [&str; 2] = [
    "ALTER TABLE canonical_texts ADD COLUMN rederive_failed_version TEXT",
    "ALTER TABLE canonical_texts ADD COLUMN rederive_error TEXT",
];

/// Every migration, in order. bmlib records the applied ones in `schema_version`.
pub fn migrations() -> Vec<Migration> {
    vec![
        Migration::new(1, "initial schema", |db| run_all(db, &SCHEMA_V1)),
        Migration::new(2, "failed re-derive recorded", |db| run_all(db, &SCHEMA_V2)),
    ]
}

/// Runs one migration's statements in order.
fn run_all(db: &mut dyn Db, ddl: &[&str]) -> Result<(), DbError> {
    for statement in ddl {
        execute(db, statement, &[])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Versions are sequential from 1, as bmlib requires.
    #[test]
    fn versions_are_sequential() {
        let v: Vec<i64> = migrations().iter().map(|m| m.version).collect();
        assert_eq!(v, vec![1, 2]);
    }
}
