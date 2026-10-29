//! Intake: turns any source (PDF, JATS, HTML, Markdown, plain text) into
//! bmlib's canonical text, refusing what it cannot read faithfully.

use std::fmt;
use std::path::{Path, PathBuf};

use bmlib::fulltext::canonical::{
    assess_full_text, canonical_from_html, canonical_from_jats, canonical_from_markdown,
    canonical_from_plain_text, canonical_from_segmented, CanonicalText, FullTextAssessment,
    SourceFormat, HTML_CANONICAL_VERSION, JATS_CANONICAL_VERSION, MARKDOWN_VERSION,
    PDF_CANONICAL_VERSION, PLAIN_TEXT_VERSION,
};
use bmlib::fulltext::jats_reader;
use bmlib::fulltext::pdf_text::PdfTextExtractor;
use bmlib::fulltext::segmenter::SectionSegmenter;
use encoding_rs::Encoding;
use serde::{Deserialize, Serialize};

use crate::config::{
    BYTES_PER_MB, HTML_CHARSET_PRESCAN_BYTES, HTML_EXTENSIONS, JATS_EXTENSIONS,
    MARKDOWN_EXTENSIONS, MAX_HTML_BYTES, MAX_PDF_BYTES, MAX_TEXT_BYTES, PDF_MAGIC,
    PLAIN_TEXT_EXTENSION,
};
use crate::identifiers::{validate_identifiers, Identifiers};

/// The kind of source a paper's text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A PDF file.
    Pdf,
    /// JATS XML (PubMed Central and publishers).
    Jats,
    /// An HTML page.
    Html,
    /// A Markdown file.
    Markdown,
    /// Plain text.
    PlainText,
}

impl SourceKind {
    /// The stable name used in storage (`"plain_text"`).
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Pdf => "pdf",
            SourceKind::Jats => "jats",
            SourceKind::Html => "html",
            SourceKind::Markdown => "markdown",
            SourceKind::PlainText => "plain_text",
        }
    }

    /// The inverse of [`SourceKind::as_str`]; `None` for an unknown name.
    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::Pdf,
            Self::Jats,
            Self::Html,
            Self::Markdown,
            Self::PlainText,
        ]
        .into_iter()
        .find(|k| k.as_str() == name)
    }

    /// A short name for messages shown to the user.
    fn label(self) -> &'static str {
        match self {
            SourceKind::Pdf => "PDF",
            SourceKind::Jats => "XML",
            SourceKind::Html => "HTML",
            SourceKind::Markdown => "Markdown",
            SourceKind::PlainText => "text",
        }
    }
}

/// Why a source could not be turned into canonical text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntakeError {
    /// The file type is not one we read (the text names it).
    Unsupported(String),
    /// The file is larger than the limit for its kind.
    TooLarge {
        /// The kind of source.
        kind: SourceKind,
        /// The size received, in bytes.
        len: usize,
        /// The limit, in bytes.
        max: usize,
    },
    /// The file holds binary data, not text.
    Binary,
    /// The text decodes under no encoding without errors.
    Undecodable,
    /// The JATS XML could not be read.
    Jats(String),
    /// The PDF could not be read.
    Pdf(String),
    /// The source holds no text.
    Empty,
    /// bmlib rejected the canonical text.
    Canonical(String),
    /// A file could not be written or removed.
    Io(String),
}

impl fmt::Display for IntakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntakeError::Unsupported(what) => {
                write!(f, "StudyAnalyzer cannot read this kind of file ({what}). Use a PDF, XML, HTML, Markdown or text file.")
            }
            IntakeError::TooLarge { kind, len, max } => write!(
                f,
                "This {} file is {:.1} MB; the limit is {:.1} MB.",
                kind.label(),
                *len as f64 / BYTES_PER_MB,
                *max as f64 / BYTES_PER_MB
            ),
            IntakeError::Binary => write!(
                f,
                "This file holds binary data, not text, and cannot be read as a paper."
            ),
            IntakeError::Undecodable => write!(
                f,
                "The text could not be decoded under any known character encoding."
            ),
            IntakeError::Jats(m) => write!(f, "The XML could not be read as a paper: {m}"),
            IntakeError::Pdf(m) => write!(f, "The PDF could not be read: {m}"),
            IntakeError::Empty => write!(f, "No text was found in this source."),
            IntakeError::Canonical(m) => write!(f, "The paper's text could not be prepared: {m}"),
            IntakeError::Io(m) => write!(f, "A temporary file could not be handled: {m}"),
        }
    }
}

