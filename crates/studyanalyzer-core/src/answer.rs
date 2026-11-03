//! Quote marks in a model's answer: split them out, find each in the paper.
//!
//! The model wraps every quote in `<q>…</q>`. Each closed quote is located
//! with bmlib's `find_passage`; what is stored and highlighted is the paper's
//! own span, never the model's copy.

use crate::config::{QUOTE_CLOSE, QUOTE_OPEN};
use crate::conversations::NewQuote;
use bmlib::citations::passage_match::{find_passage, InvalidSimilarity};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use serde::Serialize;

/// Characters a model wraps around a quote that are not part of the paper's text.
const QUOTE_MARKS: &str = "\"'“”‘’«»…";

/// One part of a raw answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    /// Plain text between quotes.
    Text(String),
    /// A quote; `closed` is false when the answer ended before `</q>`.
    Quote {
        /// The text between the marks.
        text: String,
        /// Whether the closing mark was seen.
        closed: bool,
    },
}

/// One part of an answer as shown to the user.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerPiece {
    /// Plain text.
    Text {
        /// The text.
        text: String,
    },
    /// A citation marker for a located quote.
    Cite {
        /// The citation number.
        number: u32,
    },
}

/// The result of locating an answer's quotes in the paper.
#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    /// The answer to show: text and numbered citations.
    pub pieces: Vec<AnswerPiece>,
    /// Every quote of the answer, in order, located or not.
    pub quotes: Vec<NewQuote>,
}

/// Appends `text` to `out`, merging with a text piece already last.
fn push_text(out: &mut Vec<AnswerPiece>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(AnswerPiece::Text { text: last }) = out.last_mut() {
        last.push_str(text);
    } else {
        out.push(AnswerPiece::Text { text: text.into() });
    }
}

/// Splits a raw answer on `<q>` / `</q>` (case-sensitive, no nesting).
///
/// A `</q>` outside a quote is text; a quote left open at the end is
/// `closed: false`.
pub fn parse_answer(content: &str) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut rest = content;
    let push = |pieces: &mut Vec<Piece>, t: &str| {
        if t.is_empty() {
            return;
        }
        if let Some(Piece::Text(last)) = pieces.last_mut() {
            last.push_str(t);
        } else {
            pieces.push(Piece::Text(t.into()));
        }
    };
    while let Some(i) = rest.find(QUOTE_OPEN) {
        push(&mut pieces, &rest[..i]);
        rest = &rest[i + QUOTE_OPEN.len()..];
        match rest.find(QUOTE_CLOSE) {
            Some(j) => {
                pieces.push(Piece::Quote {
                    text: rest[..j].into(),
                    closed: true,
                });
                rest = &rest[j + QUOTE_CLOSE.len()..];
            }
            None => {
                pieces.push(Piece::Quote {
                    text: rest.into(),
                    closed: false,
                });
                rest = "";
            }
        }
    }
    push(&mut pieces, rest);
    pieces
}

/// Trims whitespace, then quotation marks and `…` / `...` at either end.
pub fn clean_quote(quote: &str) -> &str {
    let mut s = quote.trim();
    loop {
        let t = s
            .trim_matches(|c| QUOTE_MARKS.contains(c))
            .trim()
            .trim_start_matches("...")
            .trim_end_matches("...")
            .trim();
        if t == s {
            return s;
        }
        s = t;
    }
}

/// Finds each closed quote in the paper and numbers the ones found.
///
/// An unclosed quote, an empty one, or one `find_passage` rejects keeps its
/// place in `quotes` with `number: None` and gets no piece in `pieces`.
///
/// # Errors
/// Passes on `find_passage`'s [`InvalidSimilarity`] for a bad `min_similarity`.
pub fn locate(
    pieces: &[Piece],
    canonical: &CanonicalText,
    min_similarity: f64,
) -> Result<Located, InvalidSimilarity> {
    let mut out = Vec::new();
    let mut quotes = Vec::new();
    let mut next = 1u32;
    for piece in pieces {
        match piece {
            Piece::Text(t) => push_text(&mut out, t),
            Piece::Quote { text, closed } => {
                let cleaned = clean_quote(text);
                let m = if *closed && !cleaned.is_empty() {
                    Some(find_passage(cleaned, &canonical.text, min_similarity)?)
                } else {
                    None
                };
                // A rejected match still reports how close it came, and why the
                // guard refused it if it did.
                let similarity = m.as_ref().map_or(0.0, |m| m.similarity);
                let contradiction = m
                    .as_ref()
                    .and_then(|m| m.contradiction)
                    .map(|k| k.as_str().to_string());
                match m.and_then(|m| m.span) {
                    Some(s) => {
                        out.push(AnswerPiece::Cite { number: next });
                        quotes.push(NewQuote {
                            number: Some(next),
                            quote: text.clone(),
                            span: Some(TextSpan {
                                start: s.start,
                                end: s.end,
                            }),
                            similarity,
                            contradiction: None,
                        });
                        next += 1;
                    }
                    None => quotes.push(NewQuote {
                        number: None,
                        quote: text.clone(),
                        span: None,
                        similarity,
                        contradiction,
                    }),
                }
            }
        }
    }
    Ok(Located {
        pieces: out,
        quotes,
    })
}

