//! The JSON contract with the frontend: every shape `src/lib/api.ts` mirrors by
//! hand, pinned in a fixture that `src/lib/contract.test.ts` reads too.

use bmlib::fulltext::canonical::{assess_full_text, canonical_from_markdown};
use serde_json::{json, Value};
use studyanalyzer_core::identifiers::Identifiers;
use studyanalyzer_core::intake::Decoded;
use studyanalyzer_core::open::OpenOutcome;
use studyanalyzer_core::store::{PaperSummary, StoredCanonical};
use studyanalyzer_core::view::paper_view;

use crate::error::{CommandError, ErrorKind};

/// The fixture both sides read.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/lib/fixtures/ipc_contract.json"
);

/// A paper with astral characters (two UTF-16 units each), a table and a
/// heading the full-text check does not recognise, so the banner has
/// something to name.
const PAPER: &str =
    "# Trial 😀\n\n## Méthods 𝛼\n\nBody 😀 text.\n\n| a | b😀 |\n|---|---|\n| 1 | 2 |\n\nAfter.\n";

/// Every error kind. Add a new kind here, then to `ERROR_KINDS` in `api.ts`.
const EVERY_KIND: [ErrorKind; 8] = [
    ErrorKind::InvalidInput,
    ErrorKind::TooLarge,
    ErrorKind::UnsupportedFormat,
    ErrorKind::Undecodable,
    ErrorKind::NeedsEmail,
    ErrorKind::Cancelled,
    ErrorKind::Storage,
    ErrorKind::Internal,
];

/// Builds every value the fixture pins, plus `span_texts`: the title's,
/// then each heading's and block's text in document order, cut by code point
/// in Rust, for the frontend to compare its UTF-16 slices with.
fn contract() -> Value {
    let canonical = canonical_from_markdown(PAPER);
    let stored = StoredCanonical {
        id: 1,
        source_id: 1,
        assessment_json: serde_json::to_string(&assess_full_text(&canonical).unwrap()).unwrap(),
        text: canonical.clone(),
        confirmed_full_text: false,
    };
    let decoded = Decoded {
        encoding: "windows-1252".into(),
        guessed: true,
    };
    let view = paper_view("p1", "Trial 😀", &stored, Some(&decoded)).unwrap();
    let spans = canonical
        .title_span
        .into_iter()
        .chain(canonical.sections.iter().flat_map(|s| {
            s.heading_span
                .into_iter()
                .chain(s.blocks.iter().map(|b| b.span))
        }));
    let span_texts: Vec<&str> = spans.map(|s| canonical.span_text(s).unwrap()).collect();
    let summary = PaperSummary {
        id: "p1".into(),
        title: "Trial 😀".into(),
        identifiers: Identifiers {
            pmid: Some("123".into()),
            doi: Some("10.1/x".into()),
            pmc_id: None,
        },
        opened_at: "2026-10-31 09:00:00".into(),
    };
    let errors: Vec<CommandError> = EVERY_KIND
        .iter()
        .map(|&kind| CommandError {
            kind,
            message: "m".into(),
        })
        .collect();
    json!({
        "open_ready": OpenOutcome::Ready { paper_id: "p1".into() },
        "open_needs_full_text": OpenOutcome::NeedsFullText {
            paper_id: "p1".into(),
            reason: "r".into(),
            unreachable: true,
        },
        "paper_summary": summary,
        "paper_view": view,
        "span_texts": span_texts,
        "command_errors": errors,
    })
}

/// The values serialise exactly as the committed fixture. After a deliberate
/// change, update `api.ts`, then rerun with `UPDATE_CONTRACT=1` to rewrite it.
#[test]
fn json_matches_the_fixture() {
    let actual = contract();
    if std::env::var_os("UPDATE_CONTRACT").is_some() {
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(FIXTURE, pretty + "\n").unwrap();
    }
    let committed: Value =
        serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap()).unwrap();
    assert_eq!(
        actual, committed,
        "the frontend's JSON changed: update src/lib/api.ts, then rerun with UPDATE_CONTRACT=1"
    );
}
