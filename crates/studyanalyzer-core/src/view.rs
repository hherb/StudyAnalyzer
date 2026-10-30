//! The paper view handed to the frontend: the canonical text with every span
//! converted from code points (Rust) to UTF-16 units (JavaScript).
//!
//! This is the only module that converts offsets. Everything below it counts
//! code points; everything the frontend receives counts UTF-16 units.

use bmlib::fulltext::canonical::{BlockKind, TextSpan};
use serde::{Deserialize, Serialize};

use crate::intake::Decoded;
use crate::store::{StoreError, StoredCanonical};

/// A half-open range `[start, end)` of UTF-16 code units, as JavaScript's
/// `String.prototype.slice` counts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Utf16Span {
    /// The first unit covered.
    pub start: usize,
    /// One past the last unit covered.
    pub end: usize,
}

/// The whole paper as the frontend renders it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PaperView {
    /// The paper's id.
    pub paper_id: String,
    /// The paper's title.
    pub title: String,
    /// The full canonical text, never truncated.
    pub text: String,
    /// Where the title sits in `text`, if the paper has one.
    pub title_span: Option<Utf16Span>,
    /// The sections, in document order.
    pub sections: Vec<SectionView>,
    /// The converter version that produced `text`.
    pub converter_version: String,
    /// Whether the text passed the full-text check, and whether the user confirmed it.
    pub full_text: FullTextStatus,
    /// How the source's bytes were decoded; `None` for a PDF.
    pub decoded: Option<Decoded>,
}

/// One section of the paper view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SectionView {
    /// The section's heading level (1 is top level).
    pub level: usize,
    /// Where the heading sits in the text, if the section has one.
    pub heading: Option<Utf16Span>,
    /// The section's blocks, in order.
    pub blocks: Vec<BlockView>,
}

/// One block of a section, located in UTF-16 units.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlockView {
    /// Whether the block is prose or a table row.
    pub kind: BlockKind,
    /// Where the block sits in the text.
    pub span: Utf16Span,
}

/// The full-text check's outcome, as the paper banner needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FullTextStatus {
    /// Whether the check judged the text to be a whole paper.
    pub passed: bool,
    /// Whether the user confirmed "this is the whole paper".
    pub confirmed: bool,
    /// The words in the body.
    pub body_words: usize,
    /// The fewest body words the check accepts (bmlib's constant), so the banner can name it.
    pub min_body_words: usize,
    /// The headings the check requires and did not find, named for the banner
    /// (`Methods`, `Results or Discussion`); empty when it found them.
    pub missing_headings: Vec<String>,
}

/// The heading the full-text check requires, as bmlib's `SectionType` serialises.
const REQUIRED_HEADING: (&str, &str) = ("methods", "Methods");
/// The headings of which the check requires one, and their banner name.
const ONE_OF_HEADINGS: (&[&str], &str) = (&["results", "discussion"], "Results or Discussion");

/// The required headings `section_types` lacks, named for the banner.
///
/// bmlib keeps `assess_full_text`'s heading rule private, so this copies it
/// (Methods, and Results or Discussion); the test
/// `missing_headings_agree_with_bmlib` fails when bmlib's rule moves.
fn missing_headings(section_types: &[String]) -> Vec<String> {
    let has = |t: &str| section_types.iter().any(|s| s == t);
    let mut missing = Vec::new();
    if !has(REQUIRED_HEADING.0) {
        missing.push(REQUIRED_HEADING.1.to_string());
    }
    if !ONE_OF_HEADINGS.0.iter().any(|t| has(t)) {
        missing.push(ONE_OF_HEADINGS.1.to_string());
    }
    missing
}

/// The UTF-16 offset of every code point of `text`: entry `i` is where code
/// point `i` starts, and the last entry is the total length in units. One pass.
#[must_use]
pub fn utf16_offsets(text: &str) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(text.chars().count() + 1);
    let mut units = 0;
    for c in text.chars() {
        offsets.push(units);
        units += c.len_utf16();
    }
    offsets.push(units);
    offsets
}

/// Converts a code-point span into UTF-16 units using `offsets` from
/// [`utf16_offsets`].
///
/// # Panics
///
/// If `span` reaches past the table, so a wrong span cannot become a wrong
/// number silently. `paper_view` validates the text first, so this only fires
/// on a caller's own bad span.
#[must_use]
pub fn span_to_utf16(offsets: &[usize], span: TextSpan) -> Utf16Span {
    Utf16Span {
        start: offsets[span.start],
        end: offsets[span.end],
    }
}

