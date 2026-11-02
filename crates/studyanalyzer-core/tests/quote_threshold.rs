//! Sets and guards `MIN_QUOTE_SIMILARITY` by measurement: hand-labelled quotes
//! (some invented, some collected from real models) against the fixture paper.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;

use bmlib::citations::passage_match::find_passage;
use bmlib::fulltext::models::TextBlock;
use bmlib::fulltext::pdf_text::{PdfError, PdfLimit, PdfTextExtractor};
use bmlib::llm::{ChatRequest, LlmClient};
use studyanalyzer_core::answer::{clean_quote, parse_answer, Piece};
use studyanalyzer_core::config::MIN_QUOTE_SIMILARITY;
use studyanalyzer_core::intake::{to_canonical, SourceKind};
use studyanalyzer_core::prompt::{build_messages, paper_context};

const JATS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/article.xml"
));
const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/quotes.json"
));

/// Seconds a collected answer may take (a local model can be slow).
const MODEL_TIMEOUT_SECONDS: u64 = 900;

/// Questions put to a real model by `collect_model_quotes`.
const QUESTIONS: [&str; 10] = [
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

/// A PDF backend that is never called: the fixture is JATS.
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

/// The fixture paper's canonical text.
fn paper() -> bmlib::fulltext::canonical::CanonicalText {
    to_canonical(
        SourceKind::Jats,
        JATS,
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
    quote: String,
    accept: bool,
    why: String,
}

/// The labelled cases from `quotes.json`.
fn cases() -> Vec<Case> {
    let v: serde_json::Value = serde_json::from_str(FIXTURE).expect("quotes.json parses");
    v["cases"]
        .as_array()
        .expect("cases is an array")
        .iter()
        .map(|c| Case {
            quote: c["quote"].as_str().expect("quote").into(),
            accept: c["accept"].as_bool().expect("accept"),
            why: c["why"].as_str().expect("why").into(),
        })
        .collect()
}

/// The similarity `find_passage` reports for a quote, as the app would ask it.
fn similarity(quote: &str, text: &str) -> f64 {
    find_passage(clean_quote(quote), text, MIN_QUOTE_SIMILARITY)
        .expect("threshold is valid")
        .similarity
}

/// Every labelled quote is accepted iff it is labelled `accept`.
#[test]
fn threshold_separates_labelled_quotes() {
    let text = paper().text;
    let all = cases();
    assert!(all.len() >= 40, "the fixture has at least 40 cases");
    let mut wrong = Vec::new();
    for c in &all {
        let m = find_passage(clean_quote(&c.quote), &text, MIN_QUOTE_SIMILARITY).unwrap();
        if m.matched() != c.accept {
            wrong.push(format!(
                "{:.4} want accept={} ({}): {}",
                m.similarity, c.accept, c.why, c.quote
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "threshold {MIN_QUOTE_SIMILARITY} misjudges:\n{}",
        wrong.join("\n")
    );
}

/// Prints the lowest accepted and highest rejected similarity (run with
/// `--include-ignored --nocapture`).
#[test]
#[ignore = "prints the margins that set MIN_QUOTE_SIMILARITY"]
fn print_margins() {
    let text = paper().text;
    let (mut low_accept, mut high_reject) = (f64::MAX, f64::MIN);
    for c in cases() {
        let s = similarity(&c.quote, &text);
        println!("{s:.4} accept={} {} | {}", c.accept, c.why, c.quote);
        if c.accept {
            low_accept = low_accept.min(s);
        } else {
            high_reject = high_reject.max(s);
        }
    }
    println!("lowest accepted {low_accept:.4}, highest rejected {high_reject:.4}");
}

/// Asks a real model the questions and prints each `<q>` quote with its
/// similarity, for hand-labelling. Needs `STUDYANALYZER_TEST_MODEL`
/// (e.g. `ollama:gemma4:31b-it-qat`) and, for a cloud model, its key.
#[test]
#[ignore = "calls a real model; needs STUDYANALYZER_TEST_MODEL"]
fn collect_model_quotes() {
    let model = std::env::var("STUDYANALYZER_TEST_MODEL").expect("STUDYANALYZER_TEST_MODEL");
    let canonical = paper();
    let context = paper_context(&canonical).unwrap();
    // A local model may think for minutes; the default timeout is for cloud calls.
    let http = bmlib::http::UreqClient {
        timeout_seconds: MODEL_TIMEOUT_SECONDS,
        ..bmlib::http::UreqClient::new()
    };
    let client = LlmClient::new(Arc::new(http));
    for question in QUESTIONS {
        let mut request =
            ChatRequest::new(build_messages(&context, &[], question)).with_model(model.clone());
        request.max_tokens = 2048;
        request.temperature = 0.0;
        request.num_ctx = Some(NonZeroU32::new(32768).unwrap());
        let answer = match client.chat(&request) {
            Ok(r) => r.content,
            Err(e) => {
                println!("Q: {question}\nERROR: {e}");
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
}
