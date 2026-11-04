//! The hand-off format: how another app gives StudyAnalyzer a paper, as a
//! `.studyanalyzer` JSON file or a `studyanalyzer://open?…` link. This module
//! only parses and validates (a trust boundary); storing and opening a
//! hand-off live elsewhere. The format is documented in `docs/handoff-format.md`.

use std::fmt;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;

use crate::config::{
    BIDI_CONTROLS, HANDOFF_FORMAT, HANDOFF_VERSION, LINK_HOST, LINK_ORIGIN, LINK_SCHEME,
    MAX_HANDOFF_BYTES, MAX_HANDOFF_LABEL_CHARS, MAX_HANDOFF_TITLE_CHARS, MAX_LINK_CHARS, UTF8_BOM,
};
use crate::identifiers::{check_handed, Identifiers};
use crate::intake::SourceKind;

/// A validated hand-off: who sent it, what it names, and maybe the full text.
#[derive(Debug, Clone, PartialEq)]
pub struct Handoff {
    /// The sending app (`None` for a link, which carries no sender).
    pub sender: Option<String>,
    /// The title the sender already had, if any.
    pub title: Option<String>,
    /// The identifiers that passed their checks.
    pub identifiers: Identifiers,
    /// One sentence per identifier that was dropped, for the user.
    pub notices: Vec<String>,
    /// The full text, if the sender included it.
    pub document: Option<HandedDocument>,
}

/// The full text a hand-off carried, still unconverted.
#[derive(Debug, Clone, PartialEq)]
pub struct HandedDocument {
    /// The kind of file, from the hand-off's `full_text.format`.
    pub kind: SourceKind,
    /// The sender's file name (a name only, a title hint and nothing else).
    pub file_name: Option<String>,
    /// Where the sender got the text (shown, never followed).
    pub source: Option<String>,
    /// The file's bytes, decoded from base64.
    pub bytes: Vec<u8>,
}

impl Handoff {
    /// Where the paper came from, for the user: "{sender}, from {source}",
    /// "{sender}", or [`LINK_ORIGIN`] when there is no sender.
    pub fn origin(&self) -> String {
        let source = self.document.as_ref().and_then(|d| d.source.as_deref());
        match (self.sender.as_deref(), source) {
            (Some(sender), Some(source)) => format!("{sender}, from {source}"),
            (Some(sender), None) => sender.to_string(),
            (None, _) => LINK_ORIGIN.to_string(),
        }
    }
}

/// Why a hand-off was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffError {
    /// The file is longer than `max` bytes (it was not parsed).
    TooLarge {
        /// The file's length in bytes.
        len: usize,
        /// The most that is read.
        max: usize,
    },
    /// Valid JSON, but not a hand-off (`format` missing or different).
    NotAHandoff,
    /// Made to a newer format version than this app reads.
    Newer {
        /// The file's version.
        version: u64,
    },
    /// Not valid: the text names the field and the rule it broke.
    Malformed(String),
    /// No valid identifier and no full text; the reasons each was refused.
    Nothing {
        /// One sentence per refused identifier.
        refused: Vec<String>,
    },
    /// A bad `studyanalyzer://` link; the text says why.
    Link(String),
}

impl fmt::Display for HandoffError {
    /// A sentence the user can read.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { len, max } => write!(
                f,
                "The hand-off file is {len} bytes long; the limit is {max} bytes."
            ),
            Self::NotAHandoff => write!(f, "This is not a StudyAnalyzer hand-off file."),
            Self::Newer { version } => write!(
                f,
                "This hand-off is format version {version}, made by a newer version of its sender; \
                 this StudyAnalyzer reads version {HANDOFF_VERSION}. Update StudyAnalyzer."
            ),
            Self::Malformed(why) => write!(f, "The hand-off file is not valid: {why}"),
            Self::Nothing { refused } => {
                write!(
                    f,
                    "The hand-off carries no valid identifier and no full text."
                )?;
                refused.iter().try_for_each(|r| write!(f, " {r}"))
            }
            Self::Link(why) => write!(f, "The link cannot be opened: {why}"),
        }
    }
}

