//! Sets and guards `MIN_QUOTE_SIMILARITY` by measurement: hand-labelled quotes
//! (some invented, some collected from real models) against two fixture papers:
//! `article.xml` (tiny, templated) and `real_rct.xml` (a real CC BY trial).

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;

use bmlib::citations::passage_match::find_passage;
use bmlib::fulltext::canonical::CanonicalText;
use bmlib::fulltext::models::TextBlock;
use bmlib::fulltext::pdf_text::{PdfError, PdfLimit, PdfTextExtractor};
use bmlib::llm::{ChatRequest, LlmClient};
use studyanalyzer_core::answer::{clean_quote, parse_answer, Piece};
use studyanalyzer_core::config::MIN_QUOTE_SIMILARITY;
use studyanalyzer_core::intake::{to_canonical, SourceKind};
use studyanalyzer_core::prompt::{build_messages, paper_context};

const ARTICLE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/article.xml"
));
const REAL_RCT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/real_rct.xml"
));
const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/quotes.json"
));

/// Seconds a collected answer may take (a local model can be slow).
const MODEL_TIMEOUT_SECONDS: u64 = 900;

/// The most tokens a collected answer may use.
const COLLECT_MAX_TOKENS: i64 = 2048;

/// The context window asked of Ollama, so it does not cut the paper.
const COLLECT_NUM_CTX: u32 = 32768;

/// The similarity floor used to *measure*. `find_passage` prunes its search by
/// the threshold it is given, so measuring at the real threshold deflates (or
/// zeroes) the scores of rejected quotes; a low floor reports their true score.
const MEASURE_FLOOR: f64 = 0.5;

/// Questions put to a real model about `article.xml`.
const ARTICLE_QUESTIONS: [&str; 10] = [
    "Who took part in the trial, and how were they assigned to treatment?",
    "What was the intervention and what was it compared with?",
    "What was the main result? Give the number.",
    "How long was each participant observed, and who recorded the outcome?",
    "Was the outcome assessor blinded?",
    "How many participant groups were observed in the Methods?",
    "Were there any subgroup results?",
    "What limitations do the authors name?",
    "What does the Discussion say about the findings?",
    "What was the effect on mortality or hospital admission?",
];

/// Questions put to a real model about `real_rct.xml`.
const REAL_QUESTIONS: [&str; 10] = [
    "How many patients were enrolled, and what were they undergoing?",
    "What were the two treatment groups?",
    "What was the primary outcome and what was its result? Give the numbers and the confidence interval.",
    "What was the opioid consumption at 24 and 48 hours in each group?",
    "How many patients needed rescue analgesia in each group?",
    "Who was blinded in the trial?",
    "What statistical tests were used to compare the groups?",
    "What limitations do the authors name?",
    "Were there any complications?",
    "What was the effect on length of hospital stay?",
];

/// A PDF backend that is never called: the fixtures are JATS.
struct NoPdf;

impl PdfTextExtractor for NoPdf {
    fn name(&self) -> &'static str {
        "none"
    }
    fn extract_blocks_within(&self, _: &Path, _: &PdfLimit) -> Result<Vec<TextBlock>, PdfError> {
        Err(PdfError::unreadable("not used"))
    }
    fn page_texts_within(&self, _: &Path, _: &PdfLimit) -> Result<Vec<String>, PdfError> {
        Err(PdfError::unreadable("not used"))
    }
}

/// The canonical text of the named fixture paper (`article.xml` or `real_rct.xml`).
fn paper(name: &str) -> CanonicalText {
    let bytes = match name {
        "article.xml" => ARTICLE,
        "real_rct.xml" => REAL_RCT,
        other => panic!("unknown fixture paper {other}"),
    };
    to_canonical(
        SourceKind::Jats,
        bytes,
        None,
        &NoPdf,
        &std::env::temp_dir(),
        &PdfLimit::default(),
    )
    .expect("fixture converts")
    .0
}

/// One labelled quote.
struct Case {
    paper: String,
    quote: String,
    /// The true label: are these the paper's own words, near-verbatim?
    accept: bool,
    /// What the threshold is known to decide: `accept` unless the fixture
    /// names a true accept it rejects (one scoring below a reject, or one the
    /// guard refuses against a neighbouring sentence).
    expected: bool,
    why: String,
}

/// The labelled cases from `quotes.json`. Panics on an `expected` that is not
/// `false` on a true accept: the override may only reject a true quote, never
/// let a wrong one be cited.
fn cases() -> Vec<Case> {
    let v: serde_json::Value = serde_json::from_str(FIXTURE).expect("quotes.json parses");
    v["cases"]
        .as_array()
        .expect("cases is an array")
        .iter()
        .map(|c| {
            let accept = c["accept"].as_bool().expect("accept");
            let quote: String = c["quote"].as_str().expect("quote").into();
            let expected = c
                .get("expected")
                .map(|e| e.as_bool().expect("expected is a boolean"));
            assert!(
                expected.is_none_or(|e| accept && !e),
                "`expected` may only be false on a true accept: {quote}"
            );
            Case {
                paper: c["paper"].as_str().expect("paper").into(),
                quote,
                accept,
                expected: expected.unwrap_or(accept),
                why: c["why"].as_str().expect("why").into(),
            }
        })
        .collect()
}

