//! Opening a paper: from a file, a paste or an identifier, and reopening or
//! completing one from the recent list. Orchestrates intake, retrieval and
//! storage. A file or paste commits in one transaction; an identifier that
//! reopens a stored paper commits its identifiers, then the reopen, separately.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use bmlib::db::{transaction_with, Db, DbError};
use bmlib::fulltext::canonical::{CanonicalText, FullTextAssessment};
use bmlib::fulltext::pdf_text::PdfTextExtractor;
use serde::Serialize;

use crate::config::{ORIGIN_PASTE, ORIGIN_UPLOAD, UNTITLED_PAPER};
use crate::identifiers::{parse_identifier, IdentifierError, Identifiers};
use crate::intake::{
    assess, current_converter_version, detect_kind, jats_identifiers, to_canonical, IntakeError,
    SourceKind,
};
use crate::limit::{Limit, Stop};
use crate::retrieval::{retrieve, FullTextSource, Retrieved};
use crate::store::{
    add_identifiers, confirm_full_text, create_paper, find_paper, identifier_conflict,
    latest_canonical, paper_by_id, record_rederive_failure, save_canonical, save_source, set_title,
    source_by_id, touch_opened, NewSource, PaperId, RederiveFailure, StoreError,
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
    /// The lookup's total time limit and cancel switch; read only by a lookup
    /// by identifier. It must be the `Limit` that `source`'s HTTP client was
    /// built with, or a cancel is only seen between attempts.
    pub limit: &'a Limit,
}

/// How a request ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpenOutcome {
    /// The paper has a canonical text and can be read.
    Ready {
        /// The paper's id.
        paper_id: String,
        /// What the user should know about this open, one sentence each (an
        /// identifier not recorded, a failed re-derive); usually empty.
        notices: Vec<String>,
    },
    /// The paper is stored but has no full text yet; the user can add a file or paste.
    NeedsFullText {
        /// The paper's id.
        paper_id: String,
        /// Why there is no full text, for the user.
        reason: String,
        /// True only when the sources could not be reached (try again later);
        /// false for every other cause, which `reason` explains.
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
    /// The attached document's own identifiers say it is a different paper.
    WrongPaper(String),
    /// The user cancelled a lookup by identifier; nothing was stored.
    Cancelled,
}

impl fmt::Display for OpenError {
    /// A message the user can read.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Identifier(e) => write!(f, "{e}"),
            OpenError::Intake(e) => write!(f, "{e}"),
            OpenError::Store(e) => write!(f, "{e}"),
            OpenError::UnknownPaper(id) => write!(f, "No paper with id {id} exists."),
            OpenError::WrongPaper(why) => write!(
                f,
                "This document is a different paper: {why}. Choose the file for this paper."
            ),
            OpenError::Cancelled => write!(f, "The lookup was cancelled."),
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
/// the converter has moved on, this version has not already failed on it
/// (`failed_version`, the version of the last failed attempt), and nothing
/// (chunks, citations) points into the stored text. In 2a `has_dependents`
/// is always false; 2b passes the truth.
pub fn needs_rederive(
    stored_version: &str,
    current_version: &str,
    failed_version: Option<&str>,
    has_dependents: bool,
) -> bool {
    stored_version != current_version && failed_version != Some(current_version) && !has_dependents
}

/// Opens a file or paste (on the stored paper its JATS identifiers name, else
/// a new paper), or an identifier (reopening a stored paper offline when it
/// already has a canonical text).
pub fn open(ctx: &mut OpenContext, request: OpenRequest) -> Result<OpenOutcome, OpenError> {
    let result = match request {
        OpenRequest::Identifier { input } => open_identifier(ctx, &input),
        other => {
            let doc = convert_request(ctx, other)?;
            transaction_with(&mut *ctx.db, |tx| {
                let (paper, notices) = find_or_create(tx, &doc.ids)?;
                persist(tx, &paper, doc, notices)
            })
        }
    };
    log_failure("open", result)
}

/// Adds a file or paste to an existing paper (the fallback for `NeedsFullText`).
/// An identifier request is refused with `OpenError::Intake(Unsupported)`, and
/// a JATS file whose own identifiers name another paper with `WrongPaper`.
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
        transaction_with(&mut *ctx.db, |tx| {
            if let Some(why) = identifier_conflict(tx, &paper, &doc.ids)? {
                return Err(OpenError::WrongPaper(why));
            }
            let notices = add_identifiers(tx, &paper, &doc.ids)?;
            persist(tx, &paper, doc, notices)
        })
    })();
    log_failure("attach", result)
}