/// Rebuilds the pieces of a stored turn from its raw answer and stored quotes.
///
/// Quote pieces map to `quotes` by position. A quote with no stored quote at
/// its position (a stopped or failed turn stores none) becomes text in curly
/// quotes, so nothing the model wrote is lost.
pub fn answer_pieces(answer: &str, quotes: &[NewQuote]) -> Vec<AnswerPiece> {
    let mut out = Vec::new();
    let mut k = 0;
    for piece in parse_answer(answer) {
        match piece {
            Piece::Text(t) => push_text(&mut out, &t),
            Piece::Quote { text, .. } => {
                match quotes.get(k) {
                    Some(q) => {
                        if let Some(number) = q.number {
                            out.push(AnswerPiece::Cite { number });
                        }
                    }
                    None => push_text(&mut out, &format!("“{text}”")),
                }
                k += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MIN_QUOTE_SIMILARITY;
    use bmlib::fulltext::canonical::canonical_from_markdown;

    /// A small paper with the two sentences the tests quote.
    fn paper() -> CanonicalText {
        canonical_from_markdown(
            "# Trial\n\n## Results\n\nOf 412 patients, 61 were over 75 years.\n\nThe hazard ratio was 0.95.\n",
        )
    }

    /// A closed quote piece.
    fn q(t: &str) -> Piece {
        Piece::Quote {
            text: t.into(),
            closed: true,
        }
    }

    /// Quote marks split the answer; text around them is kept.
    #[test]
    fn parses_quotes() {
        assert_eq!(
            parse_answer("A <q>x</q> b <q>y</q>."),
            vec![
                Piece::Text("A ".into()),
                q("x"),
                Piece::Text(" b ".into()),
                q("y"),
                Piece::Text(".".into())
            ]
        );
    }

    /// A closing mark with nothing open is ordinary text.
    #[test]
    fn stray_close_is_text() {
        assert_eq!(
            parse_answer("a </q> b"),
            vec![Piece::Text("a </q> b".into())]
        );
    }

    /// An answer cut off inside a quote marks it unclosed.
    #[test]
    fn unclosed_quote_is_marked() {
        let p = parse_answer("A <q>par");
        assert_eq!(
            p.last(),
            Some(&Piece::Quote {
                text: "par".into(),
                closed: false
            })
        );
    }

    /// A `<q>` inside a quote belongs to the quote.
    #[test]
    fn no_nesting() {
        assert_eq!(parse_answer("<q>a <q>b</q>"), vec![q("a <q>b")]);
    }

    /// Marks and ellipses around a quote are removed, inner dots kept.
    #[test]
    fn clean_strips_marks() {
        assert_eq!(
            clean_quote(" “…61 were over 75 years.” "),
            "61 were over 75 years."
        );
        assert_eq!(clean_quote("\"...x...\""), "x");
        assert_eq!(clean_quote(" “…” "), "");
    }

    /// The stored span is the paper's wording, not the model's copy.
    #[test]
    fn located_text_is_the_papers() {
        let c = paper();
        let l = locate(&[q("of 412 patients, 61 were over 75 years")], &c, 0.95).unwrap();
        let span = l.quotes[0].span.unwrap();
        let shown = c.span_text(span).unwrap();
        assert!(shown.starts_with("Of 412 patients"), "{shown}");
        assert_ne!(shown, l.quotes[0].quote);
        assert_eq!(l.pieces, vec![AnswerPiece::Cite { number: 1 }]);
    }

    /// A quote that changes a number never cites: it is unlocated at the
    /// app's threshold and, through bmlib's guard (#632), at a loose one too.
    /// Only a match that reached the threshold is checked, so only the loose
    /// one says why.
    #[test]
    fn changed_number_is_refused_and_says_why() {
        let c = paper();
        let quote = [q("The hazard ratio was 0.59.")];
        for (threshold, why) in [(MIN_QUOTE_SIMILARITY, None), (0.8, Some("number"))] {
            let l = locate(&quote, &c, threshold).unwrap();
            assert_eq!(l.quotes[0].number, None, "at {threshold}");
            assert_eq!(l.quotes[0].span, None, "at {threshold}");
            assert_eq!(l.quotes[0].contradiction.as_deref(), why, "at {threshold}");
            assert!(l.pieces.is_empty());
        }
    }

    /// A located quote and a quote nothing came close to carry no contradiction.
    #[test]
    fn no_contradiction_without_a_refused_match() {
        let c = paper();
        let l = locate(
            &parse_answer(
                "<q>of 412 patients, 61 were over 75 years</q> <q>a quarter were elderly</q>",
            ),
            &c,
            MIN_QUOTE_SIMILARITY,
        )
        .unwrap();
        assert_eq!(l.quotes[0].number, Some(1));
        assert_eq!(l.quotes[0].contradiction, None);
        assert_eq!(l.quotes[1].number, None);
        assert_eq!(l.quotes[1].contradiction, None);
    }

    /// A paraphrase is not located: no citation, but the quote is kept.
    #[test]
    fn paraphrase_unlocated() {
        let l = locate(
            &parse_answer("<q>a quarter of the patients were elderly</q>"),
            &paper(),
            0.95,
        )
        .unwrap();
        assert_eq!(l.quotes[0].number, None);
        assert_eq!(l.quotes[0].span, None);
        assert!(l.pieces.is_empty());
    }

    /// A near miss is rejected but keeps the similarity `find_passage` reported.
    #[test]
    fn rejected_quote_keeps_its_similarity() {
        let c = paper();
        let l = locate(
            &[q("Of 412 patients, 61 were over 80 years of age")],
            &c,
            0.95,
        )
        .unwrap();
        let sim = l.quotes[0].similarity;
        assert_eq!(l.quotes[0].number, None);
        assert!(sim > 0.0 && sim < MIN_QUOTE_SIMILARITY, "{sim}");
    }

    /// Numbers count located quotes only; unlocated ones keep their place.
    #[test]
    fn numbers_in_order() {
        let p = parse_answer(
            "<q>Of 412 patients, 61 were over 75 years.</q> and <q>nonsense that is absent</q> and <q>The hazard ratio was 0.95.</q>",
        );
        let l = locate(&p, &paper(), 0.95).unwrap();
        let n: Vec<_> = l.quotes.iter().map(|q| q.number).collect();
        assert_eq!(n, vec![Some(1), None, Some(2)]);
    }

    /// A bad similarity is reported, not unwrapped.
    #[test]
    fn bad_similarity_is_an_error() {
        assert!(locate(&[q("x")], &paper(), 2.0).is_err());
    }

    /// An unclosed or empty quote is never located.
    #[test]
    fn unclosed_and_empty_unlocated() {
        let p = parse_answer("<q>Of 412 patients, 61 were over 75 years.");
        assert_eq!(locate(&p, &paper(), 0.95).unwrap().quotes[0].number, None);
        let l = locate(&[q(" “…” ")], &paper(), 0.95).unwrap();
        assert_eq!(l.quotes[0].number, None);
    }

    /// A stored turn rebuilds to the pieces shown live.
    #[test]
    fn pieces_rebuild() {
        let raw = "See <q>Of 412 patients, 61 were over 75 years.</q> but <q>made up words</q> and <q>The hazard ratio was 0.95.</q> ok";
        let l = locate(&parse_answer(raw), &paper(), 0.95).unwrap();
        assert_eq!(answer_pieces(raw, &l.quotes), l.pieces);
    }

    /// Quotes with nothing stored come back as curly-quoted text, merged.
    #[test]
    fn pieces_without_stored_quotes() {
        assert_eq!(
            answer_pieces("A <q>x</q> b <q>y", &[]),
            vec![AnswerPiece::Text {
                text: "A “x” b “y”".into()
            }]
        );
    }

    /// Empty input has no pieces.
    #[test]
    fn empty_input_has_no_pieces() {
        assert!(parse_answer("").is_empty());
    }

    /// An empty quote is a quote piece and is never located.
    #[test]
    fn empty_quote_is_unlocated() {
        let p = parse_answer("<q></q>");
        assert_eq!(p, vec![q("")]);
        let l = locate(&p, &paper(), 0.95).unwrap();
        assert_eq!(l.quotes[0].number, None);
        assert!(l.pieces.is_empty());
    }

    /// Back-to-back quotes have no empty text between them.
    #[test]
    fn adjacent_quotes_have_no_text_between() {
        assert_eq!(parse_answer("<q>a</q><q>b</q>"), vec![q("a"), q("b")]);
    }

    /// Multi-byte characters beside the tags survive intact.
    #[test]
    fn multibyte_next_to_tags() {
        assert_eq!(
            parse_answer("é<q>é</q>é"),
            vec![Piece::Text("é".into()), q("é"), Piece::Text("é".into())]
        );
    }
}