impl std::error::Error for HandoffError {}

/// A nested object that is either well formed or not; either way it parses,
/// so a wrong shape is reported by field name (and a newer version's changed
/// shape does not hide `Newer`). Buffered, so a repeated key is still seen.
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient<T> {
    /// An array, which serde would otherwise read as the struct's fields in
    /// order; it must be refused, so it is matched first.
    #[allow(dead_code)] // only matched, never read
    List(Vec<serde::de::IgnoredAny>),
    /// The object, as expected.
    Good(T),
    /// Anything else (not an object, or a repeated key inside it).
    Bad(serde::de::IgnoredAny),
}

/// The hand-off JSON as written, field by field. Leaf values stay
/// `serde_json::Value` so a wrong type is reported by us, by field name,
/// never as serde's text (which echoes the input). Unknown fields are ignored
/// on purpose; a repeated known key is an error.
#[derive(Deserialize)]
struct Raw {
    format: Option<Value>,
    version: Option<Value>,
    sender: Option<Value>,
    title: Option<Value>,
    identifiers: Option<Lenient<RawIds>>,
    full_text: Option<Lenient<RawText>>,
}

/// The `identifiers` object.
#[derive(Deserialize)]
struct RawIds {
    pmid: Option<Value>,
    doi: Option<Value>,
    pmcid: Option<Value>,
}

/// The `full_text` object.
#[derive(Deserialize)]
struct RawText {
    format: Option<Value>,
    data: Option<Value>,
    file_name: Option<Value>,
    source: Option<Value>,
}

/// A field that must be a string (or absent or null): its text.
fn string_field(field: &str, value: Option<Value>) -> Result<Option<String>, HandoffError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(HandoffError::Malformed(format!(
            "`{field}` must be a string."
        ))),
    }
}

/// The contents of an optional nested object, or a refusal naming it.
fn object_field<T>(field: &str, value: Option<Lenient<T>>) -> Result<Option<T>, HandoffError> {
    match value {
        None => Ok(None),
        Some(Lenient::Good(v)) => Ok(Some(v)),
        Some(Lenient::List(_) | Lenient::Bad(_)) => Err(HandoffError::Malformed(format!(
            "`{field}` must be an object with no repeated key."
        ))),
    }
}

/// Refuse `value` if it has more than `max` characters or, unless
/// `control_ok`, a control character or one of [`BIDI_CONTROLS`] (named by
/// its code point); nothing is ever cut to fit.
fn check_text(field: &str, value: &str, max: usize, control_ok: bool) -> Result<(), HandoffError> {
    let count = value.chars().count();
    if count > max {
        return Err(HandoffError::Malformed(format!(
            "{field} is {count} characters long; the limit is {max}."
        )));
    }
    if !control_ok && value.chars().any(char::is_control) {
        return Err(HandoffError::Malformed(format!(
            "{field} contains a control character."
        )));
    }
    if let Some(c) = value
        .chars()
        .find(|c| !control_ok && BIDI_CONTROLS.contains(c))
    {
        return Err(HandoffError::Malformed(format!(
            "{field} contains a bidirectional control character (U+{:04X}).",
            u32::from(c)
        )));
    }
    Ok(())
}

/// Check the three identifiers each on its own: the ones that pass, one notice
/// per dropped one, and (what `Nothing` reports) the same sentences.
fn check_ids(
    pmid: Option<&str>,
    doi: Option<&str>,
    pmc: Option<&str>,
) -> (Identifiers, Vec<String>) {
    let mut notices = Vec::new();
    let mut one = |kind: &str, ids: (Identifiers, Vec<crate::identifiers::IdentifierError>)| {
        for e in ids.1 {
            log::warn!("hand-off: dropping the {kind}: {e}");
            notices.push(format!("The hand-off's {kind} was not recorded: {e}"));
        }
        ids.0
    };
    let p = one("PMID", check_handed(pmid, None, None));
    let d = one("DOI", check_handed(None, doi, None));
    let c = one("PMC ID", check_handed(None, None, pmc));
    let ids = Identifiers {
        pmid: p.pmid,
        doi: d.doi,
        pmc_id: c.pmc_id,
    };
    (ids, notices)
}