impl std::error::Error for IntakeError {}

/// How a text file's bytes were turned into characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoded {
    /// The WHATWG encoding name, e.g. `"windows-1252"`.
    pub encoding: String,
    /// True when the encoding was detected rather than declared by the file.
    pub guessed: bool,
}

/// Decide what kind of source `bytes` are, from the PDF magic, then the file
/// name's extension, and for a paste (no name) from the text's first characters.
pub fn detect_kind(file_name: Option<&str>, bytes: &[u8]) -> Result<SourceKind, IntakeError> {
    if bytes.starts_with(PDF_MAGIC) {
        return Ok(SourceKind::Pdf);
    }
    if let Some(name) = file_name {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .unwrap_or_default();
        let ext = ext.as_str();
        return if JATS_EXTENSIONS.contains(&ext) {
            Ok(SourceKind::Jats)
        } else if HTML_EXTENSIONS.contains(&ext) {
            Ok(SourceKind::Html)
        } else if MARKDOWN_EXTENSIONS.contains(&ext) {
            Ok(SourceKind::Markdown)
        } else if ext == PLAIN_TEXT_EXTENSION {
            Ok(SourceKind::PlainText)
        } else {
            Err(IntakeError::Unsupported(name.to_string()))
        };
    }
    let text = String::from_utf8_lossy(bytes);
    let head = text.trim_start().to_lowercase();
    Ok(
        if head.starts_with("<?xml") || head.starts_with("<article") {
            SourceKind::Jats
        } else if head.starts_with("<!doctype html") || head.starts_with("<html") {
            SourceKind::Html
        } else if text.lines().any(|l| l.starts_with("# ")) {
            SourceKind::Markdown
        } else {
            SourceKind::PlainText
        },
    )
}

/// Refuse a source larger than the limit for its kind (before any parsing).
pub fn check_size(kind: SourceKind, len: usize) -> Result<(), IntakeError> {
    let max = match kind {
        SourceKind::Pdf => MAX_PDF_BYTES,
        SourceKind::Html => MAX_HTML_BYTES,
        _ => MAX_TEXT_BYTES,
    };
    if len > max {
        log::warn!(
            "refusing a {} source of {len} bytes (limit {max})",
            kind.as_str()
        );
        return Err(IntakeError::TooLarge { kind, len, max });
    }
    Ok(())
}

/// Decode `bytes` strictly (no replacement characters) under `enc`.
fn strict(enc: &'static Encoding, bytes: &[u8]) -> Option<String> {
    enc.decode_without_bom_handling_and_without_replacement(bytes)
        .map(|c| c.into_owned())
}

/// The value after `charset` in the first bytes of an HTML file, if any.
fn html_declared_label(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(HTML_CHARSET_PRESCAN_BYTES)];
    let head = String::from_utf8_lossy(head).to_lowercase();
    let rest = head[head.find("charset")? + "charset".len()..].trim_start();
    label_after(rest.strip_prefix('=')?)
}

/// The value of `encoding="…"` in an XML declaration at the start of `bytes`.
fn xml_declared_label(bytes: &[u8]) -> Option<String> {
    let head =
        String::from_utf8_lossy(&bytes[..bytes.len().min(HTML_CHARSET_PRESCAN_BYTES)]).into_owned();
    let decl = head.trim_start().strip_prefix("<?xml")?;
    let decl = &decl[..decl.find("?>")?];
    let rest = decl[decl.find("encoding")? + "encoding".len()..].trim_start();
    label_after(rest.strip_prefix('=')?)
}

