//! The JSON contract with the frontend: every shape `src/lib/api.ts` mirrors by
//! hand, pinned in a fixture that `src/lib/contract.test.ts` reads too.

use bmlib::fulltext::canonical::{
    assess_full_text, canonical_from_markdown, BlockKind, CanonicalText,
};
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
/// something to name. `src/lib/contract.test.ts` asserts on its layout (one
/// section: paragraph, two table rows, paragraph) and its banner (both
/// headings missing, 8 words), so change the two together.
const PAPER: &str =
    "# Trial 😀\n\n## Méthods 𝛼\n\nBody 😀 text.\n\n| a | b😀 |\n|---|---|\n| 1 | 2 |\n\nAfter.\n";

/// A paper with no title and no heading, so the fixture pins the `null`s
/// (`title_span`, `heading`, and `decoded` for a file that needed none).
const BARE_PAPER: &str = "Just a body.\n";

/// Every error kind, kept by hand. [`every_kind_is_listed`] stops compiling
/// when `ErrorKind` gains one, to send its author here; add it here and to
/// `ERROR_KINDS` in `api.ts`.
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

/// Stops compiling when `ErrorKind` gains a variant. Add the variant here
/// and to [`EVERY_KIND`], or the fixture never shows the frontend the new kind.
fn every_kind_is_listed(kind: ErrorKind) {
    match kind {
        ErrorKind::InvalidInput
        | ErrorKind::TooLarge
        | ErrorKind::UnsupportedFormat
        | ErrorKind::Undecodable
        | ErrorKind::NeedsEmail
        | ErrorKind::Cancelled
        | ErrorKind::Storage
        | ErrorKind::Internal => {}
    }
}

/// Fails to compile when a bmlib bump adds a `BlockKind`: add it to
/// `BlockView.kind` in `api.ts`, to `groupBlocks`, and to [`PAPER`].
fn every_block_kind_is_mirrored(kind: BlockKind) {
    match kind {
        BlockKind::Paragraph | BlockKind::TableRow => {}
    }
}

/// `text` as stored, with its own assessment and the full-text check unconfirmed.
fn stored(text: CanonicalText) -> StoredCanonical {
    StoredCanonical {
        id: 1,
        source_id: 1,
        assessment_json: serde_json::to_string(&assess_full_text(&text).unwrap()).unwrap(),
        text,
        confirmed_full_text: false,
    }
}

/// Builds every value the fixture pins, plus `span_texts`: the title's,
/// then each heading's and block's text in document order, cut by code point
/// in Rust, for the frontend to compare its UTF-16 slices with.
fn contract() -> Value {
    let canonical = canonical_from_markdown(PAPER);
    let decoded = Decoded {
        encoding: "windows-1252".into(),
        guessed: true,
    };
    let view = paper_view("p1", "Trial 😀", &stored(canonical.clone()), Some(&decoded)).unwrap();
    let bare = paper_view("p2", "", &stored(canonical_from_markdown(BARE_PAPER)), None).unwrap();
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
        "paper_view_bare": bare,
        "span_texts": span_texts,
        "command_errors": errors,
    })
}

/// The values serialise exactly as the committed fixture. After a deliberate
/// change, run with `UPDATE_CONTRACT` set (to any value) to rewrite it; the
/// test then fails once so the rewrite is never a silent pass. Rerun without
/// it, then make `pnpm run check:web` and `pnpm run test:web` pass.
#[test]
fn json_matches_the_fixture() {
    EVERY_KIND.iter().for_each(|&k| every_kind_is_listed(k));
    every_block_kind_is_mirrored(BlockKind::Paragraph);
    let actual = contract();
    if std::env::var_os("UPDATE_CONTRACT").is_some() {
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(FIXTURE, pretty + "\n").unwrap();
        panic!("rewrote {FIXTURE}: review its diff, then rerun without UPDATE_CONTRACT");
    }
    let committed: Value =
        serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap()).unwrap();
    assert_eq!(
        actual, committed,
        "the frontend's JSON changed: rerun with UPDATE_CONTRACT=1, then update src/lib/api.ts"
    );
}
