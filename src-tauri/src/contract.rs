//! The JSON contract with the frontend: every shape `src/lib/api.ts` mirrors by
//! hand, pinned in a fixture that `src/lib/contract.test.ts` reads too.

use bmlib::fulltext::canonical::{
    assess_full_text, canonical_from_markdown, BlockKind, CanonicalText,
};
use serde_json::{json, Value};
use studyanalyzer_core::identifiers::Identifiers;
use studyanalyzer_core::intake::{Decoded, SourceKind};
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

/// Every command `api.ts` invokes, with the argument keys it sends and each
/// one's JSON type. [`every_command_decodes_its_arguments`] sends these to the
/// real handlers; `contract.test.ts` checks the `api.ts` wrappers it calls send
/// exactly these. Both sides fail when a command is missing here: a new command
/// goes here, in `handler()`, in `api.ts` and in that test's call list together.
fn commands() -> Value {
    json!({
        "open_file": {"path": "string"},
        "open_paste": {"text": "string"},
        "open_identifier": {"input": "string"},
        "cancel_lookup": {},
        "attach_file": {"paperId": "string", "path": "string"},
        "attach_paste": {"paperId": "string", "text": "string"},
        "reopen_paper": {"paperId": "string"},
        "recent_papers": {},
        "paper_view": {"paperId": "string"},
        "original_pdf": {"paperId": "string"},
        "confirm_full_text": {"paperId": "string", "canonicalId": "number"},
        "get_contact_email": {},
        "set_contact_email": {"email": "string"},
        "get_keep_mismatched": {},
        "set_keep_mismatched": {"keep": "boolean"},
    })
}

/// `text` as stored, with its own assessment and the full-text check unconfirmed.
fn stored(text: CanonicalText) -> StoredCanonical {
    StoredCanonical {
        id: 1,
        source_id: 1,
        assessment_json: serde_json::to_string(&assess_full_text(&text).unwrap()).unwrap(),
        text,
        confirmed_full_text: false,
        rederive_failure: None,
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
    let view = paper_view(
        "p1",
        "Trial 😀",
        &stored(canonical.clone()),
        SourceKind::PlainText,
        Some(&decoded),
    )
    .unwrap();
    let bare = paper_view(
        "p2",
        "",
        &stored(canonical_from_markdown(BARE_PAPER)),
        SourceKind::Pdf,
        None,
    )
    .unwrap();
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
        "open_ready": OpenOutcome::Ready {
            paper_id: "p1".into(),
            notices: vec!["n".into()],
        },
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
        "commands": commands(),
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

/// The app's own origin as Tauri sets it per platform. A URL Tauri does not
/// treat as local is a remote origin, which the ACL refuses for app commands.
const APP_ORIGIN: &str = if cfg!(any(windows, target_os = "android")) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

/// Invokes `cmd` with `args` through the mock runtime, which has no
/// `AppState`, so a command that decodes its arguments fails with a
/// `CommandError` (an object) and touches nothing (`cancel_lookup`, which only
/// logs, returns null); one that cannot decode them, or that is not
/// registered, fails with Tauri's message (a string).
fn invoke(cmd: &str, args: Value) -> Result<Value, Value> {
    let app = crate::handler_app();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    tauri::test::get_ipc_response(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: APP_ORIGIN.parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize().unwrap())
}

/// A value of the JSON type the fixture names. `"number"` is always `1`, so a
/// Rust integer narrowed to another that still holds 1 is not caught.
fn sample(kind: &str) -> Value {
    match kind {
        "string" => json!("x"),
        "number" => json!(1),
        "boolean" => json!(true),
        other => panic!("no sample for JSON type {other}"),
    }
}

/// A value that is not of the JSON type the fixture names.
fn wrong_sample(kind: &str) -> Value {
    match kind {
        "string" => json!(1),
        "number" | "boolean" => json!("x"),
        other => panic!("no sample for JSON type {other}"),
    }
}

/// The arguments `api.ts` sends for `cmd`, built from `keys` (key → JSON type).
fn arguments(keys: &Value) -> serde_json::Map<String, Value> {
    keys.as_object()
        .unwrap()
        .iter()
        .map(|(k, kind)| (k.clone(), sample(kind.as_str().unwrap())))
        .collect()
}

/// Each command, given the keys and types `api.ts` sends, gets past argument
/// decoding, and each key is one Rust requires with that type: dropping it,
/// or sending another JSON type, is refused. So renaming, removing or
/// retyping a Rust parameter, or making it optional, fails here.
#[test]
fn every_command_decodes_its_arguments() {
    for (cmd, keys) in commands().as_object().unwrap() {
        let args = arguments(keys);
        if let Err(e) = invoke(cmd, Value::Object(args.clone())) {
            assert!(e.get("kind").is_some(), "{cmd} refused its arguments: {e}");
        }
        for (key, kind) in keys.as_object().unwrap() {
            let mut missing = args.clone();
            missing.remove(key);
            let e = invoke(cmd, Value::Object(missing)).unwrap_err();
            assert!(e.is_string(), "{cmd} ran without {key}: {e}");
            let mut wrong = args.clone();
            wrong.insert(key.clone(), wrong_sample(kind.as_str().unwrap()));
            let e = invoke(cmd, Value::Object(wrong)).unwrap_err();
            assert!(
                e.is_string(),
                "{cmd} took a {key} that is not a {kind}: {e}"
            );
        }
    }
}

/// The checks above can fail: a missing key or an unknown command is Tauri's
/// string, not a `CommandError`.
#[test]
fn a_missing_key_or_command_is_tauris_error() {
    assert!(invoke("open_file", json!({})).unwrap_err().is_string());
    assert!(invoke("no_such_command", json!({}))
        .unwrap_err()
        .is_string());
}

/// The command names `handler()` in `lib.rs` registers, read from its source.
fn registered_commands() -> Vec<String> {
    let src = include_str!("lib.rs");
    let start = src.find("generate_handler![").unwrap() + "generate_handler![".len();
    let end = start + src[start..].find(']').unwrap();
    src[start..end]
        .split(',')
        .map(|c| c.trim().trim_start_matches("commands::").to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

/// [`commands`] lists every command `handler()` registers, so a new one
/// cannot skip the decoding check above.
#[test]
fn every_registered_command_is_listed() {
    let mut registered = registered_commands();
    registered.sort();
    let listed: Vec<String> = commands().as_object().unwrap().keys().cloned().collect();
    assert_eq!(registered, listed, "add the command to commands() here");
}
