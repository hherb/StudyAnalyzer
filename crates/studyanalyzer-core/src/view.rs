//! The paper view handed to the frontend: the canonical text with every span
//! converted from code points (Rust) to UTF-16 units (JavaScript).
//!
//! This is the only module that converts offsets. Everything below it counts
//! code points; everything the frontend receives counts UTF-16 units.

use bmlib::db::Db;
use bmlib::fulltext::canonical::{BlockKind, CanonicalText, TextSpan};
use serde::{Deserialize, Serialize};

use crate::answer::{answer_pieces, AnswerPiece};
use crate::conversations::{
    conversation_text, conversations, current_conversation, turns, ConversationSummary, Mode,
    StoredTurn, TurnStatus,
};
use crate::intake::{Decoded, SourceKind};
use crate::store::{
    canonical_by_id, latest_canonical, paper_title, source_info, PaperId, StoreError,
    StoredCanonical,
};

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
    /// The stored canonical text shown, so a confirmation names the text the
    /// user saw, not one a re-derive stored since.
    pub canonical_id: i64,
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
    /// What kind of source the text was derived from.
    pub source_kind: SourceKind,
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
const REQUIRED_HEADING: &str = "methods";
/// [`REQUIRED_HEADING`]'s name in the banner.
const REQUIRED_HEADING_NAME: &str = "Methods";
/// The headings of which the check requires at least one, as `SectionType` serialises.
const ONE_OF_HEADINGS: &[&str] = &["results", "discussion"];
/// [`ONE_OF_HEADINGS`]' shared name in the banner.
const ONE_OF_HEADINGS_NAME: &str = "Results or Discussion";

/// The required headings `section_types` lacks, named for the banner.
///
/// bmlib keeps `assess_full_text`'s heading rule private, so this copies it
/// (Methods, and Results or Discussion). `missing_headings_agree_with_bmlib`
/// fails when bmlib changes how it treats any of the five headings it
/// varies; a newly required heading of another type would go unnoticed.
/// The types stay strings, so an assessment stored under an older bmlib
/// still loads after a `SectionType` is renamed.
fn missing_headings(section_types: &[String]) -> Vec<String> {
    let has = |t: &str| section_types.iter().any(|s| s == t);
    let mut missing = Vec::new();
    if !has(REQUIRED_HEADING) {
        missing.push(REQUIRED_HEADING_NAME.to_string());
    }
    if !ONE_OF_HEADINGS.iter().any(|t| has(t)) {
        missing.push(ONE_OF_HEADINGS_NAME.to_string());
    }
    missing
}

/// Whether the stored verdict and the copied heading rule disagree: failed
/// with no reason to name, or passed with one. Possible for an assessment
/// stored under an older bmlib.
fn verdict_disagrees(status: &FullTextStatus) -> bool {
    let has_reason =
        !status.missing_headings.is_empty() || status.body_words < status.min_body_words;
    status.passed == has_reason
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
/// `source_kind` and `decoded` describe the source the canonical text was
/// derived from: its kind, and how its bytes were decoded.
///
/// # Errors
///
/// [`StoreError::Corrupt`] if the canonical text fails validation or the
/// stored assessment is not the JSON this code wrote.
pub fn paper_view(
    paper_id: &str,
    title: &str,
    stored: &StoredCanonical,
    source_kind: SourceKind,
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
        canonical_id: stored.id,
        text: stored.text.text.clone(),
        title_span: stored.text.title_span.map(to_utf16),
        sections,
        converter_version: stored.text.converter_version.clone(),
        full_text,
        source_kind,
        decoded: decoded.cloned(),
    })
}

/// A located quote of an answer: its number and the paper's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CitationView {
    /// The number the answer shows as `[n]`.
    pub number: u32,
    /// Where the quote sits in the paper's text.
    pub span: Utf16Span,
    /// The paper's wording at `span`, never the model's copy.
    pub text: String,
}