/// Read an encoding label from `s`, skipping spaces and one opening quote.
fn label_after(s: &str) -> Option<String> {
    let s = s.trim_start().trim_start_matches(['"', '\'']);
    let label: String = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
        .collect();
    (!label.is_empty()).then_some(label)
}

/// Turn text-file bytes into a string: BOM, then valid UTF-8, then the file's
/// own declaration, then a detected guess. Never invents replacement characters.
pub fn decode_text(kind: SourceKind, bytes: &[u8]) -> Result<(String, Decoded), IntakeError> {
    // Step 1: a BOM. UTF-16 holds NUL bytes legitimately, so this precedes the binary check.
    if let Some((enc, bom_len)) = Encoding::for_bom(bytes) {
        let text = strict(enc, &bytes[bom_len..]).ok_or(IntakeError::Undecodable)?;
        return Ok((
            text,
            Decoded {
                encoding: enc.name().to_string(),
                guessed: false,
            },
        ));
    }
    if bytes.contains(&0) {
        return Err(IntakeError::Binary);
    }
    // Step 2: valid UTF-8.
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok((
            text.to_string(),
            Decoded {
                encoding: "UTF-8".into(),
                guessed: false,
            },
        ));
    }
    // Step 3: what the file says about itself.
    let label = match kind {
        SourceKind::Html => html_declared_label(bytes),
        SourceKind::Jats => xml_declared_label(bytes),
        _ => None,
    };
    if let Some(label) = label {
        match Encoding::for_label(label.as_bytes()) {
            // Without a BOM a UTF-16 declaration cannot be true (WHATWG reads it as UTF-8).
            Some(enc) if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE => {
                log::warn!("ignoring declared {label} on a file with no UTF-16 BOM")
            }
            Some(enc) => match strict(enc, bytes) {
                Some(text) => {
                    return Ok((
                        text,
                        Decoded {
                            encoding: enc.name().to_string(),
                            guessed: false,
                        },
                    ))
                }
                None => log::warn!("bytes do not decode under the declared encoding {label}"),
            },
            None => log::warn!("unknown declared encoding label {label}"),
        }
    }
    // Step 4: detection.
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let enc = detector.guess(None, chardetng::Utf8Detection::Deny);
    let text = strict(enc, bytes).ok_or(IntakeError::Undecodable)?;
    // Single-byte decoders never fail, so binary data must be caught by its control characters.
    if text
        .chars()
        .any(|c| c < ' ' && !matches!(c, '\t' | '\n' | '\r' | '\x0c'))
    {
        return Err(IntakeError::Binary);
    }
    log::warn!("encoding was not declared; guessed {}", enc.name());
    Ok((
        text,
        Decoded {
            encoding: enc.name().to_string(),
            guessed: true,
        },
    ))
}

/// Removes its file when dropped, so scratch PDFs go on every path.
struct ScratchFile(PathBuf);

impl Drop for ScratchFile {
    /// Delete the file; a failure is logged, not hidden.
    fn drop(&mut self) {
        match std::fs::remove_file(&self.0) {
            Ok(()) => {}
            // Nothing was written (the write failed before creating it).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::error!("could not remove scratch file {}: {e}", self.0.display()),
        }
    }
}

/// Convert a PDF through `pdf`, via a uniquely named file in `scratch_dir`.
fn pdf_to_canonical(
    bytes: &[u8],
    title_hint: Option<&str>,
    pdf: &dyn PdfTextExtractor,
    scratch_dir: &Path,
) -> Result<CanonicalText, IntakeError> {
    let path = scratch_dir.join(format!("intake-{}.pdf", uuid::Uuid::new_v4()));
    // The guard comes first so a partial write is cleaned up too.
    let _guard = ScratchFile(path.clone());
    std::fs::write(&path, bytes).map_err(|e| IntakeError::Io(e.to_string()))?;
    let blocks = pdf.extract_blocks(&path).map_err(IntakeError::Pdf)?;
    let doc = SectionSegmenter::default().segment_document(
        &blocks,
        &path.to_string_lossy(),
        title_hint,
        serde_json::json!({}),
    );
    Ok(canonical_from_segmented(&doc))
}

