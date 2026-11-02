//! Every limit, default and name the core uses lives here (no magic numbers).

/// The product name shown in window titles and messages.
pub const APP_NAME: &str = "StudyAnalyzer";

/// The longest free-text or stored identifier accepted, in characters.
/// The check runs after surrounding whitespace is trimmed, so padding does not
/// count. Anything longer is refused before it is looked at, so a hostile
/// hand-off file cannot push an oversized string through the validators.
pub const MAX_IDENTIFIER_CHARS: usize = 256;

/// The prefix of a PMC ID, the PubMed Central archive's identifier.
pub const PMC_PREFIX: &str = "PMC";

/// The prefix of a preprint accession (PubMed's `PPR` numbers), followed by digits.
pub const PREPRINT_PREFIX: &str = "PPR";

/// The leading part of every DOI, the Crossref/DataCite namespace.
pub const DOI_NAMESPACE: &str = "10.";

/// The fewest digits a DOI registrant code may have.
pub const DOI_REGISTRANT_MIN_DIGITS: usize = 4;

/// The most digits a DOI registrant code may have.
pub const DOI_REGISTRANT_MAX_DIGITS: usize = 9;

/// The largest PDF accepted, in bytes (100 MB). Scanned papers are big; the
/// limit only stops absurd files before they are written to scratch space.
pub const MAX_PDF_BYTES: usize = 100 * 1024 * 1024;

/// The largest JATS, Markdown or plain-text file accepted, in bytes (20 MB).
pub const MAX_TEXT_BYTES: usize = 20 * 1024 * 1024;

/// The largest HTML file accepted, in bytes: the text cap plus 10% for markup
/// (22 MB), since a saved page may carry `data:` images. Size does not bound
/// the parse time, which is quadratic in nesting depth (bmlib #563).
pub const MAX_HTML_BYTES: usize = 22 * 1024 * 1024;

/// How many leading bytes of an HTML or XML file are searched for a charset
/// declaration (the HTML standard's prescan window).
pub const HTML_CHARSET_PRESCAN_BYTES: usize = 1024;

/// How many leading characters of a paste are looked at to tell XML or HTML
/// (enough for the longest marker, `<!doctype html`).
pub const PASTE_SNIFF_CHARS: usize = 16;

/// The magic bytes every PDF file starts with.
pub const PDF_MAGIC: &[u8] = b"%PDF-";

/// The file extension (lower case, no dot) that means PDF; a file named so
/// without [`PDF_MAGIC`] is refused as not a PDF.
pub const PDF_EXTENSION: &str = "pdf";

/// The file extensions (lower case, no dot) that mean JATS XML.
pub const JATS_EXTENSIONS: [&str; 2] = ["xml", "nxml"];

/// The file extensions that mean HTML.
pub const HTML_EXTENSIONS: [&str; 2] = ["html", "htm"];

/// The file extensions that mean Markdown.
pub const MARKDOWN_EXTENSIONS: [&str; 2] = ["md", "markdown"];

/// The file extension that means plain text.
pub const PLAIN_TEXT_EXTENSION: &str = "txt";

/// The file name of the SQLite database inside the app's data directory.
pub const DB_FILE_NAME: &str = "studyanalyzer.sqlite3";

/// The settings key under which the contact email for retrieval is stored.
pub const CONTACT_EMAIL_KEY: &str = "contact_email";

/// The settings key of the debugging switch that opens a document
/// contradicting its stored paper as a paper of its own instead of refusing it.
pub const KEEP_MISMATCHED_KEY: &str = "keep_mismatched";

/// What Settings calls that switch; error messages name it so the user can find it.
pub const KEEP_MISMATCHED_LABEL: &str = "Open a mismatching document as a paper of its own";

/// How long a connection waits for another connection's write lock before
/// failing; each command opens its own connection to the same WAL database.
pub const DB_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The title given to a paper when neither its text nor its sender names one.
pub const UNTITLED_PAPER: &str = "Untitled paper";

/// The reason shown for a paper without full text when nothing more is known.
pub const NO_FULL_TEXT_YET: &str = "No full text has been added to this paper yet.";

/// The notice when a document replaces a text the user had confirmed as the
/// whole paper with one that differs (another file, or a renamed one).
pub const REPLACED_CONFIRMED_NOTICE: &str = "This document differs from the text you had confirmed as the whole paper, so it is stored as a new text and needs confirming again.";

/// How a notice about the PDF engine's messages begins; a re-derive finds the
/// old one by it to replace it.
pub const ENGINE_NOTICE_LEAD: &str =
    "The PDF engine reported problems while reading this file, so some text may be missing";

/// Bytes per megabyte, for messages shown to the user.
pub const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// How many times a failing network call is tried in all (first try included).
pub const RETRIEVAL_ATTEMPTS: u32 = 3;

