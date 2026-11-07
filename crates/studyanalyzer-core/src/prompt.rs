//! The Interrogator's prompt: which mode fits the model's window, the paper (or
//! its best passages) as context, and the messages sent.

use bmlib::embed::store::ChunkHit;
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::llm::LLMMessage;

use crate::config::{
    FIT_CHARS_PER_TOKEN, MAX_REPLY_TOKENS, MIN_REPLY_TOKENS, REPLY_WINDOW_SHARE,
    UNCAPPED_REPLY_PROVIDERS,
};
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

/// The room for a reply, in tokens: the request's `max_tokens` and what is kept
/// free of the window for it. A thinking model's reasoning counts against it,
/// so a large window gives more: 1/[`REPLY_WINDOW_SHARE`] of `window`, between
/// [`MIN_REPLY_TOKENS`] and [`MAX_REPLY_TOKENS`].
///
/// It never exceeds `max_output`, the model's own output limit (bmlib's
/// `ModelLimits::max_output`), since a provider refuses a larger request.
/// Unstated, the room stays at [`MIN_REPLY_TOKENS`], except for a provider in
/// [`UNCAPPED_REPLY_PROVIDERS`] (`provider` is the part of `provider:model`
/// before the colon).
pub fn reply_tokens(provider: &str, window: Option<u64>, max_output: Option<u64>) -> u64 {
    let cap = match max_output {
        Some(cap) => cap,
        None if UNCAPPED_REPLY_PROVIDERS.contains(&provider) => u64::MAX,
        None => return MIN_REPLY_TOKENS,
    };
    window
        .map_or(MIN_REPLY_TOKENS, |w| w / REPLY_WINDOW_SHARE)
        .clamp(MIN_REPLY_TOKENS, MAX_REPLY_TOKENS)
        .min(cap)
}

/// The tokens `chars` code points are estimated to take: `ceil(chars / FIT_CHARS_PER_TOKEN)`.
pub fn estimated_tokens(chars: usize) -> u64 {
    (chars as u64).div_ceil(FIT_CHARS_PER_TOKEN)
}

/// Whether `messages` plus `reply` tokens of room for the answer fit in
/// `window` tokens. Counts code points, not bytes. An unknown window (`None`)
/// never fits.
pub fn fits(window: Option<u64>, reply: u64, messages: &[LLMMessage]) -> bool {
    let Some(window) = window else {
        return false;
    };
    let chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    estimated_tokens(chars).saturating_add(reply) <= window
}

/// Why a prompt could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptError {
    /// A span (or section index) does not lie in the paper; the message says which.
    Span(String),
}

impl std::fmt::Display for PromptError {
    /// Writes the message of the error.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptError::Span(m) => write!(f, "the paper has no text at {m}"),
        }
    }
}

impl std::error::Error for PromptError {}

/// The text of `span` for `what`; a span the paper refuses is an error naming it.
fn text_of<'a>(
    canonical: &'a CanonicalText,
    span: TextSpan,
    what: &str,
) -> Result<&'a str, PromptError> {
    canonical
        .span_text(span)
        .ok_or_else(|| PromptError::Span(format!("{what} ({}..{})", span.start, span.end)))
}

/// A section heading as markdown: `#` repeated `level + 1` times, then the text.
fn heading_line(level: usize, heading: &str) -> String {
    format!("{} {heading}", "#".repeat(level + 1))
}

/// The whole paper as prompt context: the title, then each section's heading
/// and its blocks separated by blank lines. Every block is included, never cut.
/// A missing title or heading emits nothing; a span outside the paper is an
/// error for the caller to log and report, never a silently dropped block.
pub fn paper_context(canonical: &CanonicalText) -> Result<String, PromptError> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(span) = canonical.title_span {
        parts.push(format!("# {}", text_of(canonical, span, "the title")?));
    }
    for (i, section) in canonical.sections.iter().enumerate() {
        if let Some(span) = section.heading_span {
            let heading = text_of(canonical, span, &format!("the heading of section {i}"))?;
            parts.push(heading_line(section.level, heading));
        }
        for (j, block) in section.blocks.iter().enumerate() {
            let what = format!("block {j} of section {i}");
            parts.push(text_of(canonical, block.span, &what)?.to_string());
        }
    }
    Ok(parts.join("\n\n"))
}

