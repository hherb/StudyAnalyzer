//! Opening a paper: from a file, a paste or an identifier, and reopening or
//! completing one from the recent list. Orchestrates intake, retrieval and
//! storage; every request's writes commit together or not at all.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use bmlib::db::{fetch_one, transaction_with, Db, DbError, Value};
use bmlib::fulltext::canonical::{CanonicalText, FullTextAssessment};
use bmlib::fulltext::pdf_text::PdfTextExtractor;
use serde::Serialize;

use crate::config::{ORIGIN_PASTE, ORIGIN_UPLOAD, UNTITLED_PAPER};
use crate::identifiers::{parse_identifier, IdentifierError, Identifiers};
use crate::intake::{
    assess, current_converter_version, detect_kind, to_canonical, IntakeError, SourceKind,
};
use crate::retrieval::{retrieve, FullTextSource, Retrieved};
use crate::store::{
    add_identifiers, create_paper, find_paper, latest_canonical, latest_source, save_canonical,
    save_source, set_title, touch_opened, NewSource, PaperId, StoreError,
};

/// What the user (or a hand-off) asked to open.
#[derive(Debug, Clone)]
pub enum OpenRequest {
    /// A file the user chose; `name` is its file name when known.
    File {
        /// The file name, used to tell the kind and as a title fallback.
        name: Option<String>,
        /// The file's bytes.
        bytes: Vec<u8>,
    },
    /// Text the user pasted.
    Paste {
        /// The pasted text.
        text: String,
    },
    /// A PMID, DOI or PMC ID typed in or handed over.
    Identifier {
        /// The identifier as given, unvalidated.
        input: String,
    },
}

/// Everything opening needs from the outside world.
pub struct OpenContext<'a> {
    /// The paper database.
    pub db: &'a mut dyn Db,
    /// Where full text is looked up by identifier.
    pub source: &'a dyn FullTextSource,
    /// The PDF text backend.
    pub pdf: &'a dyn PdfTextExtractor,
    /// A writable directory for temporary PDF files.
    pub scratch_dir: &'a Path,
    /// How many tries a network lookup gets in all.
    pub attempts: u32,
    /// The wait before the first retry (doubles each time).
    pub base_delay: Duration,
}

/// How a request ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpenOutcome {
    /// The paper has a canonical text and can be read.
    Ready {
        /// The paper's id.
        paper_id: String,
    },
    /// The paper is stored but has no full text yet; the user can add a file or paste.
    NeedsFullText {
        /// The paper's id.
        paper_id: String,
        /// Why there is no full text, for the user.
        reason: String,
        /// True when the sources could not be reached (try again later),
        /// false when they answered and have nothing.
        unreachable: bool,
    },
}

/// Why a request failed.
#[derive(Debug)]
pub enum OpenError {
    /// The identifier is not valid.
    Identifier(IdentifierError),
    /// The file or text could not be turned into a paper (also used when a
    /// request kind is not allowed, e.g. an identifier in `attach`).
    Intake(IntakeError),
    /// The database failed.
    Store(StoreError),
    /// No paper has this id.
    UnknownPaper(String),
}

