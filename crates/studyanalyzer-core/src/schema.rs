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

/// The DDL of migration 3: a source keeps the notices given when it was
/// stored (intake's and storage's notices about its identifiers, and a
/// mismatch kept under the debugging setting), shown on every later open of
/// a text derived from it (#25).
const SCHEMA_V3: [&str; 1] =
    ["ALTER TABLE sources ADD COLUMN notices_json TEXT NOT NULL DEFAULT '[]'"];

/// The DDL of migration 4: a paper keeps why the document its last lookup
/// fetched was not stored on it, shown when it is reopened without text (#25).
const SCHEMA_V4: [&str; 1] = ["ALTER TABLE papers ADD COLUMN refusal TEXT"];

/// The DDL of migration 5: conversations with one paper, their turns, and the
/// verified quotes each answer cites (2b). A conversation is on one canonical
/// text, so a quote's span always points into it.
const SCHEMA_V5: [&str; 3] = [
    "CREATE TABLE conversations (id INTEGER PRIMARY KEY, paper_id TEXT NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  canonical_id INTEGER NOT NULL REFERENCES canonical_texts(id), created_at TEXT NOT NULL DEFAULT (datetime('now')))",
    "CREATE TABLE turns (id INTEGER PRIMARY KEY, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  question TEXT NOT NULL, answer TEXT NOT NULL, status TEXT NOT NULL, error TEXT, model TEXT NOT NULL, mode TEXT NOT NULL,
  passages_json TEXT NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')))",
    "CREATE TABLE quotes (id INTEGER PRIMARY KEY, turn_id INTEGER NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, number INTEGER, quote TEXT NOT NULL, span_start INTEGER, span_end INTEGER, similarity REAL NOT NULL)",
];

/// The DDL of migration 6: PICO runs on one paper (2c), their four parts, and
/// the paper's spans backing each. A run is on one canonical text, so a
/// citation's span always points into it; its wording is re-read from there.
const SCHEMA_V6: [&str; 3] = [
    "CREATE TABLE pico_runs (id INTEGER PRIMARY KEY, paper_id TEXT NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  canonical_id INTEGER NOT NULL REFERENCES canonical_texts(id), model TEXT NOT NULL,
  suitability_json TEXT, suitability_from_abstract INTEGER, extracted INTEGER NOT NULL, study_type TEXT, sample_size TEXT,
  overall_confidence REAL, condensed_from_chars INTEGER, condensation_status TEXT, notes_json TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now')))",
    "CREATE TABLE pico_parts (id INTEGER PRIMARY KEY, run_id INTEGER NOT NULL REFERENCES pico_runs(id) ON DELETE CASCADE,
  part TEXT NOT NULL, description TEXT NOT NULL, state TEXT NOT NULL, confidence REAL,
  quotes_not_found_json TEXT NOT NULL, quotes_dropped INTEGER NOT NULL)",
    "CREATE TABLE pico_citations (id INTEGER PRIMARY KEY, part_id INTEGER NOT NULL REFERENCES pico_parts(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, span_start INTEGER NOT NULL, span_end INTEGER NOT NULL,
  similarity REAL NOT NULL, exact INTEGER NOT NULL)",
];

/// Every migration, in order. bmlib records the applied ones in `schema_version`.
pub fn migrations() -> Vec<Migration> {
    vec![
        Migration::new(1, "initial schema", |db| run_all(db, &SCHEMA_V1)),
        Migration::new(2, "failed re-derive recorded", |db| run_all(db, &SCHEMA_V2)),
        Migration::new(3, "notices stored with a source", |db| {
            run_all(db, &SCHEMA_V3)
        }),
        Migration::new(4, "lookup refusal stored with a paper", |db| {
            run_all(db, &SCHEMA_V4)
        }),
        Migration::new(5, "conversations, turns and quotes", |db| {
            run_all(db, &SCHEMA_V5)
        }),
        Migration::new(6, "PICO runs, parts and citations", |db| {
            run_all(db, &SCHEMA_V6)
        }),
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
        assert_eq!(v, vec![1, 2, 3, 4, 5, 6]);
    }
}