/// Whether `ids` names anything.
fn has_any(ids: &Identifiers) -> bool {
    ids.pmid.is_some() || ids.doi.is_some() || ids.pmc_id.is_some()
}

/// Map `full_text.format` to a kind.
fn kind_of(format: Option<&str>) -> Result<SourceKind, HandoffError> {
    match format {
        Some("pdf") => Ok(SourceKind::Pdf),
        Some("jats") => Ok(SourceKind::Jats),
        Some("html") => Ok(SourceKind::Html),
        Some("markdown") => Ok(SourceKind::Markdown),
        Some("text") => Ok(SourceKind::PlainText),
        _ => Err(HandoffError::Malformed(
            "`full_text.format` must be one of pdf, jats, html, markdown, text.".into(),
        )),
    }
}

/// Turn a blank optional string into `None`.
fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// Parse and validate a `.studyanalyzer` file. A leading UTF-8 BOM is
/// skipped.
///
/// # Errors
///
/// A [`HandoffError`] naming the first rule the file breaks. The length is
/// checked before anything is parsed, and anything but a JSON object (an
/// array, other text) is [`HandoffError::NotAHandoff`]. No message repeats
/// text from the file.
pub fn parse_handoff(bytes: &[u8]) -> Result<Handoff, HandoffError> {
    if bytes.len() > MAX_HANDOFF_BYTES {
        return Err(HandoffError::TooLarge {
            len: bytes.len(),
            max: MAX_HANDOFF_BYTES,
        });
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    // serde reads a struct from an array too, so only an object is let through.
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err(HandoffError::NotAHandoff);
    }
    let raw: Raw = serde_json::from_slice(bytes).map_err(|e| {
        let text = e.to_string();
        // Syntax errors and repeated keys carry no input text; a type error
        // at the top level could, so it gets a fixed sentence.
        if e.is_data() && !text.starts_with("duplicate field") {
            HandoffError::Malformed("the top level must be a JSON object.".into())
        } else {
            HandoffError::Malformed(text)
        }
    })?;
    if raw.format.as_ref().and_then(Value::as_str) != Some(HANDOFF_FORMAT) {
        return Err(HandoffError::NotAHandoff);
    }
    let version = match raw.version.as_ref().and_then(Value::as_u64) {
        Some(v) if v >= 1 => v,
        _ => {
            return Err(HandoffError::Malformed(
                "`version` must be a positive integer.".into(),
            ))
        }
    };
    if version > HANDOFF_VERSION {
        return Err(HandoffError::Newer { version });
    }

    let sender = string_field("sender", raw.sender)?
        .ok_or_else(|| HandoffError::Malformed("`sender` is missing.".into()))?;
    if sender.trim().is_empty() {
        return Err(HandoffError::Malformed("`sender` is empty.".into()));
    }
    check_text("sender", &sender, MAX_HANDOFF_LABEL_CHARS, false)?;
    let title = non_blank(string_field("title", raw.title)?);
    if let Some(t) = &title {
        check_text("title", t, MAX_HANDOFF_TITLE_CHARS, true)?;
    }

    let ids = object_field("identifiers", raw.identifiers)?;
    let (pmid, doi, pmc) = match ids {
        Some(i) => (
            string_field("identifiers.pmid", i.pmid)?,
            string_field("identifiers.doi", i.doi)?,
            string_field("identifiers.pmcid", i.pmcid)?,
        ),
        None => (None, None, None),
    };
    let (identifiers, notices) = check_ids(pmid.as_deref(), doi.as_deref(), pmc.as_deref());

    let document = object_field("full_text", raw.full_text)?
        .map(document_of)
        .transpose()?;
    if document.is_none() && !has_any(&identifiers) {
        return Err(HandoffError::Nothing { refused: notices });
    }
    Ok(Handoff {
        sender: Some(sender),
        title,
        identifiers,
        notices,
        document,
    })
}