/// Builds the paper view from a stored canonical text.
///
/// `decoded` is the decoding of the source the canonical text was derived from.
///
/// # Errors
///
/// [`StoreError::Corrupt`] if the canonical text fails validation or the
/// stored assessment is not the JSON this code wrote.
pub fn paper_view(
    paper_id: &str,
    title: &str,
    stored: &StoredCanonical,
    decoded: Option<&Decoded>,
) -> Result<PaperView, StoreError> {
    stored
        .text
        .validate()
        .map_err(|e| StoreError::Corrupt(format!("canonical text is invalid: {e}")))?;
    let full_text = full_text_status(&stored.assessment_json, stored.confirmed_full_text)?;
    let offsets = utf16_offsets(&stored.text.text);
    let to_utf16 = |span: TextSpan| span_to_utf16(&offsets, span);
    let sections = stored
        .text
        .sections
        .iter()
        .map(|section| SectionView {
            level: section.level,
            heading: section.heading_span.map(to_utf16),
            blocks: section
                .blocks
                .iter()
                .map(|block| BlockView {
                    kind: block.kind,
                    span: to_utf16(block.span),
                })
                .collect(),
        })
        .collect();
    Ok(PaperView {
        paper_id: paper_id.to_string(),
        title: title.to_string(),
        text: stored.text.text.clone(),
        title_span: stored.text.title_span.map(to_utf16),
        sections,
        converter_version: stored.text.converter_version.clone(),
        full_text,
        decoded: decoded.cloned(),
    })
}

