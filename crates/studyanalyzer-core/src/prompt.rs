//! The Interrogator's prompt: which mode fits the model's window, the paper (or
//! its best passages) as context, and the messages sent.

use bmlib::embed::store::ChunkHit;
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::llm::LLMMessage;

use crate::config::{ANSWER_TOKENS, FIT_CHARS_PER_TOKEN};
use crate::conversations::{Mode, StoredTurn, TurnStatus};

/// What the model is told, before the paper.
pub const SYSTEM_PROMPT: &str = "You answer questions about one biomedical paper, using only the text of the paper given below.\n\
- Answer only from the paper. If the paper does not say, answer that the paper does not say. Never use outside knowledge.\n\
- Back every claim with a quote copied exactly from the paper, written straight after the claim as <q>exact words from the paper</q>.\n\
- Copy quotes word for word: no ellipses, no paraphrase, no added words. Quote a whole sentence or a clause of one.\n\
- Quote the numbers as the paper gives them.";

/// The window a run will use: the smaller of the user's setting and the
/// model's maximum, whichever are known; `None` when neither is.
pub fn running_window(setting: Option<u64>, maximum: Option<u64>) -> Option<u64> {
    match (setting, maximum) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The tokens `chars` code points are estimated to take: `ceil(chars / FIT_CHARS_PER_TOKEN)`.
pub fn estimated_tokens(chars: usize) -> u64 {
    (chars as u64).div_ceil(FIT_CHARS_PER_TOKEN)
}

/// Whether `messages` plus the room for the answer fit in `window` tokens.
/// Counts code points, not bytes. An unknown window (`None`) never fits.
pub fn fits(window: Option<u64>, messages: &[LLMMessage]) -> bool {
    let Some(window) = window else {
        return false;
    };
    let chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    estimated_tokens(chars) + ANSWER_TOKENS <= window
}

/// The text of `span`, or empty when the span does not lie in the text.
fn text_of(canonical: &CanonicalText, span: Option<TextSpan>) -> &str {
    span.and_then(|s| canonical.span_text(s)).unwrap_or("")
}

/// A section heading as markdown: `#` repeated `level + 1` times, then the text.
fn heading_line(level: usize, heading: &str) -> String {
    format!("{} {heading}", "#".repeat(level + 1))
}

/// The whole paper as prompt context: the title, then each section's heading
/// and its blocks separated by blank lines. Every block is included, never cut.
pub fn paper_context(canonical: &CanonicalText) -> String {
    let mut parts: Vec<String> = Vec::new();
    let title = text_of(canonical, canonical.title_span);
    if !title.is_empty() {
        parts.push(format!("# {title}"));
    }
    for section in &canonical.sections {
        let heading = text_of(canonical, section.heading_span);
        if !heading.is_empty() {
            parts.push(heading_line(section.level, heading));
        }
        for block in &section.blocks {
            parts.push(text_of(canonical, Some(block.span)).to_string());
        }
    }
    parts.join("\n\n")
}

/// The retrieved passages as prompt context: in document order, each under its
/// section's heading, separated by `\n\n---\n\n`.
pub fn passages_context(canonical: &CanonicalText, hits: &[ChunkHit]) -> String {
    let mut ordered: Vec<&ChunkHit> = hits.iter().collect();
    ordered.sort_by_key(|h| (h.span.start, h.span.end));
    ordered
        .iter()
        .map(|hit| {
            let passage = text_of(canonical, Some(hit.span));
            match canonical.sections.get(hit.section_index) {
                Some(section) => {
                    let heading = text_of(canonical, section.heading_span);
                    if heading.is_empty() {
                        passage.to_string()
                    } else {
                        format!("{}\n\n{passage}", heading_line(section.level, heading))
                    }
                }
                None => passage.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// The messages for one question: the system prompt, the paper context, each
/// `Complete` earlier turn as question and raw answer, then the question.
pub fn build_messages(context: &str, history: &[StoredTurn], question: &str) -> Vec<LLMMessage> {
    let mut messages = vec![
        LLMMessage::system(SYSTEM_PROMPT),
        LLMMessage::user(format!("The paper:\n\n{context}")),
    ];
    for turn in history.iter().filter(|t| t.status == TurnStatus::Complete) {
        messages.push(LLMMessage::user(turn.question.clone()));
        messages.push(LLMMessage::assistant(turn.answer.clone()));
    }
    messages.push(LLMMessage::user(question));
    messages
}

/// `WholePaper` when the whole-paper messages fit the window, else `Retrieval`.
pub fn choose_mode(window: Option<u64>, whole: &[LLMMessage]) -> Mode {
    if fits(window, whole) {
        Mode::WholePaper
    } else {
        Mode::Retrieval
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmlib::fulltext::canonical::canonical_from_markdown;

    /// A three-section paper.
    fn paper() -> CanonicalText {
        canonical_from_markdown(
            "# The Title\n\n## Methods\n\nAlpha para.\n\n## Results\n\nBeta para.\n\nGamma para.\n\n## Discussion\n\nDelta para.\n",
        )
    }

    /// A turn with the given status, question and answer.
    fn turn(status: TurnStatus, q: &str, a: &str) -> StoredTurn {
        StoredTurn {
            id: 1,
            question: q.into(),
            answer: a.into(),
            status,
            error: None,
            model: "m".into(),
            mode: Mode::WholePaper,
            passages: vec![],
            quotes: vec![],
            input_tokens: 0,
            output_tokens: 0,
            created_at: String::new(),
        }
    }

    /// The smaller known window wins; unknown ones are skipped.
    #[test]
    fn window_is_the_smaller() {
        assert_eq!(running_window(Some(8000), Some(128000)), Some(8000));
        assert_eq!(running_window(None, Some(128000)), Some(128000));
        assert_eq!(running_window(Some(8000), None), Some(8000));
        assert_eq!(running_window(None, None), None);
    }

    /// An unknown window never takes the whole paper.
    #[test]
    fn unknown_window_is_retrieval() {
        let msgs = build_messages("one line", &[], "q?");
        assert_eq!(choose_mode(None, &msgs), Mode::Retrieval);
        assert_eq!(choose_mode(Some(1_000_000), &msgs), Mode::WholePaper);
    }

    /// The edge of fit is exact and counts code points, not bytes.
    #[test]
    fn fit_counts_answer_room() {
        let window = 5000u64;
        let chars = ((window - ANSWER_TOKENS) * FIT_CHARS_PER_TOKEN) as usize;
        assert!(fits(Some(window), &[LLMMessage::user("a".repeat(chars))]));
        assert!(!fits(
            Some(window),
            &[LLMMessage::user("a".repeat(chars + 1))]
        ));
        assert!(fits(
            Some(window),
            &[LLMMessage::user("\u{1F600}".repeat(chars))]
        ));
        assert!(!fits(
            Some(window),
            &[LLMMessage::user("\u{1F600}".repeat(chars + 1))]
        ));
        assert_eq!(estimated_tokens(4), 2);
    }

    /// Every block appears, in order, after its heading.
    #[test]
    fn paper_context_is_whole() {
        let ctx = paper_context(&paper());
        let mut at = 0;
        for needle in [
            "The Title",
            "Methods",
            "Alpha para.",
            "Results",
            "Beta para.",
            "Gamma para.",
            "Discussion",
            "Delta para.",
        ] {
            at += ctx[at..]
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} missing"));
        }
        assert!(ctx.contains("### Methods\n\nAlpha para.\n\n"));
    }

    /// Best-first hits come out in document order, each under its heading.
    #[test]
    fn passages_in_document_order() {
        let c = paper();
        let hit = |heading: &str| {
            let section_index = c
                .sections
                .iter()
                .position(|s| c.span_text(s.heading_span.unwrap()) == Some(heading))
                .unwrap();
            ChunkHit {
                chunk_index: section_index,
                section_index,
                span: c.sections[section_index].blocks[0].span,
                score: 0.5,
            }
        };
        let out = passages_context(&c, &[hit("Results"), hit("Methods")]);
        assert!(out.find("Alpha para.").unwrap() < out.find("Beta para.").unwrap());
        assert!(out.contains("### Methods\n\nAlpha para."));
        assert!(out.contains("\n\n---\n\n### Results\n\nBeta para."));
    }

    /// Only Complete turns go back to the model.
    #[test]
    fn history_skips_incomplete() {
        let history = [
            turn(TurnStatus::Complete, "q1", "a1"),
            turn(TurnStatus::Stopped, "q2", "partial"),
        ];
        let msgs = build_messages("ctx", &history, "q3");
        assert_eq!(msgs.len(), 5);
        assert!(msgs
            .iter()
            .all(|m| m.content != "q2" && m.content != "partial"));
        assert_eq!(msgs[0].content, SYSTEM_PROMPT);
        assert_eq!(msgs[1].content, "The paper:\n\nctx");
        assert_eq!(msgs[4].content, "q3");
    }
}