/// A quote the model wrote that could not be found in the paper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnlocatedView {
    /// The quote as the model wrote it.
    pub quote: String,
}

/// One question and its answer, as the chat pane shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnView {
    /// The turn's id.
    pub id: i64,
    /// The user's question.
    pub question: String,
    /// The answer: text and numbered citation markers.
    pub pieces: Vec<AnswerPiece>,
    /// The located quotes, in the order the answer cites them.
    pub citations: Vec<CitationView>,
    /// The quotes that could not be found in the paper, in order.
    pub unlocated: Vec<UnlocatedView>,
    /// How the turn ended.
    pub status: TurnStatus,
    /// Why it failed for a failed turn, or the cut-off notice for a complete
    /// one. Read `status`, not this, to tell them apart.
    pub error: Option<String>,
    /// The model that answered.
    pub model: String,
    /// What the model was given of the paper.
    pub mode: Mode,
}

/// A conversation with all its turns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConversationView {
    /// The conversation's id.
    pub id: i64,
    /// The canonical text its spans point into (show it with `paper_view_at`).
    pub canonical_id: i64,
    /// True unless this is the paper's current conversation (the newest on
    /// its latest text): a new question always goes to that one, so nothing
    /// more can be asked here. An earlier conversation, on the latest text or
    /// an older one, is read-only.
    pub read_only: bool,
    /// The turns, oldest first.
    pub turns: Vec<TurnView>,
}

/// Everything the Interrogator tab opens with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InterrogatorView {
    /// The paper's latest canonical text, the one questions go to.
    pub canonical_id: i64,
    /// The newest conversation on that text, if there is one.
    pub current: Option<ConversationView>,
    /// Every other conversation with turns, newest first.
    pub earlier: Vec<ConversationSummary>,
}

/// A stored turn as shown, with its spans in `offsets`' UTF-16 units
/// ([`utf16_offsets`] of `text`, the conversation's canonical text).
fn turn_view(
    turn: StoredTurn,
    text: &CanonicalText,
    offsets: &[usize],
) -> Result<TurnView, StoreError> {
    let pieces = answer_pieces(&turn.answer, &turn.quotes);
    let mut citations = Vec::new();
    let mut unlocated = Vec::new();
    for q in turn.quotes {
        match (q.number, q.span) {
            (Some(number), Some(span)) => {
                // `turns` checked every span; this only fails on damaged data.
                let words = text.span_text(span).ok_or_else(|| {
                    StoreError::Corrupt(format!(
                        "turn {}: quote span {span} is outside the text",
                        turn.id
                    ))
                })?;
                citations.push(CitationView {
                    number,
                    span: span_to_utf16(offsets, span),
                    text: words.to_string(),
                });
            }
            _ => unlocated.push(UnlocatedView { quote: q.quote }),
        }
    }
    Ok(TurnView {
        id: turn.id,
        question: turn.question,
        pieces,
        citations,
        unlocated,
        status: turn.status,
        error: turn.error,
        model: turn.model,
        mode: turn.mode,
    })
}

/// The conversation `conversation_id` with its turns.
///
/// # Errors
///
/// [`StoreError::NotFound`] when there is no such conversation; any other
/// store failure, or a stored span outside its text, as [`StoreError`].
pub fn conversation_view(
    db: &mut dyn Db,
    conversation_id: i64,
) -> Result<ConversationView, StoreError> {
    let (paper, canonical_id) = conversation_text(db, conversation_id)?
        .ok_or_else(|| StoreError::NotFound(format!("conversation {conversation_id}")))?;
    let stored = canonical_by_id(db, &paper, canonical_id)?.ok_or_else(|| {
        StoreError::Corrupt(format!(
            "conversation {conversation_id} is on text {canonical_id}, which is not its paper's"
        ))
    })?;
    // Questions go to the newest conversation on the latest text; every other one is history.
    let read_only = match latest_canonical(db, &paper)? {
        Some(latest) => current_conversation(db, &paper, latest.id)? != Some(conversation_id),
        None => true,
    };
    let offsets = utf16_offsets(&stored.text.text);
    let turns = turns(db, conversation_id)?
        .into_iter()
        .map(|t| turn_view(t, &stored.text, &offsets))
        .collect::<Result<_, _>>()?;
    Ok(ConversationView {
        id: conversation_id,
        canonical_id,
        read_only,
        turns,
    })
}