/// Validate and decode a `full_text` object.
fn document_of(raw: RawText) -> Result<HandedDocument, HandoffError> {
    let format = string_field("full_text.format", raw.format)?;
    let kind = kind_of(format.as_deref())?;
    let file_name = non_blank(string_field("full_text.file_name", raw.file_name)?);
    if let Some(name) = &file_name {
        check_text("full_text.file_name", name, MAX_HANDOFF_LABEL_CHARS, false)?;
        if name.contains(['/', '\\']) {
            return Err(HandoffError::Malformed(
                "`full_text.file_name` must be a name, not a path.".into(),
            ));
        }
    }
    let source = non_blank(string_field("full_text.source", raw.source)?);
    if let Some(s) = &source {
        check_text("full_text.source", s, MAX_HANDOFF_LABEL_CHARS, false)?;
    }
    let data = string_field("full_text.data", raw.data)?
        .ok_or_else(|| HandoffError::Malformed("`full_text.data` is missing.".into()))?;
    let bytes = STANDARD.decode(data.as_bytes()).map_err(|_| {
        HandoffError::Malformed("`full_text.data` is not valid standard base64.".into())
    })?;
    if bytes.is_empty() {
        return Err(HandoffError::Malformed("`full_text.data` is empty.".into()));
    }
    Ok(HandedDocument {
        kind,
        file_name,
        source,
        bytes,
    })
}

