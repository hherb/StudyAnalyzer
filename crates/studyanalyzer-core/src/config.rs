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

/// Whether a paper whose full text is JATS also has its PDF fetched, for the
/// *Original* tab: the JATS is what gets processed, the PDF what gets shown
/// (bmlib's `fetch_pdf_with_jats`, #739). Costs a few extra requests and up to
/// one PDF download per JATS paper looked up (a cached JATS hit is served as
/// it is).
pub const FETCH_PDF_WITH_JATS: bool = true;

/// The largest JATS, Markdown or plain-text file accepted, in bytes (20 MB).
pub const MAX_TEXT_BYTES: usize = 20 * 1024 * 1024;

/// The largest HTML file accepted, in bytes: the text cap plus 10% for markup
/// (22 MB), since a saved page may carry `data:` images. bmlib caps what its
/// parse time grows with, not the page's length: a page holding more than
/// `HTML_MAX_TAGS` `<` (bmlib #656), one whose parse builds too many elements
/// (#723), or one still parsing after `HTML_PARSE_SECONDS` (#563) is refused,
/// so a page under this cap can still be refused, saying why.
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

/// Every file extension the file dialog offers as a paper (the formats above).
pub const PAPER_FILE_EXTENSIONS: [&str; 8] = [
    PDF_EXTENSION,
    JATS_EXTENSIONS[0],
    JATS_EXTENSIONS[1],
    HTML_EXTENSIONS[0],
    HTML_EXTENSIONS[1],
    MARKDOWN_EXTENSIONS[0],
    MARKDOWN_EXTENSIONS[1],
    PLAIN_TEXT_EXTENSION,
];

/// The `format` value that marks a hand-off file.
pub const HANDOFF_FORMAT: &str = "studyanalyzer-handoff";

/// The hand-off format version this app reads; a larger one is refused.
pub const HANDOFF_VERSION: u64 = 1;

/// The file extension (lower case, no dot) of a hand-off file.
pub const HANDOFF_EXTENSION: &str = "studyanalyzer";

/// The UTF-8 byte order mark, which Windows tools write at the start of a
/// file; a hand-off file may start with it.
pub const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Room, in bytes, for everything in a hand-off file but the base64 full text.
pub const HANDOFF_MANIFEST_BYTES: usize = 1024 * 1024;

/// The largest hand-off file read, in bytes: the largest PDF in base64, plus
/// room for the rest.
pub const MAX_HANDOFF_BYTES: usize = MAX_PDF_BYTES.div_ceil(3) * 4 + HANDOFF_MANIFEST_BYTES;

/// The longest `sender`, `source` or `file_name` in a hand-off, in characters.
pub const MAX_HANDOFF_LABEL_CHARS: usize = 200;

/// The bidirectional embeddings, overrides and isolates (U+202A–U+202E,
/// U+2066–U+2069) a hand-off label refuses: together they can reorder how the
/// text after them displays (U+202C and U+2069 close an earlier one), so a
/// sender name could read as another (#69). They are
/// format characters, which `char::is_control` does not catch.
pub const BIDI_CONTROLS: [char; 9] = [
    '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}',
    '\u{2069}',
];

/// The longest `title` in a hand-off, in characters.
pub const MAX_HANDOFF_TITLE_CHARS: usize = 2000;

/// The longest `studyanalyzer://` link accepted, in characters.
pub const MAX_LINK_CHARS: usize = 2048;

/// The scheme of a hand-off link.
pub const LINK_SCHEME: &str = "studyanalyzer";

/// The host of a hand-off link.
pub const LINK_HOST: &str = "open";

/// What a hand-off from a link (which has no sender) is said to come from.
pub const LINK_ORIGIN: &str = "a studyanalyzer:// link";

/// The file name of the SQLite database inside the app's data directory.
pub const DB_FILE_NAME: &str = "studyanalyzer.sqlite3";

/// The settings key under which the contact email for retrieval is stored.
pub const CONTACT_EMAIL_KEY: &str = "contact_email";

/// The settings key holding the shared paper library's folder, when the
/// user has chosen one (design 2026-11-13); without it, sources stay in the
/// app's own storage.
pub const LIBRARY_PATH_KEY: &str = "library_path";

/// The settings key holding the library folder this app last opened: an empty
/// folder becomes a library only while it is not this one, so a folder that
/// comes back empty (an unmounted drive's mount point) is unreachable, not a
/// new library on the wrong disk.
pub const LIBRARY_OPENED_KEY: &str = "library_opened";

/// The settings key holding this installation's id in the library's records:
/// a lower-case UUID made on first use.
pub const LIBRARY_INSTALLATION_KEY: &str = "library_installation";

/// This app's name in the library's record file names.
pub const LIBRARY_APP: &str = "studyanalyzer";