/// Reopens a stored paper, re-deriving its canonical text from the stored
/// original when the converter has a newer version. Never touches the network.
pub fn reopen(ctx: &mut OpenContext, paper_id: &str) -> Result<OpenOutcome, OpenError> {
    let result = (|| {
        let paper = existing_paper(&mut *ctx.db, paper_id)?;
        reopen_paper(ctx, &paper, Vec::new())
    })();
    log_failure("reopen", result)
}

/// Logs a failed request (house rule 8) and passes the result through; a
/// cancel is the user's choice, logged as information.
fn log_failure(what: &str, r: Result<OpenOutcome, OpenError>) -> Result<OpenOutcome, OpenError> {
    match &r {
        Err(OpenError::Cancelled) => log::info!("{what} cancelled by the user"),
        Err(e) => log::error!("{what} failed: {e}"),
        Ok(_) => {}
    }
    r
}

/// A source converted and assessed, ready to be stored.
struct Converted {
    /// The original, to store as the paper's source.
    source: NewSource,
    /// The canonical text derived from it.
    text: CanonicalText,
    /// The full-text assessment of `text`.
    assessment: FullTextAssessment,
    /// The paper's title (see [`convert`]).
    title: String,
    /// The identifiers the document carries about itself (JATS only).
    ids: Identifiers,
    /// Notices for the user about the conversion (an invalid identifier dropped).
    notices: Vec<String>,
}