/// Reads the stored assessment JSON into the banner's status.
///
/// # Errors
///
/// [`StoreError::Corrupt`] if the JSON is not an assessment this code wrote.
fn full_text_status(assessment_json: &str, confirmed: bool) -> Result<FullTextStatus, StoreError> {
    /// The fields of `FullTextAssessment`'s JSON that the view reads.
    #[derive(Deserialize)]
    struct Assessment {
        is_full_text: bool,
        section_types: Vec<String>,
        body_words: usize,
    }
    let parsed: Assessment = serde_json::from_str(assessment_json)
        .map_err(|e| StoreError::Corrupt(format!("assessment is invalid: {e}")))?;
    Ok(FullTextStatus {
        passed: parsed.is_full_text,
        confirmed,
        body_words: parsed.body_words,
        min_body_words: bmlib::fulltext::canonical::MIN_BODY_WORDS,
        missing_headings: missing_headings(&parsed.section_types),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmlib::fulltext::canonical::{
        assess_full_text, canonical_from_markdown, canonical_from_plain_text, CanonicalText,
    };

    /// Wraps a canonical text as the store would return it, with its real assessment.
    fn stored_of(text: CanonicalText, confirmed: bool) -> StoredCanonical {
        let assessment = assess_full_text(&text).unwrap();
        StoredCanonical {
            id: 1,
            source_id: 1,
            assessment_json: serde_json::to_string(&assessment).unwrap(),
            text,
            confirmed_full_text: confirmed,
        }
    }

    /// The UTF-16 units of `s`, as JavaScript would hold them.
    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// Decodes the units `span` covers, as JavaScript's `slice` would.
    fn slice(text: &str, span: Utf16Span) -> String {
        String::from_utf16(&units(text)[span.start..span.end]).unwrap()
    }

    /// The status carries bmlib's own word threshold, not a copy.
    #[test]
    fn status_carries_bmlib_threshold() {
        let stored = stored_of(canonical_from_markdown("# T\n\nBody."), false);
        let view = paper_view("p", "T", &stored, None).unwrap();
        assert_eq!(
            view.full_text.min_body_words,
            bmlib::fulltext::canonical::MIN_BODY_WORDS
        );
    }

    /// For every mix of Methods, Results and Discussion headings (with an
    /// Introduction and Conclusions always present, and enough words), the
    /// copied rule finds nothing missing exactly when bmlib's check passes.
    #[test]
    fn missing_headings_agree_with_bmlib() {
        let body = vec!["word"; bmlib::fulltext::canonical::MIN_BODY_WORDS].join(" ");
        let mut passes = 0;
        for mask in 0..8u8 {
            let mut md = format!("# T\n\n## Introduction\n\n{body}\n\n");
            for (bit, heading) in ["Methods", "Results", "Discussion"].iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    md.push_str(&format!("## {heading}\n\nx\n\n"));
                }
            }
            md.push_str("## Conclusions\n\nx\n");
            let stored = stored_of(canonical_from_markdown(&md), false);
            let status = paper_view("p", "T", &stored, None).unwrap().full_text;
            assert_eq!(status.missing_headings.is_empty(), status.passed, "{md}");
            passes += usize::from(status.passed);
        }
        // Methods with Results, with Discussion, or with both.
        assert_eq!(passes, 3);
    }

    /// Each missing heading is named once; none are named when all are found.
    #[test]
    fn missing_headings_named() {
        let types = |ts: &[&str]| ts.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        assert_eq!(
            missing_headings(&types(&["methods", "discussion"])),
            Vec::<String>::new()
        );
        assert_eq!(missing_headings(&types(&["results"])), vec!["Methods"]);
        assert_eq!(
            missing_headings(&types(&[])),
            vec!["Methods", "Results or Discussion"]
        );
    }

    /// ASCII has one unit per code point, so the offsets are the identity.
    #[test]
    fn ascii_is_identity() {
        assert_eq!(utf16_offsets("abc"), vec![0, 1, 2, 3]);
    }

    /// An astral character counts two units; `b` after it starts at 3.
    #[test]
    fn astral_counts_two() {
        let text = "a😀b𝛼c";
        let offsets = utf16_offsets(text);
        assert_eq!(offsets, vec![0, 1, 3, 4, 6, 7]);
        assert_eq!(
            span_to_utf16(&offsets, TextSpan { start: 2, end: 3 }),
            Utf16Span { start: 3, end: 4 }
        );
    }

    /// Every heading and block of a paper with astral characters slices to the
    /// same text Rust's spans name.
    #[test]
    fn view_spans_slice_same_text() {
        let canonical = canonical_from_markdown("# T😀\n\n## Méthods 𝛼\n\nBody 😀 text.");
        let view = paper_view("p", "T", &stored_of(canonical.clone(), false), None).unwrap();
        let title = canonical.title_span.unwrap();
        assert_eq!(
            slice(&view.text, view.title_span.unwrap()),
            canonical.span_text(title).unwrap()
        );
        assert_eq!(view.sections.len(), canonical.sections.len());
        for (vs, cs) in view.sections.iter().zip(&canonical.sections) {
            assert_eq!(vs.level, cs.level);
            let vh = vs.heading.unwrap();
            assert_eq!(
                slice(&view.text, vh),
                canonical.span_text(cs.heading_span.unwrap()).unwrap()
            );
            for (vb, cb) in vs.blocks.iter().zip(&cs.blocks) {
                assert_eq!(vb.kind, cb.kind);
                assert_eq!(
                    slice(&view.text, vb.span),
                    canonical.span_text(cb.span).unwrap()
                );
            }
        }
    }

    /// A failing assessment, unconfirmed, reads as not passed and not confirmed.
    #[test]
    fn status_from_assessment() {
        let text = canonical_from_plain_text("Short.", "T");
        let view = paper_view("p", "T", &stored_of(text, false), None).unwrap();
        assert!(!view.full_text.passed);
        assert!(!view.full_text.confirmed);
    }

    /// A span past the offsets table panics rather than returning a wrong number.
    #[test]
    #[should_panic]
    fn span_past_the_table_panics() {
        let offsets = utf16_offsets("ab");
        let _ = span_to_utf16(&offsets, TextSpan { start: 0, end: 9 });
    }

    /// A stored assessment that is not the JSON this code wrote is `Corrupt`.
    #[test]
    fn malformed_assessment_is_corrupt() {
        let mut stored = stored_of(canonical_from_plain_text("Para.", "T"), false);
        stored.assessment_json = "{\"is_full_text\": \"yes\"}".into();
        assert!(matches!(
            paper_view("p", "T", &stored, None),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// A canonical text whose spans do not tile the text is `Corrupt`, not viewed.
    #[test]
    fn invalid_canonical_text_is_corrupt() {
        let mut stored = stored_of(canonical_from_plain_text("Para.", "T"), false);
        stored.text.text.push_str(" stray");
        assert!(matches!(
            paper_view("p", "T", &stored, None),
            Err(StoreError::Corrupt(_))
        ));
    }
}