impl fmt::Display for OpenError {
    /// A message the user can read.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Identifier(e) => write!(f, "{e}"),
            OpenError::Intake(e) => write!(f, "{e}"),
            OpenError::Store(e) => write!(f, "{e}"),
            OpenError::UnknownPaper(id) => write!(f, "No paper with id {id} exists."),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<IdentifierError> for OpenError {
    /// Wraps an identifier error.
    fn from(e: IdentifierError) -> Self {
        OpenError::Identifier(e)
    }
}
impl From<IntakeError> for OpenError {
    /// Wraps an intake error.
    fn from(e: IntakeError) -> Self {
        OpenError::Intake(e)
    }
}
impl From<StoreError> for OpenError {
    /// Wraps a store error.
    fn from(e: StoreError) -> Self {
        OpenError::Store(e)
    }
}
impl From<DbError> for OpenError {
    /// Wraps a raw database error (from a transaction's begin or commit).
    fn from(e: DbError) -> Self {
        OpenError::Store(StoreError::Db(e))
    }
}

/// True when a stored canonical text should be re-derived from its original:
/// the converter has moved on and nothing (chunks, citations) points into the
/// stored text. In 2a `has_dependents` is always false; 2b passes the truth.
pub fn needs_rederive(stored_version: &str, current_version: &str, has_dependents: bool) -> bool {
    stored_version != current_version && !has_dependents
}

/// Opens a new paper from a file or paste, or an identifier (reopening a
/// stored paper offline when it already has a canonical text).
pub fn open(ctx: &mut OpenContext, request: OpenRequest) -> Result<OpenOutcome, OpenError> {
    let result = match request {
        OpenRequest::Identifier { input } => open_identifier(ctx, &input),
        other => {
            let doc = convert_request(ctx, other)?;
            transaction_with(&mut *ctx.db, |tx| {
                let paper = create_paper(tx, &Identifiers::default(), None)?;
                persist(tx, &paper, doc)
            })
        }
    };
    log_failure("open", result)
}

/// Adds a file or paste to an existing paper (the fallback for `NeedsFullText`).
/// An identifier request is refused with `OpenError::Intake(Unsupported)`.
pub fn attach(
    ctx: &mut OpenContext,
    paper_id: &str,
    request: OpenRequest,
) -> Result<OpenOutcome, OpenError> {
    let result = (|| {
        if matches!(request, OpenRequest::Identifier { .. }) {
            return Err(OpenError::Intake(IntakeError::Unsupported(
                "an identifier cannot be attached to a paper".into(),
            )));
        }
        let paper = existing_paper(&mut *ctx.db, paper_id)?;
        let doc = convert_request(ctx, request)?;
        transaction_with(&mut *ctx.db, |tx| persist(tx, &paper, doc))
    })();
    log_failure("attach", result)
}

/// Reopens a stored paper, re-deriving its canonical text from the stored
/// original when the converter has a newer version. Never touches the network.
pub fn reopen(ctx: &mut OpenContext, paper_id: &str) -> Result<OpenOutcome, OpenError> {
    let result = (|| {
        let paper = existing_paper(&mut *ctx.db, paper_id)?;
        reopen_paper(ctx, &paper)
    })();
    log_failure("reopen", result)
}

/// Logs a failed request (house rule 8) and passes the result through.
fn log_failure(what: &str, r: Result<OpenOutcome, OpenError>) -> Result<OpenOutcome, OpenError> {
    if let Err(e) = &r {
        log::error!("{what} failed: {e}");
    }
    r
}

/// A source converted and assessed, ready to be stored.
struct Converted {
    source: NewSource,
    text: CanonicalText,
    assessment: FullTextAssessment,
    title: String,
}

/// The paper with this id, or `UnknownPaper`.
fn existing_paper(db: &mut dyn Db, id: &str) -> Result<PaperId, OpenError> {
    fetch_one(db, "SELECT id FROM papers WHERE id = ?", &[Value::from(id)])?
        .map(|_| PaperId(id.to_string()))
        .ok_or_else(|| OpenError::UnknownPaper(id.to_string()))
}

/// A file name without its extension, if it has a usable one.
fn file_stem(name: &str) -> Option<String> {
    Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Converts and assesses raw bytes. The title is the canonical title, else the
/// file name without extension, else [`UNTITLED_PAPER`].
fn convert(
    ctx: &OpenContext,
    kind: SourceKind,
    origin: &str,
    file_name: Option<String>,
    bytes: Vec<u8>,
) -> Result<Converted, OpenError> {
    let stem = file_name.as_deref().and_then(file_stem);
    let (text, decoded) = to_canonical(kind, &bytes, stem.as_deref(), ctx.pdf, ctx.scratch_dir)?;
    let assessment = assess(&text)?;
    let title = text
        .title_span
        .and_then(|s| text.span_text(s))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or(stem)
        .unwrap_or_else(|| UNTITLED_PAPER.to_string());
    Ok(Converted {
        source: NewSource {
            kind,
            origin: origin.to_string(),
            file_name,
            bytes,
            decoded,
        },
        text,
        assessment,
        title,
    })
}

/// Converts a file or paste request; refuses an identifier.
fn convert_request(ctx: &OpenContext, request: OpenRequest) -> Result<Converted, OpenError> {
    match request {
        OpenRequest::File { name, bytes } => {
            let kind = detect_kind(name.as_deref(), &bytes)?;
            convert(ctx, kind, ORIGIN_UPLOAD, name, bytes)
        }
        OpenRequest::Paste { text } => {
            let bytes = text.into_bytes();
            let kind = detect_kind(None, &bytes)?;
            convert(ctx, kind, ORIGIN_PASTE, None, bytes)
        }
        OpenRequest::Identifier { .. } => Err(OpenError::Intake(IntakeError::Unsupported(
            "an identifier is not a file".into(),
        ))),
    }
}

/// Writes the source, canonical text and title, and marks the paper opened.
/// Runs inside the caller's transaction.
fn persist(db: &mut dyn Db, paper: &PaperId, doc: Converted) -> Result<OpenOutcome, OpenError> {
    let source_id = save_source(db, paper, &doc.source)?;
    save_canonical(db, paper, source_id, &doc.text, &doc.assessment)?;
    set_title(db, paper, &doc.title)?;
    touch_opened(db, paper)?;
    Ok(OpenOutcome::Ready {
        paper_id: paper.0.clone(),
    })
}

/// Opens by identifier: a stored paper with text reopens offline; otherwise
/// the sources are asked (outside any transaction) and the result stored.
fn open_identifier(ctx: &mut OpenContext, input: &str) -> Result<OpenOutcome, OpenError> {
    let ids = parse_identifier(input)?;
    if let Some(paper) = find_paper(&mut *ctx.db, &ids)? {
        if latest_canonical(&mut *ctx.db, &paper)?.is_some() {
            transaction_with(&mut *ctx.db, |tx| add_identifiers(tx, &paper, &ids))?;
            return reopen_paper(ctx, &paper);
        }
    }
    let retrieved = retrieve(ctx.source, &ids, ctx.attempts, ctx.base_delay);
    let (doc, reason, unreachable) = match retrieved {
        Retrieved::Found {
            kind,
            origin,
            bytes,
        } => (
            Some(convert(ctx, kind, &origin, None, bytes)?),
            String::new(),
            false,
        ),
        Retrieved::NotFound { reason } => (None, reason, false),
        Retrieved::Unreachable { reason } => (None, reason, true),
    };
    transaction_with(&mut *ctx.db, |tx| {
        let paper = match find_paper(tx, &ids)? {
            Some(p) => {
                add_identifiers(tx, &p, &ids)?;
                p
            }
            None => create_paper(tx, &ids, None)?,
        };
        match doc {
            Some(doc) => persist(tx, &paper, doc),
            None => {
                touch_opened(tx, &paper)?;
                Ok(OpenOutcome::NeedsFullText {
                    paper_id: paper.0,
                    reason,
                    unreachable,
                })
            }
        }
    })
}

/// Reopens a known paper: `NeedsFullText` when it has no text, else `Ready`
/// after an optional re-derive.
fn reopen_paper(ctx: &mut OpenContext, paper: &PaperId) -> Result<OpenOutcome, OpenError> {
    let Some(stored) = latest_canonical(&mut *ctx.db, paper)? else {
        touch_opened(&mut *ctx.db, paper)?;
        return Ok(OpenOutcome::NeedsFullText {
            paper_id: paper.0.clone(),
            reason: "No full text has been added to this paper yet.".into(),
            unreachable: false,
        });
    };
    let rederived = match latest_source(&mut *ctx.db, paper)? {
        Some(src)
            if needs_rederive(
                &stored.text.converter_version,
                &current_converter_version(src.kind),
                false, // 2b passes whether chunks or citations point into the text
            ) =>
        {
            match to_canonical(src.kind, &src.bytes, None, ctx.pdf, ctx.scratch_dir)
                .map_err(OpenError::from)
                .and_then(|(t, _)| Ok((assess(&t)?, t)))
            {
                Ok((a, t)) => Some((src.id, t, a)),
                Err(e) => {
                    // Keep the old text rather than lose the paper.
                    log::error!(
                        "re-deriving paper {} failed, keeping old text: {e}",
                        paper.0
                    );
                    None
                }
            }
        }
        _ => None,
    };
    transaction_with(&mut *ctx.db, |tx| {
        if let Some((source_id, text, assessment)) = &rederived {
            save_canonical(tx, paper, *source_id, text, assessment)?;
        }
        touch_opened(tx, paper)?;
        Ok::<_, OpenError>(OpenOutcome::Ready {
            paper_id: paper.0.clone(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::Decoded;
    use crate::store::{open_memory_store, recent_papers};
    use bmlib::db::{execute, fetch_one};
    use bmlib::fulltext::models::{ContentKind, FullTextResult, TextBlock};
    use bmlib::fulltext::service::{FullTextError, FullTextRequest};
    use rusqlite::Connection;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const JATS: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/article.xml"
    ));
    const HTML: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/page.html"
    ));
    const MD: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/paper.md"
    ));
    const TXT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/paper.txt"
    ));

    /// A full-text source that replays a queue and counts calls.
    struct FakeSource {
        queue: RefCell<VecDeque<Result<FullTextResult, FullTextError>>>,
        calls: RefCell<usize>,
    }
    impl FakeSource {
        fn new(items: Vec<Result<FullTextResult, FullTextError>>) -> Self {
            Self {
                queue: RefCell::new(items.into()),
                calls: RefCell::new(0),
            }
        }
        fn calls(&self) -> usize {
            *self.calls.borrow()
        }
    }
    impl FullTextSource for FakeSource {
        fn fetch(&self, _: &FullTextRequest) -> Result<FullTextResult, FullTextError> {
            *self.calls.borrow_mut() += 1;
            self.queue.borrow_mut().pop_front().expect("queue empty")
        }
    }

    /// A PDF backend returning fixed blocks.
    struct FakePdf;
    impl PdfTextExtractor for FakePdf {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn extract_blocks(&self, _: &Path) -> Result<Vec<TextBlock>, String> {
            Ok(vec![])
        }
        fn page_texts(&self, _: &Path) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
    }

    /// A JATS hit as bmlib would return it.
    fn jats_hit() -> Result<FullTextResult, FullTextError> {
        Ok(FullTextResult {
            source: "europepmc".into(),
            html: None,
            pdf_url: None,
            web_url: None,
            file_path: None,
            content_kind: ContentKind::Fulltext,
            jats_xml: Some(JATS.to_vec()),
        })
    }

    /// Runs `f` with a context over `conn` and `source`.
    fn with_ctx<T>(
        conn: &mut Connection,
        source: &FakeSource,
        f: impl FnOnce(&mut OpenContext) -> T,
    ) -> T {
        let scratch = std::env::temp_dir();
        let mut ctx = OpenContext {
            db: conn,
            source,
            pdf: &FakePdf,
            scratch_dir: &scratch,
            attempts: 1,
            base_delay: Duration::ZERO,
        };
        f(&mut ctx)
    }

    fn ident(s: &str) -> OpenRequest {
        OpenRequest::Identifier { input: s.into() }
    }

    fn ready_id(o: OpenOutcome) -> String {
        match o {
            OpenOutcome::Ready { paper_id } => paper_id,
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn paste_opens_ready() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let o = with_ctx(&mut c, &src, |x| {
            open(x, OpenRequest::Paste { text: MD.into() })
        })
        .unwrap();
        let id = PaperId(ready_id(o));
        let canon = latest_canonical(&mut c, &id).unwrap().unwrap();
        assert_eq!(canon.text.converter_version, "markdown-1");
        assert_eq!(src.calls(), 0);
    }

    #[test]
    fn identifier_found_then_reopened_offline() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![jats_hit()]);
        let a = ready_id(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        let b = ready_id(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!(a, b);
        assert_eq!(src.calls(), 1);
        let s = latest_source(&mut c, &PaperId(a)).unwrap().unwrap();
        assert_eq!(s.kind, SourceKind::Jats);
    }

    #[test]
    fn identifier_not_found_then_attach() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Unavailable("none".into()))]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap();
        let OpenOutcome::NeedsFullText {
            paper_id,
            unreachable,
            ..
        } = o
        else {
            panic!("expected NeedsFullText")
        };
        assert!(!unreachable);
        let papers = recent_papers(&mut c).unwrap();
        assert_eq!(papers[0].identifiers.pmid.as_deref(), Some("12345678"));
        // Reopening with no text does not hit the network and still needs text.
        let r = with_ctx(&mut c, &src, |x| reopen(x, &paper_id)).unwrap();
        assert!(matches!(
            r,
            OpenOutcome::NeedsFullText {
                unreachable: false,
                ..
            }
        ));
        assert_eq!(src.calls(), 1);
        let o = with_ctx(&mut c, &src, |x| {
            attach(
                x,
                &paper_id,
                OpenRequest::File {
                    name: Some("page.html".into()),
                    bytes: HTML.to_vec(),
                },
            )
        })
        .unwrap();
        assert_eq!(ready_id(o), paper_id);
        assert!(latest_canonical(&mut c, &PaperId(paper_id))
            .unwrap()
            .is_some());
    }

    #[test]
    fn identifier_unreachable_flagged() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Other("connection reset".into()))]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap();
        assert!(matches!(
            o,
            OpenOutcome::NeedsFullText {
                unreachable: true,
                ..
            }
        ));
    }

    #[test]
    fn bad_identifier_creates_nothing() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let r = with_ctx(&mut c, &src, |x| open(x, ident("abc")));
        assert!(matches!(r, Err(OpenError::Identifier(_))));
        assert!(recent_papers(&mut c).unwrap().is_empty());
    }

    #[test]
    fn intake_error_rolls_back() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let r = with_ctx(&mut c, &src, |x| {
            open(
                x,
                OpenRequest::File {
                    name: Some("a.txt".into()),
                    bytes: vec![0, 1, 2, 3, 0, 0, 255, 254, 0, 0],
                },
            )
        });
        assert!(matches!(r, Err(OpenError::Intake(IntakeError::Binary))));
        assert!(recent_papers(&mut c).unwrap().is_empty());
        let n = fetch_one(&mut c, "SELECT count(*) AS n FROM sources", &[])
            .unwrap()
            .unwrap()
            .get_i64("n")
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn guessed_encoding_stored() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let text = format!("{TXT}\nThe caf\u{e9} cohort, r\u{e9}sum\u{e9} and donn\u{e9}es.\n");
        let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode(&text);
        let o = with_ctx(&mut c, &src, |x| {
            open(
                x,
                OpenRequest::File {
                    name: Some("old.txt".into()),
                    bytes: bytes.into_owned(),
                },
            )
        })
        .unwrap();
        let s = latest_source(&mut c, &PaperId(ready_id(o)))
            .unwrap()
            .unwrap();
        assert_eq!(
            s.decoded,
            Some(Decoded {
                encoding: "windows-1252".into(),
                guessed: true
            })
        );
    }

    #[test]
    fn rederive_rule() {
        assert!(needs_rederive("jats-1", "jats-2", false));
        assert!(!needs_rederive("jats-1", "jats-1", false));
        assert!(!needs_rederive("jats-1", "jats-2", true));
    }

    #[test]
    fn reopen_rederives_old_version() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = ready_id(
            with_ctx(&mut c, &src, |x| {
                open(
                    x,
                    OpenRequest::File {
                        name: Some("article.xml".into()),
                        bytes: JATS.to_vec(),
                    },
                )
            })
            .unwrap(),
        );
        let current = current_converter_version(SourceKind::Jats);
        execute(
            &mut c,
            r#"UPDATE canonical_texts SET converter_version = 'jats-0',
             canonical_json = replace(canonical_json, ?, '"jats-0"')"#,
            &[Value::from(format!("\"{current}\""))],
        )
        .unwrap();
        let before = latest_canonical(&mut c, &PaperId(id.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(before.text.converter_version, "jats-0");
        with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap();
        let after = latest_canonical(&mut c, &PaperId(id)).unwrap().unwrap();
        assert_ne!(after.id, before.id);
        assert_eq!(after.text.converter_version, current);
    }

    #[test]
    fn unknown_paper_and_identifier_attach() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let r = with_ctx(&mut c, &src, |x| reopen(x, "nope"));
        assert!(matches!(r, Err(OpenError::UnknownPaper(_))));
        let r = with_ctx(&mut c, &src, |x| {
            attach(x, "nope", OpenRequest::Paste { text: "x".into() })
        });
        assert!(matches!(r, Err(OpenError::UnknownPaper(_))));
        let id = ready_id(
            with_ctx(&mut c, &src, |x| {
                open(x, OpenRequest::Paste { text: MD.into() })
            })
            .unwrap(),
        );
        let r = with_ctx(&mut c, &src, |x| attach(x, &id, ident("12345678")));
        assert!(matches!(
            r,
            Err(OpenError::Intake(IntakeError::Unsupported(_)))
        ));
    }
}