/// The retrieved passages as prompt context: in document order, each under its
/// section's heading, separated by `\n\n---\n\n`. A hit whose span lies outside
/// the paper, or whose section does not exist, is an error for the caller to
/// log and report; an untitled section gives a passage with no heading.
pub fn passages_context(
    canonical: &CanonicalText,
    hits: &[ChunkHit],
) -> Result<String, PromptError> {
    let mut ordered: Vec<&ChunkHit> = hits.iter().collect();
    ordered.sort_by_key(|h| (h.span.start, h.span.end));
    let mut out: Vec<String> = Vec::new();
    for hit in ordered {
        let passage = text_of(canonical, hit.span, &format!("chunk {}", hit.chunk_index))?;
        let section = canonical.sections.get(hit.section_index).ok_or_else(|| {
            PromptError::Span(format!(
                "section {} of chunk {} (the paper has {} sections)",
                hit.section_index,
                hit.chunk_index,
                canonical.sections.len()
            ))
        })?;
        out.push(match section.heading_span {
            Some(span) => {
                let what = format!("the heading of section {}", hit.section_index);
                let heading = text_of(canonical, span, &what)?;
                format!("{}\n\n{passage}", heading_line(section.level, heading))
            }
            None => passage.to_string(),
        });
    }
    Ok(out.join("\n\n---\n\n"))
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

/// `WholePaper` when the whole-paper messages, with `reply` tokens of room,
/// fit the window, else `Retrieval`.
pub fn choose_mode(window: Option<u64>, reply: u64, whole: &[LLMMessage]) -> Mode {
    if fits(window, reply, whole) {
        Mode::WholePaper
    } else {
        Mode::Retrieval
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmlib::fulltext::canonical::{canonical_from_markdown, canonical_from_plain_text};
    use bmlib::llm::Role;

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
        assert_eq!(choose_mode(None, MIN_REPLY_TOKENS, &msgs), Mode::Retrieval);
        assert_eq!(
            choose_mode(Some(1_000_000), MIN_REPLY_TOKENS, &msgs),
            Mode::WholePaper
        );
    }

    /// A reply gets a quarter of the window, from the least to the most room,
    /// never above the model's own limit; unstated, the least, except on a
    /// provider without limits.
    #[test]
    fn reply_room_follows_the_window_under_the_limit() {
        let big = Some(1_048_576);
        assert_eq!(
            reply_tokens("deepseek", big, Some(393_216)),
            MAX_REPLY_TOKENS
        );
        assert_eq!(
            reply_tokens("deepseek", Some(65_536), Some(393_216)),
            16_384
        );
        assert_eq!(
            reply_tokens("deepseek", Some(8192), Some(393_216)),
            MIN_REPLY_TOKENS
        );
        assert_eq!(
            reply_tokens("deepseek", None, Some(393_216)),
            MIN_REPLY_TOKENS
        );
        assert_eq!(reply_tokens("anthropic", big, Some(8192)), 8192);
        assert_eq!(reply_tokens("anthropic", big, Some(2048)), 2048);
        assert_eq!(reply_tokens("openai", big, None), MIN_REPLY_TOKENS);
        assert_eq!(reply_tokens("ollama", Some(131_072), None), 32_768);
        assert_eq!(reply_tokens("ollama", Some(40_000), None), 10_000);
        assert_eq!(reply_tokens("ollama", None, None), MIN_REPLY_TOKENS);
        // The least room even above a tiny window: the fit check then refuses.
        assert_eq!(reply_tokens("ollama", Some(2048), None), MIN_REPLY_TOKENS);
    }

    /// The edge of fit is exact and counts code points, not bytes.
    #[test]
    fn fit_counts_answer_room() {
        let window = 5000u64;
        let reply = 1000u64;
        let chars = ((window - reply) * FIT_CHARS_PER_TOKEN) as usize;
        let fit = |text: String| fits(Some(window), reply, &[LLMMessage::user(text)]);
        assert!(fit("a".repeat(chars)));
        assert!(!fit("a".repeat(chars + 1)));
        assert!(fit("\u{1F600}".repeat(chars)));
        assert!(!fit("\u{1F600}".repeat(chars + 1)));
        assert!(!fits(Some(window), u64::MAX, &[]));
        assert_eq!(estimated_tokens(4), 2);
    }

    /// Every block appears, in order, after its heading.
    #[test]
    fn paper_context_is_whole() {
        let ctx = paper_context(&paper()).unwrap();
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
        let out = passages_context(&c, &[hit("Results"), hit("Methods")]).unwrap();
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
        assert_eq!((msgs[2].role, msgs[2].content.as_str()), (Role::User, "q1"));
        assert_eq!(
            (msgs[3].role, msgs[3].content.as_str()),
            (Role::Assistant, "a1")
        );
    }

    /// A hit for the first section with the given span.
    fn raw_hit(section_index: usize, span: TextSpan) -> ChunkHit {
        ChunkHit {
            chunk_index: 0,
            section_index,
            span,
            score: 0.5,
        }
    }

    /// A section index past the sections is an error, not a lost heading.
    #[test]
    fn hit_with_bad_section_is_refused() {
        let c = paper();
        let span = c.sections[0].blocks[0].span;
        let r = passages_context(&c, &[raw_hit(99, span)]);
        assert!(matches!(r, Err(PromptError::Span(_))));
    }

    /// A hit outside the text is an error, not an empty passage.
    #[test]
    fn hit_outside_text_is_refused() {
        let c = paper();
        let span = TextSpan {
            start: 0,
            end: usize::MAX / 2,
        };
        let r = passages_context(&c, &[raw_hit(0, span)]);
        assert!(matches!(r, Err(PromptError::Span(_))));
    }

    /// No hits, no context.
    #[test]
    fn no_hits_is_empty() {
        assert_eq!(passages_context(&paper(), &[]).unwrap(), "");
    }

    /// A block outside the text is an error, not a dropped block.
    #[test]
    fn block_outside_text_is_refused() {
        let mut c = paper();
        c.sections[0].blocks[0].span = TextSpan {
            start: 0,
            end: usize::MAX / 2,
        };
        assert!(matches!(paper_context(&c), Err(PromptError::Span(_))));
    }

    /// No title and an untitled section are legitimate: blocks, no heading.
    #[test]
    fn untitled_paper_has_no_heading_lines() {
        let c = canonical_from_plain_text("Just a paragraph.", "");
        let ctx = paper_context(&c).unwrap();
        assert!(ctx.contains("Just a paragraph."));
        assert!(!ctx.contains('#'));
    }
}
