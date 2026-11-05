//! Intake: turns any source (PDF, JATS, HTML, Markdown, plain text) into
//! bmlib's canonical text, refusing what it cannot read faithfully.

use std::fmt;
use std::path::{Path, PathBuf};

use bmlib::fulltext::canonical::{
    assess_full_text, canonical_from_html, canonical_from_jats, canonical_from_markdown,
    canonical_from_plain_text, canonical_from_segmented, CanonicalText, FullTextAssessment,
    HtmlError, SourceFormat, HTML_CANONICAL_VERSION, JATS_CANONICAL_VERSION, MARKDOWN_VERSION,
    PDF_CANONICAL_VERSION, PLAIN_TEXT_VERSION,
};
use bmlib::fulltext::jats_reader;
use bmlib::fulltext::pdf_text::{PdfError, PdfLimit, PdfTextExtractor};
use bmlib::fulltext::segmenter::SectionSegmenter;
use encoding_rs::Encoding;
use serde::{Deserialize, Serialize};

use crate::config::{
    BYTES_PER_MB, ENGINE_NOTICE_LEAD, HTML_CHARSET_PRESCAN_BYTES, HTML_EXTENSIONS, JATS_EXTENSIONS,
    MARKDOWN_EXTENSIONS, MAX_HTML_BYTES, MAX_PDF_BYTES, MAX_TEXT_BYTES, PASTE_SNIFF_CHARS,
    PDF_EXTENSION, PDF_MAGIC, PLAIN_TEXT_EXTENSION,
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
    /// The file type is not one we read (the text names it). `open` also uses
    /// it to refuse an identifier where only a file or paste is allowed.
    Unsupported(String),
    /// The file is named `.pdf` but does not start as a PDF does (the text
    /// names it): damaged, or something else saved under that name.
    NotPdf(String),
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
    /// bmlib refused the HTML page: longer than `HTML_MAX_CHARS` code points,
    /// so never parsed (bmlib #654), or still being parsed at
    /// `HTML_PARSE_SECONDS`, usually because it is nested too deeply (#563).
    Html(HtmlError),
    /// The PDF could not be extracted; the kind says whether the file or the
    /// engine is at fault, or whether a limit (cancel or deadline) stopped it.
    Pdf(PdfError),
    /// The source holds no text; for a PDF, the [`engine_notice`] that may
    /// say why (a page cut short before any text).
    Empty(Option<String>),
    /// bmlib rejected the canonical text.
    Canonical(String),
    /// The scratch PDF could not be written.
    Io(String),
}

impl fmt::Display for IntakeError {
    /// A message the user can read.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntakeError::Unsupported(what) => {
                write!(f, "StudyAnalyzer cannot read this kind of file ({what}). Use a PDF, XML, HTML, Markdown or text file.")
            }
            IntakeError::NotPdf(name) => write!(
                f,
                "{name} is named as a PDF but is not one: it may be damaged, or a web page \
                 saved under that name. Download the PDF again, or open the page as HTML."
            ),
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
            // Pasted text is not parsed as HTML, so a page too long to parse can still be read.
            IntakeError::Html(e @ HtmlError::TooLarge { .. }) => write!(
                f,
                "The web page could not be read: {e}. Paste its text instead."
            ),
            IntakeError::Html(e) => write!(f, "The web page could not be read: {e}."),
            // bmlib's message is a sentence that starts lower-case ("the PDF …").
            IntakeError::Pdf(e) => {
                let mut c = e.message.chars();
                match c.next() {
                    Some(first) => write!(f, "{}{}", first.to_uppercase(), c.as_str())?,
                    // Named by kind, so an engine fault does not blame the file.
                    None => write!(f, "The PDF extraction failed ({}).", e.kind.as_str())?,
                }
                // What MuPDF said before failing (a repaired xref, say; bmlib #619).
                match engine_notice(&e.engine_messages) {
                    // bmlib's message has no full stop of its own.
                    Some(n) if e.message.ends_with('.') || e.message.is_empty() => {
                        write!(f, " {n}")
                    }
                    Some(n) => write!(f, ". {n}"),
                    None => Ok(()),
                }
            }
            IntakeError::Empty(None) => write!(f, "No text was found in this source."),
            IntakeError::Empty(Some(n)) => write!(f, "No text was found in this source. {n}"),
            IntakeError::Canonical(m) => write!(f, "The paper's text could not be prepared: {m}"),
            IntakeError::Io(m) => write!(f, "A temporary file could not be handled: {m}"),
        }
    }
}

