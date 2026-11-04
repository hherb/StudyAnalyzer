//! Reads every case in `fixtures/handoff/cases.json`: each hand-off file must
//! parse to what the case says (identifiers, sender, title, the document's
//! kind, name, source and bytes, and no notices), or be refused with the
//! named error. A sender's
//! own tests can check against the same files (see `docs/handoff-format.md`).

use std::path::PathBuf;

use serde_json::Value;
use studyanalyzer_core::handoff::{parse_handoff, HandoffError};

/// The fixtures folder.
fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/handoff")
}

/// The name of an error's variant, as `cases.json` writes it.
fn variant(e: &HandoffError) -> &'static str {
    match e {
        HandoffError::TooLarge { .. } => "TooLarge",
        HandoffError::NotAHandoff => "NotAHandoff",
        HandoffError::Newer { .. } => "Newer",
        HandoffError::Malformed(_) => "Malformed",
        HandoffError::Nothing { .. } => "Nothing",
        HandoffError::Link(_) => "Link",
    }
}

/// A case's optional string field.
fn opt<'a>(case: &'a Value, key: &str) -> Option<&'a str> {
    case[key].as_str()
}

#[test]
fn every_case_gives_its_outcome() {
    let cases: Value =
        serde_json::from_slice(&std::fs::read(dir().join("cases.json")).unwrap()).unwrap();
    let cases = cases["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let file = case["file"].as_str().unwrap();
        let bytes = std::fs::read(dir().join(file)).unwrap();
        let result = parse_handoff(&bytes);
        let expect = case["expect"].as_str().unwrap();
        if expect != "ok" {
            let err = result.expect_err(file);
            assert_eq!(variant(&err), expect, "{file}");
            continue;
        }
        let h = result.unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(h.identifiers.pmid.as_deref(), opt(case, "pmid"), "{file}");
        assert_eq!(h.identifiers.doi.as_deref(), opt(case, "doi"), "{file}");
        assert_eq!(
            h.identifiers.pmc_id.as_deref(),
            opt(case, "pmcid"),
            "{file}"
        );
        assert_eq!(h.sender.as_deref(), opt(case, "sender"), "{file}");
        assert_eq!(h.title.as_deref(), opt(case, "title"), "{file}");
        let doc = h.document.as_ref();
        assert_eq!(doc.map(|d| d.kind.as_str()), opt(case, "kind"), "{file}");
        let name = doc.and_then(|d| d.file_name.as_deref());
        assert_eq!(name, opt(case, "file_name"), "{file}");
        let source = doc.and_then(|d| d.source.as_deref());
        assert_eq!(source, opt(case, "source"), "{file}");
        // `bytes` names the fixture paper (in the folder above) the data encodes.
        let expected = opt(case, "bytes").map(|f| std::fs::read(dir().join("..").join(f)).unwrap());
        assert_eq!(doc.map(|d| &d.bytes), expected.as_ref(), "{file}");
        assert!(h.notices.is_empty(), "{file}: {:?}", h.notices);
    }
}