/// The label of this installation's records when the machine's name cannot
/// be part of it (bmlib takes 1 to 200 characters, without control or bidi
/// characters).
pub const LIBRARY_LABEL_FALLBACK: &str = "StudyAnalyzer";

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
/// `find_passage`'s similarity is a character ratio, so one changed character
/// scores about `1 - 1/L` for a quote of `L` characters. Since bmlib #632 its
/// guard rejects a match that reaches this value but changes a number, a
/// negation or a direction, so this value only has to part a dropped or added
/// word from what the guard does not see: a reordering, such as the two arms
/// swapped with the numbers in place, and a spelled-out number (bmlib #651).
/// Those still overlap (no value parts them), so this is the value that
/// rejects every such case measured. A dropped word passes in a sentence of
/// about 50 characters per character dropped, its space included (a 3-letter
/// word from about 200, a 5-letter one from about 300), and so does a swap of
/// two short arm names in one of about 300 or more; shorter near-verbatim
/// quotes lose their citation (visibly).
///
/// Measured with `tests/quote_threshold.rs` and `tests/fixtures/quotes.json`
/// (106 labelled cases on `article.xml` and the CC BY trial `real_rct.xml`;
/// see `tests/fixtures/README.md`), scores taken at a 0.5 floor. Highest
/// reject the guard lets through 0.9856 (the arms swapped in a 210-character
/// sentence); lowest accepted 0.9910 (a dropped "the" in a 220-character
/// sentence); seven true accepts between 0.9758 and 0.9885 are rejected
/// (`"expected": false`), one of them because the guard compared it with a
/// neighbouring sentence that has another group number. Quotes from ollama `gemma4:31b-it-qat` and
/// `medgemma:27b-it-q8_0` scored 1.0 when verbatim.
pub const MIN_QUOTE_SIMILARITY: f64 = 0.99;

/// The characters one token is taken to span when checking a prompt against a
/// model's window. Low on purpose, so the estimate errs towards "too big".
pub const FIT_CHARS_PER_TOKEN: u64 = 3;

/// The usual least room for a model's reply, in tokens: its `max_tokens` and
/// the room kept free when checking fit. Also the room when the window is
/// unknown or, outside [`UNCAPPED_REPLY_PROVIDERS`], the model's output limit
/// is, since a larger `max_tokens` than a model allows is refused. A model
/// whose own output limit is lower gets that instead (see `prompt::reply_tokens`).
pub const MIN_REPLY_TOKENS: u64 = 4096;

/// The most room a reply is given, in tokens, however large the window: a
/// thinking model's reasoning counts against `max_tokens` (`deepseek-flash`
/// spent 4096 on it before a PICO reply ended), and this bounds what one
/// reply may cost.
pub const MAX_REPLY_TOKENS: u64 = 32_768;

/// A reply is given at most this fraction (1/n) of the window, between
/// [`MIN_REPLY_TOKENS`] and [`MAX_REPLY_TOKENS`], so the paper keeps the rest.
pub const REPLY_WINDOW_SHARE: u64 = 4;

/// Providers whose servers take any `max_tokens` (Ollama's `num_predict`),
/// so their reply is sized from the window though they state no output limit.
pub const UNCAPPED_REPLY_PROVIDERS: [&str; 1] = ["ollama"];

/// The `stop_reason`s bmlib passes through for a reply cut off at its
/// `max_tokens`: Anthropic's `max_tokens`; OpenAI-compatible servers'
/// `length` (their `finish_reason`), which bmlib also gives for Ollama's
/// `done_reason` `length`.
pub const LENGTH_STOP_REASONS: [&str; 2] = ["max_tokens", "length"];

/// The temperature of an Interrogator answer: 0, so the same question gives
/// the same quotes.
pub const ANSWER_TEMPERATURE: f64 = 0.0;

/// How many passages a retrieval prompt carries.
pub const RETRIEVAL_PASSAGES: usize = 8;

/// The embedding model passages are found with.
pub const EMBEDDING_MODEL: bmlib::embed::ModelFile = bmlib::embed::ARCTIC_EMBED_L_V2_F16;

/// The folder under app storage the embedding model is kept in.
pub const EMBEDDER_SUBDIR: &str = "models";

/// The window a prompt is checked against when the model's window is unknown
/// and the user set none: room for a retrieval prompt of about 4096 estimated
/// tokens plus [`MIN_REPLY_TOKENS`], and the window a PICO run's budget is sized
/// from (about 9 000 characters of paper). The maintainer's call is retrieval
/// mode, not refusal, while the window is unknown. It is never sent as
/// `num_ctx` and never taken as the model's maximum.
pub const UNKNOWN_WINDOW_FIT_TOKENS: u64 = 8192;

/// The temperature of a PICO run: 0, as an Interrogator answer (per-model
/// parameters are #49's).
pub const PICO_TEMPERATURE: f64 = 0.0;

/// The smallest share of the window, in characters, a PICO run gives the
/// paper: bmlib's own default batch. A smaller budget only buys many
/// condensation calls that end in failure, so the run is refused instead.
pub const PICO_MIN_BUDGET_CHARS: usize = bmlib::context_processor::DEFAULT_MAX_CONTEXT_CHARS;

/// The setting holding the chosen `provider:model`.
pub const LLM_MODEL_KEY: &str = "llm_model";

/// The setting holding the user's context-window limit, in tokens.
pub const CONTEXT_TOKENS_KEY: &str = "context_tokens";

/// The OS keychain service the API keys are filed under (bmlib's
/// `KeyBundle::keyring`, item `API_KEYS_ACCOUNT`).
pub const KEYCHAIN_SERVICE: &str = "com.bmlibrarian.studyanalyzer";

#[cfg(test)]
mod tests {
    use super::*;

    /// The dialog offers every extension the formats name, PDF and plain text included.
    #[test]
    fn paper_file_extensions_hold_every_format() {
        let all = JATS_EXTENSIONS
            .iter()
            .chain(&HTML_EXTENSIONS)
            .chain(&MARKDOWN_EXTENSIONS)
            .chain(&[PDF_EXTENSION, PLAIN_TEXT_EXTENSION]);
        for ext in all {
            assert!(PAPER_FILE_EXTENSIONS.contains(ext), "{ext}");
        }
    }

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