impl IntakeError {
    /// True when the document itself is at fault, so another file (not a
    /// retry) is the remedy; false otherwise: a PDF engine fault, a stopped
    /// extraction (cancel or deadline), a scratch file, a canonical text bmlib
    /// refused.
    pub fn is_the_documents(&self) -> bool {
        match self {
            IntakeError::Pdf(e) => e.kind.is_the_files(),
            IntakeError::Canonical(_) | IntakeError::Io(_) => false,
            IntakeError::Unsupported(_)
            | IntakeError::NotPdf(_)
            | IntakeError::TooLarge { .. }
            | IntakeError::Binary
            | IntakeError::Undecodable
            | IntakeError::Jats(_)
            | IntakeError::Html(_)
            | IntakeError::Empty(_) => true,
        }
    }
}

impl std::error::Error for IntakeError {}

/// How a text file's bytes were turned into characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoded {
    /// The WHATWG encoding name, e.g. `"windows-1252"`.
    pub encoding: String,
    /// True only when chardetng picked the encoding (step 4 of [`decode_text`]);
    /// false for a BOM, valid UTF-8, or a declaration that decoded cleanly.
    pub guessed: bool,
}

/// Decide what kind of source `bytes` are, from the PDF magic, then the file
/// name's extension ([`IntakeError::NotPdf`] for a `.pdf` that is not one). A
/// paste (no name) is XML or HTML by its first characters,
/// Markdown when any line starts with `# `, else plain text.
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
        return if ext == PDF_EXTENSION {
            Err(IntakeError::NotPdf(name.to_string()))
        } else if JATS_EXTENSIONS.contains(&ext) {
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
    // Only the head is lowercased: a paste is not size-checked yet.
    let head = text
        .trim_start()
        .chars()
        .take(PASTE_SNIFF_CHARS)
        .collect::<String>()
        .to_lowercase();
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

/// The notice telling the user what the PDF engine read past without failing
/// (bmlib #599: a repaired xref, a page cut short, a bogus font), so the text
/// may be short; `None` when it reported nothing. Every message is kept, in
/// order; a run of identical ones is given once with its count, since a page
/// cut short can repeat one message up to a hundred times (MuPDF stops a page
/// at its hundredth syntax error). It begins with [`ENGINE_NOTICE_LEAD`].
pub fn engine_notice(warnings: &[String]) -> Option<String> {
    let mut runs: Vec<(&str, usize)> = Vec::new();
    for w in warnings {
        match runs.last_mut() {
            Some((last, n)) if *last == w.as_str() => *n += 1,
            _ => runs.push((w, 1)),
        }
    }
    if runs.is_empty() {
        return None;
    }
    let messages: Vec<String> = runs
        .into_iter()
        .map(|(w, n)| {
            if n == 1 {
                w.to_string()
            } else {
                format!("{w} ({n} times)")
            }
        })
        .collect();
    Some(format!("{ENGINE_NOTICE_LEAD}: {}.", messages.join("; ")))
}

/// True when `notice` is one [`engine_notice`] made.
pub fn is_engine_notice(notice: &str) -> bool {
    notice.starts_with(ENGINE_NOTICE_LEAD)
}

/// Convert a PDF through `pdf`, via a uniquely named file in `scratch_dir`,
/// stopping at `limit`. The title offered to the segmenter, which believes it
/// only where page 1 prints it, is the PDF's own `/Title`, else `title_hint`
/// (the file's name). Also returns [`engine_notice`] for what the engine
/// reported; a failed read carries the messages on its error (bmlib #619),
/// which `IntakeError`'s message ends with.
fn pdf_to_canonical(
    bytes: &[u8],
    title_hint: Option<&str>,
    pdf: &dyn PdfTextExtractor,
    scratch_dir: &Path,
    limit: &PdfLimit,
) -> Result<(CanonicalText, Option<String>), IntakeError> {
    let path = scratch_dir.join(format!("intake-{}.pdf", uuid::Uuid::new_v4()));
    // The guard comes first so a partial write is cleaned up too.
    let _guard = ScratchFile(path.clone());
    std::fs::write(&path, bytes).map_err(|e| IntakeError::Io(e.to_string()))?;
    let read = pdf
        .extract_blocks_with_warnings(&path, limit)
        .map_err(IntakeError::Pdf)?;
    let doc = SectionSegmenter::default().segment_document(
        &read.blocks,
        &path.to_string_lossy(),
        read.metadata_title.as_deref().or(title_hint),
        serde_json::json!({}),
    );
    // bmlib's own note (a `/Title` it could not read) rides in the same notice,
    // as before bmlib split it out (#619), so a re-derive replaces it too.
    let messages = [read.engine_messages, read.warnings].concat();
    Ok((canonical_from_segmented(&doc), engine_notice(&messages)))
}

/// Convert `bytes` of the given kind to canonical text. Checks size first;
/// non-PDF kinds are decoded (the [`Decoded`] says how) before conversion.
/// A PDF's extraction stops at `pdf_limit` (bmlib's MuPDF backend checks it
/// while a page runs) with an `IntakeError::Pdf` of kind Cancelled or
/// TimedOut: a limit never yields a short text. MuPDF reading past damage
/// can, and the last element, a PDF's [`engine_notice`], says so (else
/// `None`); an `Empty` text carries it instead.
pub fn to_canonical(
    kind: SourceKind,
    bytes: &[u8],
    title_hint: Option<&str>,
    pdf: &dyn PdfTextExtractor,
    scratch_dir: &Path,
    pdf_limit: &PdfLimit,
) -> Result<(CanonicalText, Option<Decoded>, Option<String>), IntakeError> {
    check_size(kind, bytes.len())?;
    let (canonical, decoded, notice) = if kind == SourceKind::Pdf {
        let (canonical, notice) = pdf_to_canonical(bytes, title_hint, pdf, scratch_dir, pdf_limit)?;
        (canonical, None, notice)
    } else {
        let (text, decoded) = decode_text(kind, bytes)?;
        let canonical = match kind {
            SourceKind::Jats => {
                let article =
                    jats_reader::parse(&text).map_err(|e| IntakeError::Jats(e.to_string()))?;
                canonical_from_jats(&article)
            }
            SourceKind::Html => canonical_from_html(&text).map_err(IntakeError::Html)?,
            SourceKind::Markdown => canonical_from_markdown(&text),
            _ => canonical_from_plain_text(&text, title_hint.unwrap_or("")),
        };
        (canonical, Some(decoded), None)
    };
    if canonical.sections.iter().all(|s| s.blocks.is_empty()) {
        return Err(IntakeError::Empty(notice));
    }
    Ok((canonical, decoded, notice))
}

/// Check whether `text` looks like a whole paper (bmlib's full-text rule).
/// A refusal carries the PDF engine's `notice` too, since a damaged PDF is the
/// likely cause.
pub fn assess(
    text: &CanonicalText,
    notice: Option<&str>,
) -> Result<FullTextAssessment, IntakeError> {
    assess_full_text(text).map_err(|e| {
        IntakeError::Canonical(match notice {
            Some(n) => format!("{e}. {n}"),
            None => e.to_string(),
        })
    })
}

/// The identifiers a JATS document carries about itself, and a notice for
/// the user about each one dropped. Each is validated on its own; an invalid
/// one is dropped with a warning and a notice. Any failure to read the
/// document yields no identifiers and no notices (conversion reports the
/// real error).
pub fn jats_identifiers(bytes: &[u8]) -> (Identifiers, Vec<String>) {
    let Ok((text, _)) = decode_text(SourceKind::Jats, bytes) else {
        return Default::default();
    };
    let Ok(article) = jats_reader::parse(&text) else {
        return Default::default();
    };
    let notices = std::cell::RefCell::new(Vec::new());
    let one = |kind: &str, value: &str, pick: fn(Identifiers) -> Option<String>| {
        if value.trim().is_empty() {
            return None;
        }
        let ids = match kind {
            "PMID" => validate_identifiers(Some(value), None, None),
            "DOI" => validate_identifiers(None, Some(value), None),
            _ => validate_identifiers(None, None, Some(value)),
        };
        match ids {
            Ok(ids) => pick(ids),
            Err(e) => {
                log::warn!("ignoring invalid {kind} in the document: {e}");
                notices
                    .borrow_mut()
                    .push(format!("The document's own {kind} was not recorded: {e}"));
                None
            }
        }
    };
    let ids = Identifiers {
        pmid: one("PMID", &article.pmid, |i| i.pmid),
        doi: one("DOI", &article.doi, |i| i.doi),
        pmc_id: one("PMC ID", &article.pmc_id, |i| i.pmc_id),
    };
    (ids, notices.into_inner())
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
    use bmlib::fulltext::pdf_text::PdfBlocks;

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
        fn extract_blocks_within(
            &self,
            path: &Path,
            _: &PdfLimit,
        ) -> Result<Vec<TextBlock>, PdfError> {
            assert!(
                path.exists(),
                "the scratch file must exist while extracting"
            );
            if self.fail {
                return Err(PdfError::unreadable("no pdf here"));
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
        fn page_texts_within(&self, _: &Path, _: &PdfLimit) -> Result<Vec<String>, PdfError> {
            Ok(vec![])
        }
    }

    /// A fresh empty scratch directory.
    fn scratch() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sa-intake-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A file is a PDF by its magic bytes whatever its name, else its kind is its
    /// extension's (any case); an unknown extension is unsupported, and a `.pdf`
    /// without the magic is `NotPdf`.
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
        assert_eq!(
            detect_kind(Some("a.PDF"), b"%PDF-1.7").unwrap(),
            SourceKind::Pdf
        );
        match detect_kind(Some("a.PDF"), b"<html>") {
            Err(e @ IntakeError::NotPdf(_)) => assert!(e.to_string().starts_with("a.PDF is named")),
            other => panic!("expected NotPdf, got {other:?}"),
        }
    }

    /// A paste has no name, so its kind is read off its text: an XML declaration
    /// or `<article>` is JATS, a doctype or `<html>` is HTML (both read off the
    /// first characters), any line starting `# ` makes it Markdown, anything else
    /// plain text.
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

    /// Every kind parses back from the name it is stored under; an unknown name
    /// is `None`.
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

    /// Bytes that are not UTF-8 and carry no BOM or declaration are guessed (here
    /// windows-1252, flagged as a guess); a
    /// UTF-16LE or UTF-8 BOM decides the encoding, not a guess.
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

    /// A BOM promises an encoding; bytes that break it are `Undecodable`, never
    /// a guess or replacement characters. A UTF-16BE BOM decodes too.
    #[test]
    fn broken_bom_is_undecodable() {
        let odd = [0xFF, 0xFE, b'a', 0, b'b'];
        assert!(matches!(
            decode_text(SourceKind::PlainText, &odd),
            Err(IntakeError::Undecodable)
        ));
        let bad_utf8 = b"\xEF\xBB\xBFok \xFF\xFE";
        assert!(matches!(
            decode_text(SourceKind::PlainText, bad_utf8),
            Err(IntakeError::Undecodable)
        ));
        let mut be = vec![0xFE, 0xFF];
        be.extend("café".encode_utf16().flat_map(|c| c.to_be_bytes()));
        let (t, d) = decode_text(SourceKind::PlainText, &be).unwrap();
        assert_eq!((t.as_str(), d.encoding.as_str()), ("café", "UTF-16BE"));
    }

    /// Each kind's cap admits exactly its limit and refuses one byte more.
    #[test]
    fn size_caps_per_kind() {
        for (kind, max) in [
            (SourceKind::Pdf, MAX_PDF_BYTES),
            (SourceKind::Jats, MAX_TEXT_BYTES),
            (SourceKind::Markdown, MAX_TEXT_BYTES),
            (SourceKind::PlainText, MAX_TEXT_BYTES),
        ] {
            assert!(check_size(kind, max).is_ok(), "{kind:?}");
            assert!(
                matches!(check_size(kind, max + 1), Err(IntakeError::TooLarge { .. })),
                "{kind:?}"
            );
        }
    }

    /// An HTML `<meta charset>` or an XML declaration decides the encoding, not a
    /// guess; a label no encoding answers to falls through to a guess.
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

    /// A declared encoding the bytes do not decode under falls through to a guess.
    #[test]
    fn contradicted_declaration_is_guessed() {
        // EUC-KR needs a trail byte of at least 0x41 after 0xE9; a space is not one.
        let xml = b"<?xml version=\"1.0\" encoding=\"EUC-KR\"?><a>caf\xe9 au lait, cr\xe8me br\xfbl\xe9e</a>";
        let (t, d) = decode_text(SourceKind::Jats, xml).unwrap();
        assert!(t.contains("café au lait"), "{t}");
        assert!(d.guessed);
        assert_ne!(d.encoding, "EUC-KR");
    }

    /// The XML declaration's encoding is read only inside the declaration and
    /// only within the prescan window.
    #[test]
    fn xml_declared_label_bounds() {
        assert_eq!(
            xml_declared_label(b"<?xml version='1.0' encoding = 'Shift_JIS' ?><a/>").as_deref(),
            Some("Shift_JIS")
        );
        assert_eq!(
            xml_declared_label(b"<?xml version=\"1.0\"?><a encoding=\"latin1\"/>"),
            None
        );
        let mut late = vec![b' '; HTML_CHARSET_PRESCAN_BYTES];
        late.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"latin1\"?>");
        assert_eq!(xml_declared_label(&late), None);
    }

    /// A binary file (here a ZIP's header) is refused as `Binary`, not decoded.
    #[test]
    fn binary_refused() {
        assert_eq!(
            decode_text(SourceKind::PlainText, b"PK\x03\x04\x00\x00").unwrap_err(),
            IntakeError::Binary
        );
    }

    /// A guessed decoding full of control characters is binary, not text.
    #[test]
    fn guessed_text_with_control_characters_is_binary() {
        let mut b = b"\x01\x02\x03abc\x1b\x7f".to_vec();
        b.extend_from_slice(b"\xe9\xe8\xfb\xe9 caf\xe9 cr\xe8me");
        assert_eq!(
            decode_text(SourceKind::PlainText, &b).unwrap_err(),
            IntakeError::Binary
        );
    }

    /// Without a BOM a UTF-16 declaration cannot be true (WHATWG reads it as
    /// UTF-8), so it is ignored and the encoding guessed.
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

    /// A scratch directory that cannot be written (here a file, not a folder) is
    /// `IntakeError::Io`, not a PDF fault.
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
            &PdfLimit::default(),
        );
        assert!(matches!(r, Err(IntakeError::Io(_))));
        std::fs::remove_file(&file).unwrap();
    }

    /// A page past `MAX_HTML_BYTES` is `TooLarge` before html5ever sees it, and
    /// the error names the cap.
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
        let err = to_canonical(
            SourceKind::Html,
            &big,
            None,
            &FakePdf { fail: false },
            &std::env::temp_dir(),
            &PdfLimit::default(),
        )
        .unwrap_err();
        assert!(matches!(err, IntakeError::TooLarge { .. }));
        assert!(err.to_string().contains("22.0 MB"));
    }

    /// A JATS file's own PMID and DOI are read; an invalid one is dropped with a
    /// notice saying why, and a file that does not parse gives none.
    #[test]
    fn jats_identifiers_are_read_and_validated() {
        let (ids, notices) = jats_identifiers(JATS);
        assert_eq!(ids.pmid.as_deref(), Some("12345678"));
        assert_eq!(ids.doi.as_deref(), Some("10.1234/fixture.1"));
        assert_eq!(ids.pmc_id, None);
        assert!(notices.is_empty());
        let bad = String::from_utf8_lossy(JATS).replace("12345678", "not-a-pmid");
        let (ids, notices) = jats_identifiers(bad.as_bytes());
        assert_eq!(ids.pmid, None);
        assert_eq!(ids.doi.as_deref(), Some("10.1234/fixture.1"));
        assert_eq!(notices.len(), 1);
        assert_eq!(
            notices[0],
            "The document's own PMID was not recorded: \"not-a-pmid\" is not a valid PMID."
        );
        assert_eq!(jats_identifiers(b"garbage"), Default::default());
    }

    /// Each text fixture converts under its kind's current converter version and
    /// keeps its body; none has an engine notice.
    #[test]
    fn each_fixture_converts() {
        let pdf = FakePdf { fail: false };
        for (kind, bytes, sentence) in [
            (SourceKind::Jats, JATS, "fixture trial"),
            (SourceKind::Html, HTML, "saved page fixture"),
            (SourceKind::Markdown, MD, "markdown fixture"),
            (SourceKind::PlainText, TXT, "plain text fixture"),
        ] {
            let (c, _, notice) = to_canonical(
                kind,
                bytes,
                None,
                &pdf,
                &std::env::temp_dir(),
                &PdfLimit::default(),
            )
            .unwrap();
            assert_eq!(notice, None, "{kind:?}: only a PDF has an engine notice");
            assert_eq!(
                c.converter_version,
                current_converter_version(kind),
                "{kind:?}"
            );
            assert!(c.text.contains(sentence), "{kind:?}");
        }
    }

    /// A PDF's own `/Title`, printed on page 1, beats a larger masthead line.
    #[test]
    fn pdf_title_is_its_own_title_not_the_masthead() {
        /// Page 1 of a journal: the masthead set larger than the title.
        struct Masthead;
        impl PdfTextExtractor for Masthead {
            fn name(&self) -> &'static str {
                "masthead"
            }
            fn extract_blocks_within(
                &self,
                _: &Path,
                _: &PdfLimit,
            ) -> Result<Vec<TextBlock>, PdfError> {
                Ok(vec![
                    block("Life Sciences", 24.0, true, 750.0),
                    block("Tablets lower the score in adults", 14.0, true, 700.0),
                    block(
                        "We randomised adults to tablets or placebo.",
                        10.0,
                        false,
                        650.0,
                    ),
                    block("The tablets lowered the score.", 10.0, false, 600.0),
                ])
            }
            fn extract_blocks_with_warnings(
                &self,
                path: &Path,
                limit: &PdfLimit,
            ) -> Result<PdfBlocks, PdfError> {
                Ok(PdfBlocks {
                    blocks: self.extract_blocks_within(path, limit)?,
                    metadata_title: Some("Tablets lower the score in adults".into()),
                    ..Default::default()
                })
            }
            fn page_texts_within(&self, _: &Path, _: &PdfLimit) -> Result<Vec<String>, PdfError> {
                Ok(vec![])
            }
        }
        let (c, _, _) = to_canonical(
            SourceKind::Pdf,
            b"%PDF-1.7",
            Some("download"),
            &Masthead,
            &scratch(),
            &PdfLimit::default(),
        )
        .unwrap();
        let span = c.title_span.expect("a title");
        let title: String = c
            .text
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        assert_eq!(title, "Tablets lower the score in adults");
    }

    /// The JATS fixture passes `assess_full_text`, with at least `MIN_BODY_WORDS`
    /// body words.
    #[test]
    fn jats_fixture_is_full_text() {
        let (c, _, _) = to_canonical(
            SourceKind::Jats,
            JATS,
            None,
            &FakePdf { fail: false },
            &std::env::temp_dir(),
            &PdfLimit::default(),
        )
        .unwrap();
        let a = assess(&c, None).unwrap();
        assert!(a.is_full_text, "{a:?}");
        assert!(a.body_words >= MIN_BODY_WORDS);
    }

    /// XML that does not parse is `IntakeError::Jats`.
    #[test]
    fn malformed_jats_is_a_jats_error() {
        let r = to_canonical(
            SourceKind::Jats,
            b"<article><a></article>",
            None,
            &FakePdf { fail: false },
            &std::env::temp_dir(),
            &PdfLimit::default(),
        );
        assert!(matches!(r, Err(IntakeError::Jats(_))));
    }

    /// A PDF is read through a scratch file that is gone afterwards, whether the
    /// extraction succeeds or fails, and its decoding is `None`.
    #[test]
    fn pdf_via_extractor_cleans_scratch() {
        let tmp = scratch();
        let (c, d, _) = to_canonical(
            SourceKind::Pdf,
            b"%PDF-1.4 x",
            Some("T"),
            &FakePdf { fail: false },
            &tmp,
            &PdfLimit::default(),
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
            &PdfLimit::default(),
        );
        assert!(matches!(r, Err(IntakeError::Pdf(_))));
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// A text of only whitespace is `Empty`, not a paper.
    #[test]
    fn empty_text_refused() {
        let r = to_canonical(
            SourceKind::PlainText,
            b"  \n\n ",
            None,
            &FakePdf { fail: false },
            &std::env::temp_dir(),
            &PdfLimit::default(),
        );
        assert!(matches!(r, Err(IntakeError::Empty(None))));
    }

    /// Intake errors read for a user: sizes in MB, the PDF engine's message, and a
    /// kind-only wording when the message is empty.
    #[test]
    fn errors_read_for_users() {
        let e = IntakeError::TooLarge {
            kind: SourceKind::Pdf,
            len: 150 * 1024 * 1024,
            max: 100 * 1024 * 1024,
        };
        let s = e.to_string();
        assert!(s.contains("150.0 MB") && s.contains("100.0 MB"), "{s}");
        let e = IntakeError::Pdf(PdfError::engine("out of memory"));
        assert_eq!(e.to_string(), "The PDF engine failed: out of memory");
        let mut e = PdfError::engine("x");
        e.message.clear();
        assert_eq!(
            IntakeError::Pdf(e).to_string(),
            "The PDF extraction failed (engine)."
        );
        let e =
            PdfError::unreadable("no objects").with_engine_messages(vec!["a".into(), "a".into()]);
        assert_eq!(
            IntakeError::Pdf(e).to_string(),
            format!("The PDF could not be read: no objects. {ENGINE_NOTICE_LEAD}: a (2 times).")
        );
    }

    /// Only the document's own faults ask for another file (#11).
    #[test]
    fn whose_fault_an_intake_error_is() {
        assert!(IntakeError::Pdf(PdfError::unreadable("x")).is_the_documents());
        assert!(IntakeError::Pdf(PdfError::encrypted()).is_the_documents());
        assert!(!IntakeError::Pdf(PdfError::engine("x")).is_the_documents());
        assert!(!IntakeError::Pdf(PdfError::cancelled()).is_the_documents());
        assert!(!IntakeError::Io("x".into()).is_the_documents());
        assert!(!IntakeError::Canonical("x".into()).is_the_documents());
        assert!(IntakeError::Empty(None).is_the_documents());
        assert!(IntakeError::Jats("x".into()).is_the_documents());
        assert!(IntakeError::Html(HtmlError::TimedOut {
            parsed: 1,
            length: 2
        })
        .is_the_documents());
        assert!(IntakeError::NotPdf("x.pdf".into()).is_the_documents());
    }

    /// A page too long to parse names its length and points to pasting.
    #[test]
    fn a_page_too_long_to_parse_says_so() {
        let m = IntakeError::Html(HtmlError::TooLarge { length: 3_000_000 }).to_string();
        assert!(m.starts_with("The web page could not be read: the page is 3000000 characters"));
        assert!(m.ends_with("Paste its text instead."));
    }

    /// MuPDF reads bmlib's contract PDF: a soft hyphen joined, running
    /// headers set aside, the current converter version, scratch left empty.
    #[test]
    fn mupdf_reads_a_pdf() {
        const PDF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/paper.pdf"
        ));
        let pdf = bmlib::fulltext::mupdf_backend::MupdfExtractor::new();
        let tmp = scratch();
        let (c, _, notice) =
            to_canonical(SourceKind::Pdf, PDF, None, &pdf, &tmp, &PdfLimit::default()).unwrap();
        assert_eq!(notice, None, "a sound PDF reports nothing");
        assert!(c.text.contains("prevalent in adults"), "{}", c.text);
        assert!(!c.text.contains("Smith et al."), "{}", c.text);
        assert_eq!(
            c.converter_version,
            current_converter_version(SourceKind::Pdf)
        );
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// bmlib's contract PDF whose page MuPDF cuts short (#599) converts what
    /// was read before the damage, with a notice folding the hundred repeats.
    #[test]
    fn mupdf_reports_a_page_cut_short() {
        const PDF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pdf_warnings_cut_short.pdf"
        ));
        let pdf = bmlib::fulltext::mupdf_backend::MupdfExtractor::new();
        let tmp = scratch();
        let (c, _, notice) =
            to_canonical(SourceKind::Pdf, PDF, None, &pdf, &tmp, &PdfLimit::default()).unwrap();
        assert!(c.text.contains("Read before the damage."), "{}", c.text);
        let notice = notice.expect("the engine's messages are a notice");
        assert!(
            notice.contains("syntax error in content stream (100 times)"),
            "{notice}"
        );
        assert!(
            notice.contains("too many syntax errors; ignoring rest of page"),
            "{notice}"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// A page cut short before any text is `Empty` that still says what the
    /// engine reported, the only clue to why no text was found.
    #[test]
    fn mupdf_empty_page_keeps_its_notice() {
        const PDF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pdf_warnings_cut_short.pdf"
        ));
        // Blanked to the same length, so the xref offsets still hold.
        let text = b"(Read before the damage.) Tj";
        let at = PDF.windows(text.len()).position(|w| w == text).unwrap();
        let mut bytes = PDF.to_vec();
        bytes[at..at + text.len()].fill(b' ');
        let pdf = bmlib::fulltext::mupdf_backend::MupdfExtractor::new();
        let tmp = scratch();
        let r = to_canonical(
            SourceKind::Pdf,
            &bytes,
            None,
            &pdf,
            &tmp,
            &PdfLimit::default(),
        );
        match r {
            Err(e @ IntakeError::Empty(Some(_))) => {
                let m = e.to_string();
                assert!(m.starts_with("No text was found in this source. "), "{m}");
                assert!(m.contains("ignoring rest of page"), "{m}");
            }
            other => panic!("expected Empty with a notice, got {other:?}"),
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// A PDF MuPDF fails to read (bmlib's `pdf_warnings_unreadable.pdf`: a
    /// header, no objects) still says what MuPDF tried first (bmlib #619).
    #[test]
    fn mupdf_failed_read_keeps_its_messages() {
        let pdf = bmlib::fulltext::mupdf_backend::MupdfExtractor::new();
        let tmp = scratch();
        let r = to_canonical(
            SourceKind::Pdf,
            b"%PDF-1.4\nnot a pdf at all\n",
            None,
            &pdf,
            &tmp,
            &PdfLimit::default(),
        );
        let m = r.unwrap_err().to_string();
        assert!(m.contains(ENGINE_NOTICE_LEAD), "{m}");
        assert!(m.contains("trying to repair broken xref"), "{m}");
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// A refusal of the text carries the engine's notice; without one it is
    /// bmlib's message alone.
    #[test]
    fn assess_refusal_carries_the_notice() {
        // Spans that no longer tile the text fail bmlib's validation.
        let mut c = canonical_from_plain_text("Para.", "T");
        c.text.push_str(" stray");
        let bare = assess(&c, None).unwrap_err().to_string();
        let noted = assess(&c, Some("The engine said so."))
            .unwrap_err()
            .to_string();
        assert!(!bare.contains("engine"), "{bare}");
        assert!(
            noted.starts_with(&bare) && noted.ends_with(". The engine said so."),
            "{noted}"
        );
    }

    /// The notice keeps every message in order, a run of identical ones once
    /// with its count; nothing reported is no notice.
    #[test]
    fn engine_notice_folds_runs() {
        assert_eq!(engine_notice(&[]), None);
        let w = |s: &str| s.to_string();
        let n = engine_notice(&[w("a"), w("b"), w("b"), w("a")]).unwrap();
        assert!(n.ends_with(": a; b (2 times); a."), "{n}");
        assert!(is_engine_notice(&n));
        assert!(!is_engine_notice("The DOI given was invalid."));
    }

    /// MuPDF stops at a limit already reached: a cancel, never a short text.
    #[test]
    fn mupdf_stops_at_a_cancel() {
        const PDF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/paper.pdf"
        ));
        let pdf = bmlib::fulltext::mupdf_backend::MupdfExtractor::new();
        let limit = PdfLimit {
            cancel: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                true,
            ))),
            deadline: None,
        };
        let tmp = scratch();
        let r = to_canonical(SourceKind::Pdf, PDF, None, &pdf, &tmp, &limit);
        assert!(
            matches!(&r, Err(IntakeError::Pdf(e)) if e.kind == bmlib::fulltext::pdf_text::PdfErrorKind::Cancelled),
            "{r:?}"
        );
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// Manual check: STUDYANALYZER_TEST_PDF names a real PDF.
    #[test]
    #[ignore]
    fn real_pdf() {
        let path = std::env::var("STUDYANALYZER_TEST_PDF").expect("set STUDYANALYZER_TEST_PDF");
        let bytes = std::fs::read(path).unwrap();
        let pdf = bmlib::fulltext::mupdf_backend::MupdfExtractor::new();
        let tmp = scratch();
        let (c, _, notice) = to_canonical(
            SourceKind::Pdf,
            &bytes,
            None,
            &pdf,
            &tmp,
            &PdfLimit::default(),
        )
        .unwrap();
        println!(
            "{} chars, {} sections",
            c.text.chars().count(),
            c.sections.len()
        );
        println!("engine: {notice:?}");
        assert!(!c.text.is_empty());
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
