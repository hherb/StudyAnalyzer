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

/// How many leading bytes of an HTML file are searched for a charset
/// declaration (the HTML standard's prescan window).
pub const HTML_CHARSET_PRESCAN_BYTES: usize = 1024;

/// The magic bytes every PDF file starts with.
pub const PDF_MAGIC: &[u8] = b"%PDF-";

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

/// How long a connection waits for another connection's write lock before
/// failing; each command opens its own connection to the same WAL database.
pub const DB_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The title given to a paper when neither its text nor its sender names one.
pub const UNTITLED_PAPER: &str = "Untitled paper";

/// Bytes per megabyte, for messages shown to the user.
pub const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// How many times a failing network call is tried in all (first try included).
pub const RETRIEVAL_ATTEMPTS: u32 = 3;

/// The wait before the first retry; it doubles after each failed attempt.
pub const RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// The start of bmlib's message when every tier ran and nothing was found
/// (bmlib `fulltext/service.rs`, the exhaustion error near line 1745).
pub const BMLIB_EXHAUSTED_PREFIX: &str = "Nothing retrieved";

/// Text bmlib's `TierFailures::describe()` (service.rs ~376) includes only when
/// at least one tier faulted (`"N attempts failed (Kind, ...)"`).
pub const BMLIB_FAULT_MARKER: &str = " failed (";

/// bmlib's error for a request with no identifiers (service.rs ~1706).
pub const BMLIB_NO_IDENTIFIERS: &str = "No identifiers provided";

/// The `origin` stored for a source the user chose from a file.
pub const ORIGIN_UPLOAD: &str = "upload";

/// The `origin` stored for a source the user pasted as text.
pub const ORIGIN_PASTE: &str = "paste";

#[cfg(test)]
mod tests {
    use super::*;

    /// The product name is the one the bundle and the UI use.
    #[test]
    fn app_name_is_studyanalyzer() {
        assert_eq!(APP_NAME, "StudyAnalyzer");
    }
}