/// Convert `bytes` of the given kind to canonical text. Checks size first;
/// non-PDF kinds are decoded (the [`Decoded`] says how) before conversion.
pub fn to_canonical(
    kind: SourceKind,
    bytes: &[u8],
    title_hint: Option<&str>,
    pdf: &dyn PdfTextExtractor,
    scratch_dir: &Path,
) -> Result<(CanonicalText, Option<Decoded>), IntakeError> {
    check_size(kind, bytes.len())?;
    let (canonical, decoded) = if kind == SourceKind::Pdf {
        (pdf_to_canonical(bytes, title_hint, pdf, scratch_dir)?, None)
    } else {
        let (text, decoded) = decode_text(kind, bytes)?;
        let canonical = match kind {
            SourceKind::Jats => {
                let article =
                    jats_reader::parse(&text).map_err(|e| IntakeError::Jats(e.to_string()))?;
                canonical_from_jats(&article)
            }
            SourceKind::Html => canonical_from_html(&text),
            SourceKind::Markdown => canonical_from_markdown(&text),
            _ => canonical_from_plain_text(&text, title_hint.unwrap_or("")),
        };
        (canonical, Some(decoded))
    };
    if canonical.sections.iter().all(|s| s.blocks.is_empty()) {
        return Err(IntakeError::Empty);
    }
    Ok((canonical, decoded))
}

/// Check whether `text` looks like a whole paper (bmlib's full-text rule).
pub fn assess(text: &CanonicalText) -> Result<FullTextAssessment, IntakeError> {
    assess_full_text(text).map_err(|e| IntakeError::Canonical(e.to_string()))
}

/// The identifiers a JATS document carries about itself. Each one is validated
/// on its own; an invalid one is dropped with a warning. Any failure to read
/// the document yields no identifiers (conversion reports the real error).
pub fn jats_identifiers(bytes: &[u8]) -> Identifiers {
    let Ok((text, _)) = decode_text(SourceKind::Jats, bytes) else {
        return Identifiers::default();
    };
    let Ok(article) = jats_reader::parse(&text) else {
        return Identifiers::default();
    };
    let one = |kind: &str, value: &str, pick: fn(Identifiers) -> Option<String>| {
        if value.trim().is_empty() {
            return None;
        }
        let ids = match kind {
            "pmid" => validate_identifiers(Some(value), None, None),
            "doi" => validate_identifiers(None, Some(value), None),
            _ => validate_identifiers(None, None, Some(value)),
        };
        match ids {
            Ok(ids) => pick(ids),
            Err(e) => {
                log::warn!("ignoring invalid {kind} in the document: {e}");
                None
            }
        }
    };
    Identifiers {
        pmid: one("pmid", &article.pmid, |i| i.pmid),
        doi: one("doi", &article.doi, |i| i.doi),
        pmc_id: one("pmc_id", &article.pmc_id, |i| i.pmc_id),
    }
}