/// The wait before the first retry; it doubles after each failed attempt.
pub const RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// The longest a lookup by identifier may take in all, every attempt and
/// retry included (StudyAnalyzer #7). A healthy lookup takes seconds; this
/// stops a blackholed network from holding the UI for many minutes. It is
/// checked before every request and retry, so it can be overrun by under a
/// second of rounding plus one backoff wait already begun.
pub const RETRIEVAL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);

/// The lowest HTTP status that is a server error (5xx): a free PDF whose
/// server answered one may download on a retry.
pub const HTTP_SERVER_ERROR_MIN: u16 = 500;

/// HTTP client errors a retry may get past: 408 (request timeout) and 429
/// (too many requests). Any other 4xx refuses the PDF the same way again.
pub const HTTP_RETRYABLE_CLIENT_ERRORS: [u16; 2] = [408, 429];

/// bmlib's error for a request with no identifiers (service.rs, the start of `retrieve`).
pub const BMLIB_NO_IDENTIFIERS: &str = "No identifiers provided";

/// The `origin` stored for a source the user chose from a file.
pub const ORIGIN_UPLOAD: &str = "upload";

/// The `origin` stored for a source the user pasted as text.
pub const ORIGIN_PASTE: &str = "paste";

/// The mark that opens a quote in a model's answer.
pub const QUOTE_OPEN: &str = "<q>";

/// The mark that closes a quote in a model's answer.
pub const QUOTE_CLOSE: &str = "</q>";

/// The least similarity at which a quote counts as found in the paper.
///
/// PROVISIONAL, pending bmlib #632 (a token-level guard on numbers and
/// negations). `find_passage`'s similarity is a character ratio, `2M/(P+W)`,
/// so one changed character scores about `1 - 1/L` for a quote of `L`
/// characters: it cannot tell a changed digit, a dropped "not" or a dropped
/// small word from a typo. No value separates them, so this one is strict: in
/// practice a quote must equal the paper's text after normalisation (case,
/// whitespace, punctuation, dashes, accents; scores 1.0). A one-character
/// change in a quote longer than about 1000 characters can still pass. Near-
/// verbatim quotes with a dropped word lose their citation (visibly).
///
/// Measured with `tests/quote_threshold.rs` and `tests/fixtures/quotes.json`
/// (105 labelled cases on `article.xml` and the CC BY trial `real_rct.xml`;
/// see `tests/fixtures/README.md`), scores taken at a 0.5 floor. Lowest
/// accepted 1.0000; highest rejected 0.9978 (a dropped minus sign in a
/// 220-character sentence). Quotes from ollama `gemma4:31b-it-qat` and
/// `medgemma:27b-it-q8_0` scored 1.0 when verbatim; the 9 below 1.0 (0.70-0.99)
/// were all altered (merged text, inserted words, rewritten table cells).
pub const MIN_QUOTE_SIMILARITY: f64 = 0.999;

/// The characters one token is taken to span when checking a prompt against a
/// model's window. Low on purpose, so the estimate errs towards "too big".
pub const FIT_CHARS_PER_TOKEN: u64 = 3;

/// The room kept free for the model's answer, in tokens, when checking fit.
pub const ANSWER_TOKENS: u64 = 4096;

/// The temperature of an Interrogator answer: 0, so the same question gives
/// the same quotes.
pub const ANSWER_TEMPERATURE: f64 = 0.0;

/// How many passages a retrieval prompt carries.
pub const RETRIEVAL_PASSAGES: usize = 8;

/// The embedding model passages are found with.
pub const EMBEDDING_MODEL: bmlib::embed::ModelFile = bmlib::embed::ARCTIC_EMBED_L_V2_F16;

/// The folder under app storage the embedding model is kept in.
pub const EMBEDDER_SUBDIR: &str = "models";

/// The window a retrieval prompt is checked against when the model's window is
/// unknown and the user set none (Ollama's usual default, so a prompt Ollama
/// would cut is refused). It is never sent as `num_ctx` and never taken as the
/// model's maximum.
pub const UNKNOWN_WINDOW_FIT_TOKENS: u64 = 4096;

/// The setting holding the chosen `provider:model`.
pub const LLM_MODEL_KEY: &str = "llm_model";

/// The setting holding the user's context-window limit, in tokens.
pub const CONTEXT_TOKENS_KEY: &str = "context_tokens";

/// The OS keychain service the API keys are filed under (one entry per provider).
pub const KEYCHAIN_SERVICE: &str = "com.bmlibrarian.studyanalyzer";

#[cfg(test)]
mod tests {
    use super::*;

    /// The product name is the one the bundle and the UI use.
    #[test]
    fn app_name_is_studyanalyzer() {
        assert_eq!(APP_NAME, "StudyAnalyzer");
    }

    /// Settings shows the switch under the label error messages quote.
    #[test]
    fn settings_shows_the_keep_mismatched_label() {
        let settings = include_str!("../../../src/components/Settings.svelte");
        assert!(settings.contains(KEEP_MISMATCHED_LABEL));
    }
}