/// The true similarity `find_passage` gives a quote (measured at the floor).
fn similarity(quote: &str, text: &str) -> f64 {
    find_passage(clean_quote(quote), text, MEASURE_FLOOR)
        .expect("floor is valid")
        .similarity
}

/// The text of every paper named by the cases.
fn texts() -> HashMap<String, String> {
    ["article.xml", "real_rct.xml"]
        .into_iter()
        .map(|n| (n.to_string(), paper(n).text))
        .collect()
}

/// Every labelled quote is accepted iff it is `expected` to be (its `accept`
/// label unless the fixture names a true accept the threshold rejects).
#[test]
fn threshold_separates_labelled_quotes() {
    let texts = texts();
    let all = cases();
    assert!(all.len() >= 40, "the fixture has at least 40 cases");
    let mut wrong = Vec::new();
    for c in &all {
        let text = &texts[&c.paper];
        let m = find_passage(clean_quote(&c.quote), text, MIN_QUOTE_SIMILARITY).unwrap();
        if m.matched() != c.expected {
            wrong.push(format!(
                "{:.4} want matched={} ({}, {}): {}",
                similarity(&c.quote, text),
                c.expected,
                c.paper,
                c.why,
                c.quote
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "threshold {MIN_QUOTE_SIMILARITY} misjudges:\n{}",
        wrong.join("\n")
    );
}

/// Prints every case's score at the floor and the contradiction bmlib's guard
/// names (bmlib #632), then the lowest similarity expected to match and the
/// highest true reject (`accept: false`) that the guard lets through: a reject
/// the guard catches does not bound the threshold, and the `expected: false`
/// accepts are left out, as their labels come from the threshold itself (run
/// with `--include-ignored --nocapture`).
#[test]
#[ignore = "prints the margins that set MIN_QUOTE_SIMILARITY"]
fn print_margins() {
    let texts = texts();
    let (mut low_accept, mut high_reject) = (f64::MAX, f64::MIN);
    for c in cases() {
        let m = find_passage(clean_quote(&c.quote), &texts[&c.paper], MEASURE_FLOOR)
            .expect("floor is valid");
        let s = m.similarity;
        let guard = m.contradiction.map_or("-", |k| k.as_str());
        let head: String = c.quote.chars().take(60).collect();
        println!(
            "{s:.4} accept={} expected={} guard={guard} [{}] {} | {head}",
            c.accept, c.expected, c.paper, c.why
        );
        if c.expected {
            low_accept = low_accept.min(s);
        }
        if !c.accept && m.contradiction.is_none() {
            high_reject = high_reject.max(s);
        }
    }
    println!(
        "lowest expected match {low_accept:.4}, highest unguarded true reject {high_reject:.4}"
    );
}

/// Asks a real model the questions about `STUDYANALYZER_TEST_PAPER` (default
/// `real_rct.xml`) and prints each `<q>` quote with its similarity at the
/// floor, for hand-labelling. Needs `STUDYANALYZER_TEST_MODEL` (e.g.
/// `ollama:gemma4:31b-it-qat`) and, for a cloud model, its key. Fails if any
/// call failed.
#[test]
#[ignore = "calls a real model; needs STUDYANALYZER_TEST_MODEL"]
fn collect_model_quotes() {
    let model = std::env::var("STUDYANALYZER_TEST_MODEL").expect("STUDYANALYZER_TEST_MODEL");
    let name = std::env::var("STUDYANALYZER_TEST_PAPER").unwrap_or_else(|_| "real_rct.xml".into());
    let questions = if name == "article.xml" {
        ARTICLE_QUESTIONS
    } else {
        REAL_QUESTIONS
    };
    let canonical = paper(&name);
    let context = paper_context(&canonical).unwrap();
    // A local model may think for minutes; the default timeout is for cloud calls.
    let http = bmlib::http::UreqClient {
        timeout_seconds: MODEL_TIMEOUT_SECONDS,
        ..bmlib::http::UreqClient::new()
    };
    let client = LlmClient::new(Arc::new(http));
    let mut failed = 0;
    for question in questions {
        let mut request =
            ChatRequest::new(build_messages(&context, &[], question)).with_model(model.clone());
        request.max_tokens = COLLECT_MAX_TOKENS;
        request.temperature = 0.0;
        request.num_ctx = Some(NonZeroU32::new(COLLECT_NUM_CTX).unwrap());
        let answer = match client.chat(&request) {
            Ok(r) => r.content,
            Err(e) => {
                println!("Q: {question}\nERROR: {e}");
                failed += 1;
                continue;
            }
        };
        println!("Q: {question}\nA: {answer}");
        for piece in parse_answer(&answer) {
            if let Piece::Quote { text, .. } = piece {
                println!("QUOTE {:.4} | {}", similarity(&text, &canonical.text), text);
            }
        }
    }
    assert_eq!(failed, 0, "{failed} model calls failed");
}