/// The converter version bmlib currently stamps on text from `kind`.
pub fn current_converter_version(kind: SourceKind) -> String {
    let (format, version) = match kind {
        SourceKind::Pdf => (SourceFormat::Pdf, PDF_CANONICAL_VERSION),
        SourceKind::Jats => (SourceFormat::Jats, JATS_CANONICAL_VERSION),
        SourceKind::Html => (SourceFormat::Html, HTML_CANONICAL_VERSION),
        SourceKind::Markdown => (SourceFormat::Markdown, MARKDOWN_VERSION),
        SourceKind::PlainText => (SourceFormat::PlainText, PLAIN_TEXT_VERSION),
    };
    format.converter_version(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MAX_HTML_BYTES, MAX_TEXT_BYTES};
    use bmlib::fulltext::canonical::MIN_BODY_WORDS;
    use bmlib::fulltext::models::TextBlock;

    const JATS: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/article.xml"
    ));
    const HTML: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/page.html"
    ));
    const MD: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/paper.md"
    ));
    const TXT: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/paper.txt"
    ));

    /// A line of text with the given size, for the fake extractor.
    fn block(text: &str, size: f64, bold: bool, y: f64) -> TextBlock {
        TextBlock {
            text: text.to_string(),
            page_num: 0,
            font_size: size,
            font_name: "Test".into(),
            is_bold: bold,
            is_italic: false,
            x: 72.0,
            y,
            width: 400.0,
            height: size,
        }
    }

    /// A PDF backend that returns fixed blocks, or an error.
    struct FakePdf {
        fail: bool,
    }

    impl PdfTextExtractor for FakePdf {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn extract_blocks(&self, path: &Path) -> Result<Vec<TextBlock>, String> {
            assert!(
                path.exists(),
                "the scratch file must exist while extracting"
            );
            if self.fail {
                return Err("no pdf here".into());
            }
            let mut v = Vec::new();
            let mut y = 700.0;
            for (t, size, bold) in [
                ("Methods", 16.0, true),
                ("We randomised adults to tablets or placebo.", 10.0, false),
                ("Results", 16.0, true),
                ("The tablets lowered the score.", 10.0, false),
            ] {
                v.push(block(t, size, bold, y));
                y -= 40.0;
            }
            Ok(v)
        }
        fn page_texts(&self, _: &Path) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
    }

    /// A fresh empty scratch directory.
    fn scratch() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sa-intake-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn detect_by_magic_and_name() {
        assert_eq!(
            detect_kind(Some("a.txt"), b"%PDF-1.7").unwrap(),
            SourceKind::Pdf
        );
        for (n, k) in [
            ("a.xml", SourceKind::Jats),
            ("a.NXML", SourceKind::Jats),
            ("a.html", SourceKind::Html),
            ("a.htm", SourceKind::Html),
            ("a.md", SourceKind::Markdown),
            ("a.markdown", SourceKind::Markdown),
            ("a.txt", SourceKind::PlainText),
        ] {
            assert_eq!(detect_kind(Some(n), b"x").unwrap(), k, "{n}");
        }
        assert!(matches!(
            detect_kind(Some("a.docx"), b"x"),
            Err(IntakeError::Unsupported(_))
        ));
    }

    #[test]
    fn detect_paste() {
        assert_eq!(
            detect_kind(None, b"  <!DOCTYPE html><p>x").unwrap(),
            SourceKind::Html
        );
        assert_eq!(
            detect_kind(None, b"\n<?xml version=\"1.0\"?>").unwrap(),
            SourceKind::Jats
        );
        assert_eq!(detect_kind(None, b"<article>").unwrap(), SourceKind::Jats);
        assert_eq!(
            detect_kind(None, b"# Title\n\ntext").unwrap(),
            SourceKind::Markdown
        );
        assert_eq!(
            detect_kind(None, b"Plain words.").unwrap(),
            SourceKind::PlainText
        );
    }

    #[test]
    fn kind_round_trips_through_storage_names() {
        for k in [
            SourceKind::Pdf,
            SourceKind::Jats,
            SourceKind::Html,
            SourceKind::Markdown,
            SourceKind::PlainText,
        ] {
            assert_eq!(SourceKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(SourceKind::parse("zip"), None);
    }

    #[test]
    fn encodings_decoded() {
        let (t, d) = decode_text(
            SourceKind::PlainText,
            b"caf\xe9 au lait, cr\xe8me br\xfbl\xe9e",
        )
        .unwrap();
        assert_eq!(t, "café au lait, crème brûlée");
        assert_eq!(
            d,
            Decoded {
                encoding: "windows-1252".into(),
                guessed: true
            }
        );

        let mut u16 = vec![0xFF, 0xFE];
        u16.extend("café".encode_utf16().flat_map(|c| c.to_le_bytes()));
        let (t, d) = decode_text(SourceKind::PlainText, &u16).unwrap();
        assert_eq!(
            (t.as_str(), d.encoding.as_str(), d.guessed),
            ("café", "UTF-16LE", false)
        );

        let (t, d) = decode_text(SourceKind::PlainText, b"\xEF\xBB\xBFcaf\xC3\xA9").unwrap();
        assert_eq!(
            (t.as_str(), d.encoding.as_str(), d.guessed),
            ("café", "UTF-8", false)
        );
    }

    #[test]
    fn declarations_win() {
        let html = b"<meta charset=\"iso-8859-1\"><p>caf\xe9";
        let (t, d) = decode_text(SourceKind::Html, html).unwrap();
        assert!(t.ends_with("café"));
        assert_eq!(
            d,
            Decoded {
                encoding: "windows-1252".into(),
                guessed: false
            }
        );

        let xml = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>caf\xe9</a>";
        let (t, d) = decode_text(SourceKind::Jats, xml).unwrap();
        assert!(t.ends_with("café</a>"));
        assert!(!d.guessed);

        let bad = b"<meta charset=\"no-such\"><p>caf\xe9 au lait, cr\xe8me br\xfbl\xe9e";
        let (_, d) = decode_text(SourceKind::Html, bad).unwrap();
        assert!(d.guessed);
    }

    #[test]
    fn binary_refused() {
        assert_eq!(
            decode_text(SourceKind::PlainText, b"PK\x03\x04\x00\x00").unwrap_err(),
            IntakeError::Binary
        );
    }

    #[test]
    fn guessed_text_with_control_characters_is_binary() {
        let mut b = b"\x01\x02\x03abc\x1b\x7f".to_vec();
        b.extend_from_slice(b"\xe9\xe8\xfb\xe9 caf\xe9 cr\xe8me");
        assert_eq!(
            decode_text(SourceKind::PlainText, &b).unwrap_err(),
            IntakeError::Binary
        );
    }

    #[test]
    fn declared_utf16_without_bom_is_ignored() {
        let (t, d) = decode_text(
            SourceKind::Html,
            b"<meta charset=\"utf-16\"><p>caf\xe9 au lait, cr\xe8me br\xfbl\xe9e",
        )
        .unwrap();
        assert!(t.contains("café au lait"));
        assert!(d.guessed);
    }

    #[test]
    fn failed_scratch_write_leaves_nothing_and_is_io() {
        let file = std::env::temp_dir().join(format!("sa-notdir-{}", uuid::Uuid::new_v4()));
        std::fs::write(&file, b"x").unwrap();
        let r = to_canonical(
            SourceKind::Pdf,
            b"%PDF-1.4 x",
            None,
            &FakePdf { fail: false },
            &file,
        );
        assert!(matches!(r, Err(IntakeError::Io(_))));
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn oversize_html_refused_before_parse() {
        assert!(matches!(
            check_size(SourceKind::Html, MAX_HTML_BYTES + 1),
            Err(IntakeError::TooLarge {
                kind: SourceKind::Html,
                ..
            })
        ));
        assert!(check_size(SourceKind::Html, MAX_HTML_BYTES).is_ok());
        assert!(check_size(SourceKind::PlainText, MAX_TEXT_BYTES + 1).is_err());
        let big = b"<div>".repeat(MAX_HTML_BYTES / 5 + 1);
        let started = std::time::Instant::now();
        let err = to_canonical(
            SourceKind::Html,
            &big,
            None,
            &FakePdf { fail: false },
            &std::env::temp_dir(),
        )
        .unwrap_err();
        assert!(matches!(err, IntakeError::TooLarge { .. }));
        assert!(started.elapsed().as_secs_f64() < 1.0);
        assert!(err.to_string().contains("22.0 MB"));
    }

    #[test]
    fn jats_identifiers_are_read_and_validated() {
        let ids = jats_identifiers(JATS);
        assert_eq!(ids.pmid.as_deref(), Some("12345678"));
        assert_eq!(ids.doi.as_deref(), Some("10.1234/fixture.1"));
        assert_eq!(ids.pmc_id, None);
        let bad = String::from_utf8_lossy(JATS).replace("12345678", "not-a-pmid");
        assert_eq!(jats_identifiers(bad.as_bytes()).pmid, None);
        assert_eq!(jats_identifiers(b"garbage"), Identifiers::default());
    }

    #[test]
    fn each_fixture_converts() {
        let pdf = FakePdf { fail: false };
        for (kind, bytes, sentence) in [
            (SourceKind::Jats, JATS, "fixture trial"),
            (SourceKind::Html, HTML, "saved page fixture"),
            (SourceKind::Markdown, MD, "markdown fixture"),
            (SourceKind::PlainText, TXT, "plain text fixture"),
        ] {
            let (c, _) = to_canonical(kind, bytes, None, &pdf, &std::env::temp_dir()).unwrap();
            assert_eq!(
                c.converter_version,
                current_converter_version(kind),
                "{kind:?}"
            );
            assert!(c.text.contains(sentence), "{kind:?}");
        }
    }

    #[test]
    fn jats_fixture_is_full_text() {
        let (c, _) = to_canonical(
            SourceKind::Jats,
            JATS,
            None,
            &FakePdf { fail: false },
            &std::env::temp_dir(),
        )
        .unwrap();
        let a = assess(&c).unwrap();
        assert!(a.is_full_text, "{a:?}");
        assert!(a.body_words >= MIN_BODY_WORDS);
    }

    #[test]
    fn malformed_jats_is_a_jats_error() {
        let r = to_canonical(
            SourceKind::Jats,
            b"<article><a></article>",
            None,
            &FakePdf { fail: false },
            &std::env::temp_dir(),
        );
        assert!(matches!(r, Err(IntakeError::Jats(_))));
    }

    #[test]
    fn pdf_via_extractor_cleans_scratch() {
        let tmp = scratch();
        let (c, d) = to_canonical(
            SourceKind::Pdf,
            b"%PDF-1.4 x",
            Some("T"),
            &FakePdf { fail: false },
            &tmp,
        )
        .unwrap();
        assert!(d.is_none());
        assert!(c.text.contains("Methods"));
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        let r = to_canonical(
            SourceKind::Pdf,
            b"%PDF-1.4 x",
            Some("T"),
            &FakePdf { fail: true },
            &tmp,
        );
        assert!(matches!(r, Err(IntakeError::Pdf(_))));
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn empty_text_refused() {
        let r = to_canonical(
            SourceKind::PlainText,
            b"  \n\n ",
            None,
            &FakePdf { fail: false },
            &std::env::temp_dir(),
        );
        assert!(matches!(r, Err(IntakeError::Empty)));
    }

    #[test]
    fn errors_read_for_users() {
        let e = IntakeError::TooLarge {
            kind: SourceKind::Pdf,
            len: 150 * 1024 * 1024,
            max: 100 * 1024 * 1024,
        };
        let s = e.to_string();
        assert!(s.contains("150.0 MB") && s.contains("100.0 MB"), "{s}");
    }

    /// Manual check: STUDYANALYZER_TEST_PDF names a real PDF.
    #[test]
    #[ignore]
    fn real_pdf() {
        let path = std::env::var("STUDYANALYZER_TEST_PDF").expect("set STUDYANALYZER_TEST_PDF");
        let bytes = std::fs::read(path).unwrap();
        let pdfium = bmlib::fulltext::pdfium_backend::PdfiumExtractor::bundled().unwrap();
        let tmp = scratch();
        let (c, _) = to_canonical(SourceKind::Pdf, &bytes, None, &pdfium, &tmp).unwrap();
        println!(
            "{} chars, {} sections",
            c.text.chars().count(),
            c.sections.len()
        );
        assert!(!c.text.is_empty());
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