/// Parse and validate a `studyanalyzer://open?pmid=…&doi=…&pmcid=…` link.
///
/// # Errors
///
/// [`HandoffError::Link`] for a wrong scheme or host, a link that is too long,
/// a repeated key or an empty value; [`HandoffError::Nothing`] when no
/// identifier passes.
pub fn parse_link(link: &str) -> Result<Handoff, HandoffError> {
    let bad = |why: String| HandoffError::Link(why);
    let count = link.chars().count();
    if count > MAX_LINK_CHARS {
        return Err(bad(format!(
            "it is {count} characters long; the limit is {MAX_LINK_CHARS}."
        )));
    }
    let url = url::Url::parse(link).map_err(|e| bad(format!("it is not a valid link ({e}).")))?;
    if url.scheme() != LINK_SCHEME || url.host_str() != Some(LINK_HOST) {
        return Err(bad(format!(
            "it is not a {LINK_SCHEME}://{LINK_HOST} link."
        )));
    }
    if url.fragment().is_some() {
        return Err(bad(
            "it has a # fragment; a sender must percent-encode # inside a value as %23.".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return Err(bad("it must not have a user name, password or port.".into()));
    }
    if !matches!(url.path(), "" | "/") {
        return Err(bad("it must not have a path.".into()));
    }
    let (mut pmid, mut doi, mut pmc) = (None, None, None);
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "pmid" => &mut pmid,
            "doi" => &mut doi,
            "pmcid" => &mut pmc,
            other => {
                log::info!("link: ignoring unknown key {other:?}");
                continue;
            }
        };
        if value.trim().is_empty() {
            return Err(bad(format!("{key} is empty.")));
        }
        if slot.replace(value.into_owned()).is_some() {
            return Err(bad(format!("{key} is given twice.")));
        }
    }
    let (identifiers, notices) = check_ids(pmid.as_deref(), doi.as_deref(), pmc.as_deref());
    if !has_any(&identifiers) {
        return Err(HandoffError::Nothing { refused: notices });
    }
    Ok(Handoff {
        sender: None,
        title: None,
        identifiers,
        notices,
        document: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// A minimal valid hand-off value (identifiers only) to alter in tests.
    fn base() -> Value {
        json!({"format": "studyanalyzer-handoff", "version": 1, "sender": "MFC",
               "identifiers": {"pmid": "123"}})
    }

    /// Parse a JSON value as a file.
    fn parse(v: &Value) -> Result<Handoff, HandoffError> {
        parse_handoff(v.to_string().as_bytes())
    }

    /// The `Malformed` text of a refusal.
    fn malformed(v: &Value) -> String {
        match parse(v) {
            Err(HandoffError::Malformed(m)) => m,
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    /// Every field parses; bytes are the decoded data and the kind follows `format`.
    #[test]
    fn full_handoff_parses() {
        let v = json!({"format": "studyanalyzer-handoff", "version": 1, "sender": "MFC",
            "title": "A title",
            "identifiers": {"pmid": "0123", "doi": "10.1000/xyz", "pmcid": "pmc7"},
            "full_text": {"format": "markdown", "data": STANDARD.encode(b"# hi"),
                "file_name": "p.md", "source": "Europe PMC"}});
        let h = parse(&v).unwrap();
        assert_eq!(h.sender.as_deref(), Some("MFC"));
        assert_eq!(h.title.as_deref(), Some("A title"));
        assert_eq!(h.identifiers.pmid.as_deref(), Some("123"));
        assert_eq!(h.identifiers.doi.as_deref(), Some("10.1000/xyz"));
        assert_eq!(h.identifiers.pmc_id.as_deref(), Some("PMC7"));
        assert!(h.notices.is_empty());
        let d = h.document.unwrap();
        assert_eq!(d.kind, SourceKind::Markdown);
        assert_eq!(d.bytes, b"# hi");
        assert_eq!(d.file_name.as_deref(), Some("p.md"));
        assert_eq!(d.source.as_deref(), Some("Europe PMC"));
    }

    /// Identifiers alone are a hand-off.
    #[test]
    fn ids_only_handoff_parses() {
        let h = parse(&base()).unwrap();
        assert_eq!(h.document, None);
        assert_eq!(h.identifiers.pmid.as_deref(), Some("123"));
    }

    /// A preprint accession in the PMID field is kept.
    #[test]
    fn ppr_pmid_is_kept() {
        let mut v = base();
        v["identifiers"]["pmid"] = json!("PPR123");
        assert_eq!(
            parse(&v).unwrap().identifiers.pmid.as_deref(),
            Some("PPR123")
        );
    }

    /// A bad PMID is dropped with a notice that names it; the DOI stays.
    #[test]
    fn invalid_id_dropped_with_notice() {
        let mut v = base();
        v["identifiers"] = json!({"pmid": "nope", "doi": "10.1000/xyz"});
        let h = parse(&v).unwrap();
        assert_eq!(h.identifiers.pmid, None);
        assert_eq!(h.identifiers.doi.as_deref(), Some("10.1000/xyz"));
        assert_eq!(
            h.notices,
            vec!["The hand-off's PMID was not recorded: \"nope\" is not a valid PMID."]
        );
    }

    /// No valid identifier and no text is refused with the reasons.
    #[test]
    fn nothing_valid_is_refused() {
        let mut v = base();
        v["identifiers"] = json!({"pmid": "nope"});
        match parse(&v) {
            Err(HandoffError::Nothing { refused }) => {
                assert_eq!(refused.len(), 1);
                assert!(refused[0].contains("\"nope\""));
            }
            other => panic!("{other:?}"),
        }
        v.as_object_mut().unwrap().remove("identifiers");
        assert!(matches!(parse(&v), Err(HandoffError::Nothing { .. })));
    }

    /// The length is checked before parsing: junk of the wrong size is `TooLarge`.
    #[test]
    fn oversize_refused_before_parse() {
        let big = vec![b'{'; MAX_HANDOFF_BYTES + 1];
        assert_eq!(
            parse_handoff(&big),
            Err(HandoffError::TooLarge {
                len: MAX_HANDOFF_BYTES + 1,
                max: MAX_HANDOFF_BYTES
            })
        );
    }

    /// An object that is not valid JSON is malformed; text that does not
    /// start as an object is no hand-off at all.
    #[test]
    fn not_json_is_malformed() {
        assert!(matches!(
            parse_handoff(b"{ hello"),
            Err(HandoffError::Malformed(_))
        ));
        assert_eq!(parse_handoff(b"hello"), Err(HandoffError::NotAHandoff));
    }

    /// A top-level array is not a hand-off, though serde reads a struct
    /// from a sequence.
    #[test]
    fn top_level_array_is_not_a_handoff() {
        let v = json!(["studyanalyzer-handoff", 1, "MFC", null, {"pmid": "123"}]);
        assert_eq!(parse(&v), Err(HandoffError::NotAHandoff));
        assert_eq!(parse_handoff(b" \n[]"), Err(HandoffError::NotAHandoff));
    }

    /// A leading UTF-8 BOM (as Windows tools write) is skipped, also before
    /// whitespace.
    #[test]
    fn bom_is_accepted() {
        let mut bytes = b"\xEF\xBB\xBF \n".to_vec();
        bytes.extend(base().to_string().into_bytes());
        assert_eq!(
            parse_handoff(&bytes).unwrap().sender.as_deref(),
            Some("MFC")
        );
    }

    /// JSON of another format, or none, is not a hand-off.
    #[test]
    fn other_format_is_not_a_handoff() {
        let mut v = base();
        v["format"] = json!("something-else");
        assert_eq!(parse(&v), Err(HandoffError::NotAHandoff));
        assert_eq!(parse_handoff(b"{}"), Err(HandoffError::NotAHandoff));
    }

    /// A newer version is refused even when its shape has changed.
    #[test]
    fn newer_version_refused() {
        let v = json!({"format": "studyanalyzer-handoff", "version": 2, "sender": 7});
        let e = parse(&v).unwrap_err();
        assert_eq!(e, HandoffError::Newer { version: 2 });
        assert_eq!(
            e.to_string(),
            "This hand-off is format version 2, made by a newer version of its sender; \
             this StudyAnalyzer reads version 1. Update StudyAnalyzer."
        );
    }

    /// A duplicate key is malformed.
    #[test]
    fn duplicate_key_is_malformed() {
        let text = r#"{"format":"studyanalyzer-handoff","version":1,"sender":"a","sender":"b",
            "identifiers":{"pmid":"1"}}"#;
        assert!(matches!(
            parse_handoff(text.as_bytes()),
            Err(HandoffError::Malformed(m)) if m.contains("sender")
        ));
    }

    /// A field this version does not know is ignored.
    #[test]
    fn unknown_field_ignored() {
        let mut v = base();
        v["future"] = json!({"x": 1});
        assert!(parse(&v).is_ok());
    }

    /// Bad base64 is malformed, naming the field.
    #[test]
    fn bad_base64_is_malformed() {
        let mut v = base();
        v["full_text"] = json!({"format": "pdf", "data": "!!!"});
        assert!(malformed(&v).contains("full_text.data"));
    }

    /// An unknown text format is malformed.
    #[test]
    fn unknown_text_format_is_malformed() {
        let mut v = base();
        v["full_text"] = json!({"format": "docx", "data": "AA=="});
        assert!(malformed(&v).contains("full_text.format"));
    }

    /// A file name is a name: a slash, backslash or control character refuses it.
    #[test]
    fn path_in_file_name_is_malformed() {
        for name in ["../x.pdf", "a\\b.pdf", "a\u{0}.pdf"] {
            let mut v = base();
            v["full_text"] = json!({"format": "pdf", "data": "AA==", "file_name": name});
            assert!(malformed(&v).contains("full_text.file_name"), "{name:?}");
        }
    }

    /// An over-long title is refused, never cut.
    #[test]
    fn long_title_is_malformed() {
        let mut v = base();
        v["title"] = json!("t".repeat(MAX_HANDOFF_TITLE_CHARS + 1));
        let m = malformed(&v);
        assert!(m.contains("title") && m.contains(&MAX_HANDOFF_TITLE_CHARS.to_string()));
    }

    /// A control character in the sender is refused.
    #[test]
    fn control_char_in_sender_is_malformed() {
        let mut v = base();
        v["sender"] = json!("MFC\n");
        assert!(malformed(&v).contains("sender"));
    }

    /// A bidi override, embedding or isolate in a label is refused, naming
    /// it (#69); the title may carry one.
    #[test]
    fn bidi_control_in_label_is_malformed() {
        // Written out, so a character dropped from the constant fails here.
        let all = [
            '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}',
            '\u{2068}', '\u{2069}',
        ];
        assert_eq!(BIDI_CONTROLS, all);
        for c in all {
            let mut v = base();
            v["sender"] = json!(format!("MFC{c}cfm"));
            let m = malformed(&v);
            let code = format!("U+{:04X}", u32::from(c));
            assert_eq!(
                m,
                format!("sender contains a bidirectional control character ({code})."),
            );
        }
        for field in ["file_name", "source"] {
            let mut v = base();
            v["full_text"] = json!({"format": "pdf", "data": "AA==", field: "a\u{202E}b"});
            assert!(
                malformed(&v).contains(&format!("full_text.{field}")),
                "{field}"
            );
        }
        let mut v = base();
        v["title"] = json!("A\u{202E}B");
        assert!(parse(&v).is_ok());
    }

    /// A blank or missing sender is refused.
    #[test]
    fn empty_sender_is_malformed() {
        let mut v = base();
        v["sender"] = json!("  ");
        assert!(malformed(&v).contains("sender"));
        v.as_object_mut().unwrap().remove("sender");
        assert!(malformed(&v).contains("sender"));
    }

    /// The origin names the sender and source, the sender, or the link.
    #[test]
    fn origin_names_sender_and_source() {
        let mut h = parse(&base()).unwrap();
        assert_eq!(h.origin(), "MFC");
        h.document = Some(HandedDocument {
            kind: SourceKind::Pdf,
            file_name: None,
            source: Some("Europe PMC".into()),
            bytes: vec![1],
        });
        assert_eq!(h.origin(), "MFC, from Europe PMC");
        h.sender = None;
        assert_eq!(h.origin(), LINK_ORIGIN);
    }

    /// A link yields all three identifiers, percent-decoded.
    #[test]
    fn link_parses() {
        let h = parse_link("studyanalyzer://open?pmid=123&doi=10.1000%2Fxyz&pmcid=PMC9").unwrap();
        assert_eq!(h.identifiers.pmid.as_deref(), Some("123"));
        assert_eq!(h.identifiers.doi.as_deref(), Some("10.1000/xyz"));
        assert_eq!(h.identifiers.pmc_id.as_deref(), Some("PMC9"));
        assert_eq!((h.sender, h.title, h.document), (None, None, None));
    }

    /// Only the studyanalyzer scheme and open host are accepted.
    #[test]
    fn link_wrong_scheme_or_host() {
        for l in [
            "https://open?pmid=1",
            "studyanalyzer://elsewhere?pmid=1",
            "nonsense",
        ] {
            assert!(matches!(parse_link(l), Err(HandoffError::Link(_))), "{l}");
        }
    }

    /// A repeated key is refused.
    #[test]
    fn link_repeated_key() {
        assert!(matches!(
            parse_link("studyanalyzer://open?pmid=1&pmid=2"),
            Err(HandoffError::Link(m)) if m.contains("pmid")
        ));
    }

    /// An empty value is refused.
    #[test]
    fn link_empty_value() {
        assert!(matches!(
            parse_link("studyanalyzer://open?pmid="),
            Err(HandoffError::Link(_))
        ));
    }

    /// An unknown key is ignored.
    #[test]
    fn link_unknown_key_ignored() {
        let h = parse_link("studyanalyzer://open?pmid=1&future=x").unwrap();
        assert_eq!(h.identifiers.pmid.as_deref(), Some("1"));
    }

    /// A link past the cap is refused; one at exactly the cap is read (#67).
    #[test]
    fn link_too_long() {
        let l = format!("studyanalyzer://open?doi={}", "a".repeat(MAX_LINK_CHARS));
        assert!(matches!(parse_link(&l), Err(HandoffError::Link(m)) if m.contains("limit")));
        let head = "studyanalyzer://open?pmid=1&pad=";
        let at_cap = format!("{head}{}", "a".repeat(MAX_LINK_CHARS - head.len()));
        assert_eq!(at_cap.chars().count(), MAX_LINK_CHARS);
        assert_eq!(
            parse_link(&at_cap).unwrap().identifiers.pmid.as_deref(),
            Some("1")
        );
        let over = format!("{at_cap}a");
        assert!(matches!(parse_link(&over), Err(HandoffError::Link(m)) if m.contains("limit")));
    }

    /// A link with no usable identifier is `Nothing`.
    #[test]
    fn link_without_ids() {
        assert!(matches!(
            parse_link("studyanalyzer://open"),
            Err(HandoffError::Nothing { .. })
        ));
        assert!(matches!(
            parse_link("studyanalyzer://open?pmid=bad"),
            Err(HandoffError::Nothing { refused }) if refused.len() == 1
        ));
    }

    /// A `format` of the wrong type is "not a hand-off", not malformed.
    #[test]
    fn format_wrong_type_is_not_a_handoff() {
        let mut v = base();
        v["format"] = json!(7);
        assert_eq!(parse(&v), Err(HandoffError::NotAHandoff));
    }

    /// A wrong type names the field and never repeats the input.
    #[test]
    fn malformed_names_field_without_echo() {
        let mut v = base();
        v["identifiers"] = json!("x".repeat(100_000));
        let m = malformed(&v);
        assert!(m.contains("identifiers") && m.len() < 200, "{m}");
        let mut v = base();
        v["sender"] = json!(7);
        assert!(malformed(&v).contains("`sender`"));
        let mut v = base();
        v["full_text"] = json!({"format": "x".repeat(100_000), "data": "AA=="});
        let m = malformed(&v);
        assert!(m.contains("full_text.format") && m.len() < 200, "{m}");
    }

    /// `identifiers` must be an object, not an array.
    #[test]
    fn identifiers_array_is_malformed() {
        let mut v = base();
        v["identifiers"] = json!(["1", "10.1/x", null]);
        assert!(malformed(&v).contains("identifiers"));
    }

    /// `version` must be a positive integer.
    #[test]
    fn version_string_is_malformed() {
        for bad in [json!("1.0.0"), json!(0), json!(1.5)] {
            let mut v = base();
            v["version"] = bad;
            assert!(malformed(&v).contains("version"));
        }
    }

    /// A fragment is refused, with advice to encode `#`.
    #[test]
    fn link_with_fragment_refused() {
        assert!(matches!(
            parse_link("studyanalyzer://open?doi=10.1002/x;2-#"),
            Err(HandoffError::Link(m)) if m.contains("%23")
        ));
    }

    /// User info, a port or a path is refused; a lone `/` is fine.
    #[test]
    fn link_with_userinfo_port_or_path_refused() {
        for l in [
            "studyanalyzer://u:p@open?pmid=1",
            "studyanalyzer://u@open?pmid=1",
            "studyanalyzer://open:99?pmid=1",
            "studyanalyzer://open/a/b?pmid=1",
        ] {
            assert!(matches!(parse_link(l), Err(HandoffError::Link(_))), "{l}");
        }
        assert!(parse_link("studyanalyzer://open/?pmid=1").is_ok());
    }
}