/// The paper with this id, or `UnknownPaper`.
fn existing_paper(db: &mut dyn Db, id: &str) -> Result<PaperId, OpenError> {
    paper_by_id(db, id)?.ok_or_else(|| OpenError::UnknownPaper(id.to_string()))
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
    let (ids, notices) = if kind == SourceKind::Jats {
        jats_identifiers(&bytes)
    } else {
        Default::default()
    };
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
        ids,
        notices,
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
/// The outcome carries the document's notices, then `notices`. Runs inside
/// the caller's transaction.
fn persist(
    db: &mut dyn Db,
    paper: &PaperId,
    doc: Converted,
    notices: Vec<String>,
) -> Result<OpenOutcome, OpenError> {
    let source_id = save_source(db, paper, doc.source)?;
    save_canonical(db, paper, source_id, &doc.text, &doc.assessment)?;
    set_title(db, paper, &doc.title)?;
    touch_opened(db, paper)?;
    Ok(OpenOutcome::Ready {
        paper_id: paper.0.clone(),
        notices: doc.notices.into_iter().chain(notices).collect(),
    })
}

/// The paper holding any of `ids` (absent ones are filled in, with a notice
/// for each that could not be), else a new paper.
fn find_or_create(
    db: &mut dyn Db,
    ids: &Identifiers,
) -> Result<(PaperId, Vec<String>), StoreError> {
    match find_paper(db, ids)? {
        Some(p) => {
            let notices = add_identifiers(db, &p, ids)?;
            Ok((p, notices))
        }
        None => Ok((create_paper(db, ids, None)?, Vec::new())),
    }
}

/// `primary` with its absent fields filled from `extra`.
fn merge_ids(primary: &Identifiers, extra: &Identifiers) -> Identifiers {
    Identifiers {
        pmid: primary.pmid.clone().or_else(|| extra.pmid.clone()),
        doi: primary.doi.clone().or_else(|| extra.doi.clone()),
        pmc_id: primary.pmc_id.clone().or_else(|| extra.pmc_id.clone()),
    }
}

/// Opens by identifier: a stored paper with text reopens offline (a cancel
/// does not stop that); otherwise the sources are asked (outside any
/// transaction) and the result stored unless the user cancelled first.
fn open_identifier(ctx: &mut OpenContext, input: &str) -> Result<OpenOutcome, OpenError> {
    let ids = parse_identifier(input)?;
    if let Some(paper) = find_paper(&mut *ctx.db, &ids)? {
        if latest_canonical(&mut *ctx.db, &paper)?.is_some() {
            let notices = transaction_with(&mut *ctx.db, |tx| add_identifiers(tx, &paper, &ids))?;
            return reopen_paper(ctx, &paper, notices);
        }
    }
    let retrieved = retrieve(ctx.source, &ids, ctx.attempts, ctx.base_delay, ctx.limit);
    let (doc, reason, unreachable) = match retrieved {
        Retrieved::Found {
            kind,
            origin,
            bytes,
        } => match convert(ctx, kind, &origin, None, bytes) {
            Ok(doc) => (Some(doc), String::new(), false),
            Err(e) => {
                // Keep the paper (and its identifiers) so the user can add the text by hand.
                log::warn!("the document from {origin} could not be read: {e}");
                (
                    None,
                    format!("The document from {origin} could not be read: {e}"),
                    false,
                )
            }
        },
        Retrieved::NotFound { reason } => (None, reason, false),
        Retrieved::Unreachable { reason } => (None, reason, true),
        Retrieved::Cancelled => return Err(OpenError::Cancelled),
    };
    // A cancel pressed while the found document was converted still stores nothing.
    if ctx.limit.remaining() == Err(Stop::Cancelled) {
        log::info!("lookup cancelled by the user after the sources answered");
        return Err(OpenError::Cancelled);
    }
    let all_ids = doc
        .as_ref()
        .map_or_else(|| ids.clone(), |d| merge_ids(&ids, &d.ids));
    transaction_with(&mut *ctx.db, |tx| {
        let (paper, notices) = find_or_create(tx, &all_ids)?;
        match doc {
            Some(doc) => persist(tx, &paper, doc, notices),
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
/// after an optional re-derive, carrying `notices` and, while a re-derive with
/// the current converter has failed, a notice saying so. A failed attempt is
/// recorded so it is not repeated until the converter changes again (#22).
fn reopen_paper(
    ctx: &mut OpenContext,
    paper: &PaperId,
    mut notices: Vec<String>,
) -> Result<OpenOutcome, OpenError> {
    let Some(stored) = latest_canonical(&mut *ctx.db, paper)? else {
        touch_opened(&mut *ctx.db, paper)?;
        return Ok(OpenOutcome::NeedsFullText {
            paper_id: paper.0.clone(),
            reason: "No full text has been added to this paper yet.".into(),
            unreachable: false,
        });
    };
    // The source the stored text was derived from, not merely the newest one.
    let src = source_by_id(&mut *ctx.db, stored.source_id)?;
    let current = current_converter_version(src.kind);
    // A failure under an older converter is stale: the current one tries afresh.
    let mut failure = stored
        .rederive_failure
        .clone()
        .filter(|f| f.converter_version == current);
    let mut rederived = None;
    let mut newly_failed = false;
    if needs_rederive(
        &stored.text.converter_version,
        &current,
        failure.as_ref().map(|f| f.converter_version.as_str()),
        false, // 2b passes whether chunks or citations point into the text
    ) {
        // The same title hint as the first conversion, or the text (and every offset) changes.
        let stem = src.file_name.as_deref().and_then(file_stem);
        match to_canonical(
            src.kind,
            &src.bytes,
            stem.as_deref(),
            ctx.pdf,
            ctx.scratch_dir,
        )
        .map_err(OpenError::from)
        .and_then(|(t, _)| Ok((assess(&t)?, t)))
        {
            Ok((a, t)) => rederived = Some((src.id, t, a)),
            Err(e) => {
                // The old text is unchanged and correct for its converter, so the
                // paper stays Ready, with a notice.
                log::error!(
                    "re-deriving paper {} with {current} failed, keeping old text: {e}",
                    paper.0
                );
                failure = Some(RederiveFailure {
                    converter_version: current.clone(),
                    error: e.to_string(),
                });
                newly_failed = true;
            }
        }
    }
    // `failure` and `rederived` are never both set: a recorded failure blocks the attempt.
    if let Some(f) = &failure {
        notices.push(format!(
            "This text was made by an older converter ({}). Reading the original again with {current} failed, so the older text is shown: {}",
            stored.text.converter_version, f.error
        ));
    }
    transaction_with(&mut *ctx.db, |tx| {
        if let Some((source_id, text, assessment)) = &rederived {
            let id = save_canonical(tx, paper, *source_id, text, assessment)?;
            // Same source, same paper: the user's "this is the whole paper" still holds.
            if stored.confirmed_full_text {
                confirm_full_text(tx, id)?;
            }
        }
        if let Some(f) = failure.as_ref().filter(|_| newly_failed) {
            record_rederive_failure(tx, stored.id, f)?;
        }
        touch_opened(tx, paper)?;
        Ok::<_, OpenError>(OpenOutcome::Ready {
            paper_id: paper.0.clone(),
            notices,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::Decoded;
    use crate::store::{
        latest_canonical, latest_source, open_memory_store, recent_papers, save_source,
    };
    use bmlib::db::{execute, fetch_one, Value};
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
            pdf_download_failure: None,
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
        let limit = Limit::new(Duration::from_secs(3600));
        let mut ctx = OpenContext {
            db: conn,
            source,
            pdf: &FakePdf,
            scratch_dir: &scratch,
            attempts: 1,
            base_delay: Duration::ZERO,
            limit: &limit,
        };
        f(&mut ctx)
    }

    fn ident(s: &str) -> OpenRequest {
        OpenRequest::Identifier { input: s.into() }
    }

    fn ready_id(o: OpenOutcome) -> String {
        ready(o).0
    }

    /// The id and notices of a `Ready` outcome; panics on any other.
    fn ready(o: OpenOutcome) -> (String, Vec<String>) {
        match o {
            OpenOutcome::Ready { paper_id, notices } => (paper_id, notices),
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

    /// A lookup the user cancelled is `Cancelled` and stores no paper.
    #[test]
    fn cancelled_lookup_stores_nothing() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Other("reset".into()))]);
        let cancelled = Limit::new(Duration::from_secs(3600));
        cancelled.cancel();
        let scratch = std::env::temp_dir();
        let mut ctx = OpenContext {
            db: &mut c,
            source: &src,
            pdf: &FakePdf,
            scratch_dir: &scratch,
            attempts: 1,
            base_delay: Duration::ZERO,
            limit: &cancelled,
        };
        assert!(matches!(
            open(&mut ctx, ident("12345678")),
            Err(OpenError::Cancelled)
        ));
        assert!(recent_papers(&mut c).unwrap().is_empty());
    }

    /// "Try again" on a stored paper with no text, cancelled, leaves it as it
    /// was: still one paper, same identifiers.
    #[test]
    fn cancelled_retry_leaves_stored_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Unavailable("none".into()))]);
        with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap();
        let before = recent_papers(&mut c).unwrap();
        let src = FakeSource::new(vec![jats_hit()]);
        let cancelled = Limit::new(Duration::from_secs(3600));
        cancelled.cancel();
        let scratch = std::env::temp_dir();
        let mut ctx = OpenContext {
            db: &mut c,
            source: &src,
            pdf: &FakePdf,
            scratch_dir: &scratch,
            attempts: 1,
            base_delay: Duration::ZERO,
            limit: &cancelled,
        };
        assert!(matches!(
            open(&mut ctx, ident("12345678")),
            Err(OpenError::Cancelled)
        ));
        let after = recent_papers(&mut c).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].identifiers, before[0].identifiers);
        assert!(latest_canonical(&mut c, &PaperId(after[0].id.clone()))
            .unwrap()
            .is_none());
    }

    /// A cancel pressed while a found PDF is converted stores nothing.
    #[test]
    fn cancel_during_conversion_stores_nothing() {
        /// A PDF backend that cancels its limit when asked for text.
        struct CancellingPdf(Limit);
        impl PdfTextExtractor for CancellingPdf {
            fn name(&self) -> &'static str {
                "cancelling"
            }
            fn extract_blocks(&self, _: &Path) -> Result<Vec<TextBlock>, String> {
                self.0.cancel();
                Ok(vec![])
            }
            fn page_texts(&self, _: &Path) -> Result<Vec<String>, String> {
                self.0.cancel();
                Ok(vec![])
            }
        }
        let dir = std::env::temp_dir().join(format!("sa-open-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf_path = dir.join("a.pdf");
        std::fs::write(&pdf_path, b"%PDF-1.4 fake").unwrap();
        let src = FakeSource::new(vec![Ok(FullTextResult {
            file_path: Some(pdf_path.to_string_lossy().into_owned()),
            content_kind: ContentKind::Extracted,
            jats_xml: None,
            ..jats_hit().unwrap()
        })]);
        let limit = Limit::new(Duration::from_secs(3600));
        let pdf = CancellingPdf(limit.clone());
        let mut c = open_memory_store().unwrap();
        let mut ctx = OpenContext {
            db: &mut c,
            source: &src,
            pdf: &pdf,
            scratch_dir: &dir,
            attempts: 1,
            base_delay: Duration::ZERO,
            limit: &limit,
        };
        let r = open(&mut ctx, ident("12345678"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(r, Err(OpenError::Cancelled)), "{r:?}");
        assert_eq!(limit.remaining(), Err(Stop::Cancelled));
        assert!(recent_papers(&mut c).unwrap().is_empty());
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
    fn intake_error_leaves_nothing() {
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

    /// Counts the rows of a table.
    fn count(c: &mut Connection, table: &str) -> i64 {
        fetch_one(c, &format!("SELECT count(*) AS n FROM {table}"), &[])
            .unwrap()
            .unwrap()
            .get_i64("n")
            .unwrap()
    }

    #[test]
    fn failed_write_rolls_back_the_whole_request() {
        let mut c = open_memory_store().unwrap();
        execute(
            &mut c,
            "CREATE TRIGGER no_canonical BEFORE INSERT ON canonical_texts
             BEGIN SELECT RAISE(ABORT, 'refused'); END",
            &[],
        )
        .unwrap();
        let src = FakeSource::new(vec![]);
        let r = with_ctx(&mut c, &src, |x| {
            open(x, OpenRequest::Paste { text: MD.into() })
        });
        assert!(matches!(r, Err(OpenError::Store(_))));
        for table in ["papers", "sources", "canonical_texts"] {
            assert_eq!(count(&mut c, table), 0, "{table} must be empty");
        }
    }

    #[test]
    fn unreadable_download_keeps_the_paper() {
        let mut c = open_memory_store().unwrap();
        let bad = Ok(FullTextResult {
            jats_xml: Some(b"this is not xml".to_vec()),
            ..jats_hit().unwrap()
        });
        let src = FakeSource::new(vec![bad]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap();
        let OpenOutcome::NeedsFullText {
            reason,
            unreachable,
            ..
        } = o
        else {
            panic!("expected NeedsFullText")
        };
        assert!(!unreachable);
        assert!(reason.contains("europepmc"), "{reason}");
        let papers = recent_papers(&mut c).unwrap();
        assert_eq!(papers[0].identifiers.pmid.as_deref(), Some("12345678"));
        assert_eq!(count(&mut c, "canonical_texts"), 0);
    }

    #[test]
    fn jats_identifiers_find_the_same_paper_later() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![jats_hit()]);
        let a = ready_id(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        let b = ready_id(with_ctx(&mut c, &src, |x| open(x, ident("10.1234/fixture.1"))).unwrap());
        assert_eq!(a, b);
        assert_eq!(src.calls(), 1);
        assert_eq!(count(&mut c, "papers"), 1);
    }

    #[test]
    fn uploaded_jats_records_its_identifiers() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let o = with_ctx(&mut c, &src, |x| {
            open(
                x,
                OpenRequest::File {
                    name: Some("article.xml".into()),
                    bytes: JATS.to_vec(),
                },
            )
        })
        .unwrap();
        let ids = recent_papers(&mut c).unwrap()[0].identifiers.clone();
        assert_eq!(ids.pmid.as_deref(), Some("12345678"));
        assert_eq!(ids.doi.as_deref(), Some("10.1234/fixture.1"));
        // Uploading it again lands on the same paper, not a conflict.
        let again = with_ctx(&mut c, &src, |x| {
            open(
                x,
                OpenRequest::File {
                    name: Some("article.xml".into()),
                    bytes: JATS.to_vec(),
                },
            )
        })
        .unwrap();
        assert_eq!(ready_id(o), ready_id(again));
    }

    #[test]
    fn reopen_rederives_from_the_source_the_text_names() {
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
        let paper = PaperId(id.clone());
        let a = latest_source(&mut c, &paper).unwrap().unwrap();
        // A newer source B that the stored text does not come from.
        save_source(
            &mut c,
            &paper,
            NewSource {
                kind: SourceKind::Markdown,
                origin: ORIGIN_PASTE.into(),
                file_name: None,
                bytes: MD.as_bytes().to_vec(),
                decoded: None,
            },
        )
        .unwrap();
        let current = current_converter_version(SourceKind::Jats);
        execute(
            &mut c,
            r#"UPDATE canonical_texts SET converter_version = 'jats-0',
               canonical_json = replace(canonical_json, ?, '"jats-0"')"#,
            &[Value::from(format!("\"{current}\""))],
        )
        .unwrap();
        with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap();
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(after.source_id, a.id);
        assert_eq!(after.text.converter_version, current);
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
        assert!(needs_rederive("jats-1", "jats-2", None, false));
        assert!(!needs_rederive("jats-1", "jats-1", None, false));
        assert!(!needs_rederive("jats-1", "jats-2", None, true));
        // A failure under the current version blocks; one under an older version does not.
        assert!(!needs_rederive("jats-1", "jats-2", Some("jats-2"), false));
        assert!(needs_rederive("jats-1", "jats-3", Some("jats-2"), false));
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

    /// Opens `bytes` as a file named `name` and returns the paper id.
    fn open_file(c: &mut Connection, src: &FakeSource, name: &str, bytes: &[u8]) -> String {
        ready_id(
            with_ctx(c, src, |x| {
                open(
                    x,
                    OpenRequest::File {
                        name: Some(name.into()),
                        bytes: bytes.to_vec(),
                    },
                )
            })
            .unwrap(),
        )
    }

    /// Marks every stored canonical text of `kind` as made by an older converter.
    fn age_canonical(c: &mut Connection, kind: SourceKind) {
        let current = current_converter_version(kind);
        execute(
            c,
            "UPDATE canonical_texts SET converter_version = 'old-0',
             canonical_json = replace(canonical_json, ?, '\"old-0\"')",
            &[Value::from(format!("\"{current}\""))],
        )
        .unwrap();
    }

    /// A plain-text upload re-derives to the very same text: the file name is
    /// passed as the title hint again, so the title and every offset survive.
    #[test]
    fn rederived_plain_text_keeps_its_title() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "trial notes.txt", TXT.as_bytes());
        let paper = PaperId(id.clone());
        let before = latest_canonical(&mut c, &paper).unwrap().unwrap();
        age_canonical(&mut c, SourceKind::PlainText);
        with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap();
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_ne!(after.id, before.id);
        assert_eq!(after.text.text, before.text.text);
        assert_eq!(after.text.title_span, before.text.title_span);
    }

    /// A confirmed text stays confirmed when it is re-derived from the same source.
    #[test]
    fn rederive_keeps_the_confirmation() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        let before = latest_canonical(&mut c, &paper).unwrap().unwrap();
        confirm_full_text(&mut c, before.id).unwrap();
        age_canonical(&mut c, SourceKind::Jats);
        with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap();
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_ne!(after.id, before.id);
        assert!(after.confirmed_full_text);
    }

    /// When re-deriving fails the old text is kept, the paper opens with a
    /// notice, and the failure is recorded so later opens do not try again
    /// (#22) but still show the notice.
    #[test]
    fn failed_rederive_keeps_the_old_text() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        age_canonical(&mut c, SourceKind::Jats);
        execute(&mut c, "UPDATE sources SET bytes = x'00'", &[]).unwrap();
        let before = latest_canonical(&mut c, &paper).unwrap().unwrap();
        let (got, notices) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert_eq!(got, id);
        assert_eq!(notices.len(), 1);
        assert!(
            notices[0].contains("older converter (old-0)"),
            "{notices:?}"
        );
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.text.converter_version, "old-0");
        let current = current_converter_version(SourceKind::Jats);
        assert_eq!(
            after.rederive_failure.map(|f| f.converter_version),
            Some(current)
        );
        // Even with a readable source now, the same converter is not tried again.
        execute(
            &mut c,
            "UPDATE sources SET bytes = ?",
            &[Value::from(JATS.to_vec())],
        )
        .unwrap();
        let (_, again) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert_eq!(again, notices);
        assert_eq!(
            latest_canonical(&mut c, &paper).unwrap().unwrap().id,
            before.id
        );
    }

    /// A failure recorded under an older converter does not block the current one.
    #[test]
    fn stale_rederive_failure_is_retried() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        age_canonical(&mut c, SourceKind::Jats);
        execute(
            &mut c,
            "UPDATE canonical_texts SET rederive_failed_version = 'old-1', rederive_error = 'e'",
            &[],
        )
        .unwrap();
        let (_, notices) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(
            after.text.converter_version,
            current_converter_version(SourceKind::Jats)
        );
        assert_eq!(after.rederive_failure, None);
    }

    /// An invalid identifier inside a JATS file is dropped with a notice.
    #[test]
    fn invalid_jats_identifier_is_a_notice() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let bad = String::from_utf8_lossy(JATS).replace("12345678", "not-a-pmid");
        let (_, notices) = ready(
            with_ctx(&mut c, &src, |x| {
                open(
                    x,
                    OpenRequest::File {
                        name: Some("article.xml".into()),
                        bytes: bad.into_bytes(),
                    },
                )
            })
            .unwrap(),
        );
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("PMID"), "{notices:?}");
    }

    /// A file whose PMID names paper A and whose DOI another paper holds lands
    /// on A and says the DOI was not recorded.
    #[test]
    fn partly_clashing_file_lands_with_a_notice() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let a = create_paper(&mut c, &parse_identifier("12345678").unwrap(), None).unwrap();
        create_paper(
            &mut c,
            &parse_identifier("10.1234/fixture.1").unwrap(),
            None,
        )
        .unwrap();
        let (got, notices) = ready(
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
        assert_eq!(got, a.0);
        assert_eq!(
            notices,
            ["DOI 10.1234/fixture.1 was not recorded: it belongs to another stored paper."]
        );
    }

    /// "Try again" re-sends the paper's own identifier: the stored paper without
    /// text is looked up again and completed, not duplicated.
    #[test]
    fn retrying_an_identifier_completes_the_same_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Other("reset".into())), jats_hit()]);
        let first = with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap();
        let OpenOutcome::NeedsFullText { paper_id, .. } = first else {
            panic!("expected NeedsFullText, got {first:?}")
        };
        let again = with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap();
        assert_eq!(ready_id(again), paper_id);
        assert_eq!(count(&mut c, "papers"), 1);
        assert_eq!(src.calls(), 2);
    }

    /// A JATS file whose own PMID differs from the paper's is refused, and nothing is stored.
    #[test]
    fn attaching_another_papers_file_is_refused() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Unavailable("none".into()))]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("999"))).unwrap();
        let OpenOutcome::NeedsFullText { paper_id, .. } = o else {
            panic!("expected NeedsFullText")
        };
        let r = with_ctx(&mut c, &src, |x| {
            attach(
                x,
                &paper_id,
                OpenRequest::File {
                    name: Some("article.xml".into()),
                    bytes: JATS.to_vec(),
                },
            )
        });
        match r {
            Err(OpenError::WrongPaper(why)) => assert!(why.contains("12345678"), "{why}"),
            other => panic!("expected WrongPaper, got {other:?}"),
        }
        assert_eq!(count(&mut c, "sources"), 0);
        assert_eq!(count(&mut c, "canonical_texts"), 0);
    }

    /// A PDF with no text layer (a scan) is refused as empty and stores nothing.
    #[test]
    fn pdf_without_text_is_empty() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let r = with_ctx(&mut c, &src, |x| {
            open(
                x,
                OpenRequest::File {
                    name: Some("scan.pdf".into()),
                    bytes: b"%PDF-1.4 scanned".to_vec(),
                },
            )
        });
        assert!(
            matches!(r, Err(OpenError::Intake(IntakeError::Empty))),
            "{r:?}"
        );
        assert_eq!(count(&mut c, "papers"), 0);
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