/// The Interrogator of `paper`: its latest text, the newest conversation on it
/// and the other conversations that hold turns.
///
/// # Errors
///
/// [`StoreError::NotFound`] when the paper has no canonical text; any other
/// store failure as [`StoreError`].
pub fn interrogator_view(db: &mut dyn Db, paper: &PaperId) -> Result<InterrogatorView, StoreError> {
    let canonical_id = latest_canonical(db, paper)?
        .ok_or_else(|| StoreError::NotFound(format!("full text of paper {}", paper.as_str())))?
        .id;
    let current_id = current_conversation(db, paper, canonical_id)?;
    let current = current_id.map(|id| conversation_view(db, id)).transpose()?;
    let earlier = conversations(db, paper)?
        .into_iter()
        .filter(|c| Some(c.id) != current_id && c.turns > 0)
        .collect();
    Ok(InterrogatorView {
        canonical_id,
        current,
        earlier,
    })
}

/// The paper view of `paper`'s canonical text `canonical_id`, which need not be
/// its latest (an earlier conversation's text).
///
/// # Errors
///
/// [`StoreError::NotFound`] when the paper has no text with that id (another
/// paper's included); otherwise as [`paper_view`].
pub fn stored_paper_view(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<PaperView, StoreError> {
    let stored = canonical_by_id(db, paper, canonical_id)?.ok_or_else(|| {
        StoreError::NotFound(format!("text {canonical_id} of paper {}", paper.as_str()))
    })?;
    let title = paper_title(db, paper)?;
    let (kind, decoded) = source_info(db, stored.source_id)?;
    paper_view(paper.as_str(), &title, &stored, kind, decoded.as_ref())
}

/// Reads the stored assessment JSON into the banner's status.
///
/// # Errors
///
/// [`StoreError::Corrupt`] if the JSON is not an assessment this code wrote.
pub(crate) fn full_text_status(
    assessment_json: &str,
    confirmed: bool,
) -> Result<FullTextStatus, StoreError> {
    /// The fields of `FullTextAssessment`'s JSON that the view reads.
    #[derive(Deserialize)]
    struct Assessment {
        is_full_text: bool,
        section_types: Vec<String>,
        body_words: usize,
    }
    let parsed: Assessment = serde_json::from_str(assessment_json)
        .map_err(|e| StoreError::Corrupt(format!("assessment is invalid: {e}")))?;
    let status = FullTextStatus {
        passed: parsed.is_full_text,
        confirmed,
        body_words: parsed.body_words,
        min_body_words: bmlib::fulltext::canonical::MIN_BODY_WORDS,
        missing_headings: missing_headings(&parsed.section_types),
    };
    if verdict_disagrees(&status) {
        log::warn!(
            "stored full-text verdict (passed: {}) disagrees with the heading rule \
             (section types {:?}, {} words)",
            status.passed,
            parsed.section_types,
            status.body_words
        );
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::{create_conversation, save_turn, NewQuote, NewTurn};
    use crate::store::{create_paper, open_memory_store, save_canonical, save_source, NewSource};
    use bmlib::fulltext::canonical::{
        assess_full_text, canonical_from_markdown, canonical_from_plain_text,
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
            rederive_failure: None,
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

    /// A paper with astral characters before and inside the quoted block.
    const ASTRAL: &str = "# Trial 😀\n\n## Méthods 𝛼\n\nBody 😀 text.\n\nAfter.\n";

    /// Stores `md` as a new canonical text of `paper` (a new source each time).
    fn add_text(db: &mut rusqlite::Connection, paper: &PaperId, md: &str) -> i64 {
        let sid = save_source(
            db,
            paper,
            NewSource {
                kind: SourceKind::Markdown,
                origin: "paste".into(),
                file_name: None,
                bytes: md.as_bytes().to_vec(),
                decoded: None,
            },
        )
        .unwrap();
        let text = canonical_from_markdown(md);
        let a = assess_full_text(&text).unwrap();
        save_canonical(db, paper, sid, &text, &a).unwrap()
    }

    /// The code-point span of `needle` in `text`.
    fn span_of(text: &CanonicalText, needle: &str) -> TextSpan {
        let byte = text.text.find(needle).unwrap();
        let start = text.text[..byte].chars().count();
        TextSpan {
            start,
            end: start + needle.chars().count(),
        }
    }

    /// A complete turn citing `located` (in the model's lower-case copy)
    /// at its place in `text`, and one quote the paper does not hold.
    fn cited_turn(text: &CanonicalText, located: &str) -> NewTurn {
        NewTurn {
            question: "What?".into(),
            answer: format!(
                "It says <q>{}</q> and <q>nowhere</q>.",
                located.to_lowercase()
            ),
            status: TurnStatus::Complete,
            error: None,
            model: "ollama:m".into(),
            mode: Mode::WholePaper,
            passages: Vec::new(),
            quotes: vec![
                NewQuote {
                    number: Some(1),
                    quote: located.to_lowercase(),
                    span: Some(span_of(text, located)),
                    similarity: 1.0,
                },
                NewQuote {
                    number: None,
                    quote: "nowhere".into(),
                    span: None,
                    similarity: 0.1,
                },
            ],
            input_tokens: 1,
            output_tokens: 1,
        }
    }

    /// A store with one paper on [`ASTRAL`] and a conversation holding one
    /// cited turn; returns the store, paper, text id and conversation id.
    fn conversation_on_astral() -> (rusqlite::Connection, PaperId, i64, i64) {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &Default::default(), Some("Trial")).unwrap();
        let cid = add_text(&mut db, &p, ASTRAL);
        let conv = create_conversation(&mut db, &p, cid).unwrap();
        let text = canonical_from_markdown(ASTRAL);
        save_turn(&mut db, conv, &cited_turn(&text, "Body 😀 text.")).unwrap();
        (db, p, cid, conv)
    }

    /// On a paper with astral characters, a citation's UTF-16 span slices the
    /// paper's text to exactly the citation's `text`.
    #[test]
    fn citation_spans_slice_same_text() {
        let (mut db, p, cid, conv) = conversation_on_astral();
        let view = conversation_view(&mut db, conv).unwrap();
        let paper = stored_paper_view(&mut db, &p, cid).unwrap();
        let c = &view.turns[0].citations[0];
        assert_eq!(slice(&paper.text, c.span), c.text);
        assert_eq!(c.text, "Body 😀 text.");
        // The astral characters before it shift the UTF-16 offsets.
        let points = span_of(&canonical_from_markdown(ASTRAL), "Body 😀 text.");
        assert_eq!(c.span.start, points.start + 2);
    }

    /// The citation shows the paper's wording, not the model's copy; the
    /// unlocated quote is listed as the model wrote it; the pieces number it.
    #[test]
    fn turn_view_shows_paper_wording() {
        let (mut db, _, cid, conv) = conversation_on_astral();
        let view = conversation_view(&mut db, conv).unwrap();
        assert_eq!(
            (view.id, view.canonical_id, view.read_only),
            (conv, cid, false)
        );
        let t = &view.turns[0];
        assert_eq!(t.citations.len(), 1);
        assert_eq!(t.citations[0].number, 1);
        assert_eq!(t.citations[0].text, "Body 😀 text.");
        assert_ne!(t.citations[0].text, "body 😀 text.");
        assert_eq!(
            t.unlocated,
            vec![UnlocatedView {
                quote: "nowhere".into()
            }]
        );
        assert_eq!(
            t.pieces,
            vec![
                AnswerPiece::Text {
                    text: "It says ".into()
                },
                AnswerPiece::Cite { number: 1 },
                AnswerPiece::Text {
                    text: " and .".into()
                },
            ]
        );
        assert_eq!(
            (t.question.as_str(), t.status, t.mode, t.model.as_str()),
            ("What?", TurnStatus::Complete, Mode::WholePaper, "ollama:m")
        );
    }

    /// A stopped turn stores no quotes: its quote shows as curly-quoted text,
    /// with no citation and nothing unlocated.
    #[test]
    fn stopped_turn_keeps_its_quote_as_text() {
        let (mut db, _, _, conv) = conversation_on_astral();
        let mut turn = cited_turn(&canonical_from_markdown(ASTRAL), "After.");
        turn.status = TurnStatus::Stopped;
        turn.answer = "Partly <q>Bod".into();
        turn.quotes.clear();
        save_turn(&mut db, conv, &turn).unwrap();
        let t = conversation_view(&mut db, conv)
            .unwrap()
            .turns
            .pop()
            .unwrap();
        assert_eq!(t.status, TurnStatus::Stopped);
        assert!(t.citations.is_empty() && t.unlocated.is_empty());
        assert_eq!(
            t.pieces,
            vec![AnswerPiece::Text {
                text: "Partly \u{201c}Bod\u{201d}".into()
            }]
        );
    }

    /// After a new document, the old conversation is read-only on its own
    /// text, which `stored_paper_view` still shows; the Interrogator moves to
    /// the new text with no current conversation and lists the old one.
    #[test]
    fn earlier_conversation_uses_its_text() {
        let (mut db, p, old, conv) = conversation_on_astral();
        let new = add_text(&mut db, &p, "# Trial\n\nAnother text.\n");
        let view = conversation_view(&mut db, conv).unwrap();
        assert!(view.read_only);
        assert_eq!(view.canonical_id, old);
        let shown = stored_paper_view(&mut db, &p, old).unwrap();
        assert_eq!(shown.canonical_id, old);
        assert_eq!(shown.text, canonical_from_markdown(ASTRAL).text);
        assert_eq!(shown.title, "Trial");
        let i = interrogator_view(&mut db, &p).unwrap();
        assert_eq!(i.canonical_id, new);
        assert_eq!(i.current, None);
        assert_eq!(i.earlier.iter().map(|c| c.id).collect::<Vec<_>>(), [conv]);
        let fresh = create_conversation(&mut db, &p, new).unwrap();
        assert!(!conversation_view(&mut db, fresh).unwrap().read_only);
    }

    /// The current conversation is not listed as earlier, nor is an empty one.
    #[test]
    fn interrogator_lists_other_conversations_with_turns() {
        let (mut db, p, cid, conv) = conversation_on_astral();
        let i = interrogator_view(&mut db, &p).unwrap();
        assert_eq!(i.current.map(|c| c.id), Some(conv));
        assert!(i.earlier.is_empty());
        let empty = create_conversation(&mut db, &p, cid).unwrap();
        let i = interrogator_view(&mut db, &p).unwrap();
        assert_eq!(
            i.current
                .as_ref()
                .map(|c| (c.id, c.turns.len(), c.read_only)),
            Some((empty, 0, false))
        );
        // An earlier conversation on the current text cannot be asked in either.
        assert!(conversation_view(&mut db, conv).unwrap().read_only);
        assert_eq!(i.earlier.iter().map(|c| c.id).collect::<Vec<_>>(), [conv]);
        create_conversation(&mut db, &p, cid).unwrap();
        let i = interrogator_view(&mut db, &p).unwrap();
        assert_eq!(i.earlier.iter().map(|c| c.id).collect::<Vec<_>>(), [conv]);
    }

    /// A paper with no text, an unknown conversation, or another paper's text
    /// is `NotFound`.
    #[test]
    fn missing_things_are_not_found() {
        let (mut db, _, cid, conv) = conversation_on_astral();
        let other = create_paper(&mut db, &Default::default(), None).unwrap();
        assert!(matches!(
            interrogator_view(&mut db, &other),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            conversation_view(&mut db, conv + 1),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            stored_paper_view(&mut db, &other, cid),
            Err(StoreError::NotFound(_))
        ));
    }

    /// The status carries bmlib's own word threshold, not a copy.
    #[test]
    fn status_carries_bmlib_threshold() {
        let stored = stored_of(canonical_from_markdown("# T\n\nBody."), false);
        let view = paper_view("p", "T", &stored, SourceKind::Markdown, None).unwrap();
        assert_eq!(
            view.full_text.min_body_words,
            bmlib::fulltext::canonical::MIN_BODY_WORDS
        );
    }

    /// For every mix of Introduction, Methods, Results, Discussion and
    /// Conclusions headings (the body words come first, under no heading),
    /// the copied rule finds nothing missing exactly when bmlib's check
    /// passes, so bmlib starting to require or accept any of the five fails.
    #[test]
    fn missing_headings_agree_with_bmlib() {
        let body = vec!["word"; bmlib::fulltext::canonical::MIN_BODY_WORDS].join(" ");
        let headings = [
            "Introduction",
            "Methods",
            "Results",
            "Discussion",
            "Conclusions",
        ];
        let mut passes = 0;
        for mask in 0..(1u8 << headings.len()) {
            let mut md = format!("# T\n\n{body}\n\n");
            for (bit, heading) in headings.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    md.push_str(&format!("## {heading}\n\nx\n\n"));
                }
            }
            let stored = stored_of(canonical_from_markdown(&md), false);
            let status = paper_view("p", "T", &stored, SourceKind::Markdown, None)
                .unwrap()
                .full_text;
            assert_eq!(status.missing_headings.is_empty(), status.passed, "{md}");
            assert!(!verdict_disagrees(&status), "{md}");
            passes += usize::from(status.passed);
        }
        // Methods with Results, Discussion or both (3), each with or
        // without Introduction and Conclusions (4).
        assert_eq!(passes, 12);
    }

    /// Failing with no reason to name, or passing with one, disagrees.
    #[test]
    fn verdict_disagreement_detected() {
        let status = |passed: bool, missing: &[&str], body_words: usize| FullTextStatus {
            passed,
            confirmed: false,
            body_words,
            min_body_words: 1000,
            missing_headings: missing.iter().map(|m| m.to_string()).collect(),
        };
        assert!(!verdict_disagrees(&status(true, &[], 1000)));
        assert!(!verdict_disagrees(&status(false, &["Methods"], 5000)));
        assert!(!verdict_disagrees(&status(false, &[], 10)));
        assert!(verdict_disagrees(&status(false, &[], 5000)));
        assert!(verdict_disagrees(&status(true, &["Methods"], 5000)));
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
        let view = paper_view(
            "p",
            "T",
            &stored_of(canonical.clone(), false),
            SourceKind::Markdown,
            None,
        )
        .unwrap();
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
        let view = paper_view(
            "p",
            "T",
            &stored_of(text, false),
            SourceKind::Markdown,
            None,
        )
        .unwrap();
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
            paper_view("p", "T", &stored, SourceKind::Markdown, None),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// A canonical text whose spans do not tile the text is `Corrupt`, not viewed.
    #[test]
    fn invalid_canonical_text_is_corrupt() {
        let mut stored = stored_of(canonical_from_plain_text("Para.", "T"), false);
        stored.text.text.push_str(" stray");
        assert!(matches!(
            paper_view("p", "T", &stored, SourceKind::Markdown, None),
            Err(StoreError::Corrupt(_))
        ));
    }
}
