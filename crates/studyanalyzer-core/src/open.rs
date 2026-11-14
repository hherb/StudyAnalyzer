//! Opening a paper: from a file, a paste or an identifier, and reopening or
//! completing one from the recent list. Orchestrates intake, retrieval and
//! storage. A file or paste commits in one transaction; an identifier that
//! reopens a stored paper commits its identifiers, then the reopen, separately.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use bmlib::db::{transaction_with, Db, DbError};
use bmlib::fulltext::canonical::{CanonicalText, FullTextAssessment, HtmlError};
use bmlib::fulltext::pdf_text::{PdfErrorKind, PdfLimit, PdfTextExtractor};
use serde::Serialize;

use crate::config::{
    KEEP_MISMATCHED_LABEL, LINK_ORIGIN, NO_FULL_TEXT_YET, ORIGIN_PASTE, ORIGIN_UPLOAD, PDF_MAGIC,
    REPLACED_CONFIRMED_NOTICE, UNTITLED_PAPER,
};
use crate::conversations::has_turns;
use crate::handoff::Handoff;
use crate::identifiers::{parse_identifier, IdentifierError, Identifiers};
use crate::intake::{
    assess, current_converter_version, detect_kind, is_engine_notice, jats_identifiers,
    to_canonical, IntakeError, SourceKind,
};
use crate::library::{read_source, settle_paper, waiting_notice, Configured, LibraryFault};
use crate::limit::{Limit, Stop};
use crate::pico_store::has_citations;
use crate::retrieval::{resolve, retrieve, FullTextSource, PdfBeside, Retrieved};
use crate::store::{
    add_identifiers, bring_back, confirm_full_text, contradiction, create_paper, find_paper,
    holder_of_source, holders, identifier_conflict, latest_canonical, paper_by_id, paper_title,
    quoted_title, record_rederive_failure, record_source_notices, refusal, save_canonical,
    save_pdf_beside, save_source, set_refusal, set_title, source_by_id, source_info,
    source_notices, touch_opened, NewSource, PaperId, RederiveFailure, SourcePlace, StoreError,
};
use bmlib::library::sha256_hex;

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
    /// A PMID, DOI or PMC ID typed in.
    Identifier {
        /// The identifier as given, unvalidated.
        input: String,
    },
    /// A paper another app handed over, already validated (see
    /// [`crate::handoff`]): its full text opens as a file does, else it is a
    /// lookup by its identifiers.
    Handoff(Handoff),
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
    /// by identifier, whose PDF extraction its cancel switch also stops. It
    /// must be the `Limit` that `source`'s HTTP client was built with, or a
    /// cancel is only seen between attempts.
    pub limit: &'a Limit,
    /// The debugging setting [`crate::config::KEEP_MISMATCHED_KEY`]: a document that
    /// contradicts the paper it matches opens as a paper of its own instead of
    /// being refused (see `place`).
    pub keep_mismatched: bool,
    /// The shared paper library, if one is chosen: a new source waits for it
    /// and is moved in after its open commits (see `finish`), and a source
    /// kept there is read from it.
    pub library: &'a Configured,
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
    /// An opened file or paste contradicts the stored paper it matches: nothing
    /// was stored. A lookup reports this as `NeedsFullText.reason` instead,
    /// keeping the paper of the identifier typed. Unlike `WrongPaper` (attach),
    /// an identifier another paper holds is no mismatch, only a notice.
    Mismatch {
        /// The stored paper, as [`quoted_title`] names it.
        paper: String,
        /// Which identifier differs, and how.
        why: String,
        /// The document's own notices (an invalid identifier dropped), which
        /// would otherwise be lost with it.
        notices: Vec<String>,
    },
    /// A handed-over document's own identifiers differ from those sent with
    /// it: nothing was stored.
    HandoffMismatch {
        /// The sending app, or [`LINK_ORIGIN`] when there is none.
        sender: String,
        /// Which identifier differs, and how.
        why: String,
        /// The hand-off's notices (an identifier it dropped), then the
        /// document's own, which would otherwise be lost with it.
        notices: Vec<String>,
    },
    /// The user cancelled a lookup by identifier; nothing was stored.
    Cancelled,
    /// The source's file is kept in the shared library, which cannot give it
    /// now (not reachable, the file removed, damaged or online-only); why,
    /// naming the file. A re-derive keeps the older text: the fault may pass.
    Library(String),
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
            OpenError::Mismatch {
                paper,
                why,
                notices,
            } => {
                write!(
                    f,
                    "This document does not match the stored paper {paper} ({why}), so it was not stored. To look at it anyway, turn on \u{201c}{KEEP_MISMATCHED_LABEL}\u{201d} in Settings."
                )?;
                notices.iter().try_for_each(|n| write!(f, " {n}"))
            }
            OpenError::HandoffMismatch {
                sender,
                why,
                notices,
            } => {
                write!(
                    f,
                    "The document {sender} handed over is a different paper than the identifiers sent with it ({why}), so it was not stored."
                )?;
                notices.iter().try_for_each(|n| write!(f, " {n}"))
            }
            OpenError::Cancelled => write!(f, "The lookup was cancelled."),
            OpenError::Library(why) => write!(f, "{why}."),
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
/// points into the stored text: the caller passes `has_dependents`, so a text
/// with a conversation turn or a PICO citation is never re-derived.
pub fn needs_rederive(
    stored_version: &str,
    current_version: &str,
    failed_version: Option<&str>,
    has_dependents: bool,
) -> bool {
    stored_version != current_version && failed_version != Some(current_version) && !has_dependents
}

/// Opens a file or paste (on the stored paper its JATS identifiers name, else
/// a new paper; one contradicting that paper is refused with `Mismatch`, or
/// kept apart under `ctx.keep_mismatched`, see `place`), or an identifier
/// (see `open_identifier`). A document carrying no identifiers opens on the
/// paper that last stored the same kind and bytes, else a new one (#44). A
/// paste (no file name) of bytes stored as a file is converted with that
/// file's name as the title hint, as a re-derive is, so it reads as the
/// stored text and keeps the paper's title and confirmation. Stored anew, it
/// keeps that name, so a re-derive of it uses the same hint. A hand-off with
/// full text opens as a file does (see `convert_request`), one without as a
/// lookup by its identifiers (see `open_by_identifiers`). With a library
/// chosen, new sources are stored as waiting and moved in by `finish`.
pub fn open(ctx: &mut OpenContext, request: OpenRequest) -> Result<OpenOutcome, OpenError> {
    let result = (|| match request {
        OpenRequest::Identifier { input } => open_identifier(ctx, &input),
        OpenRequest::Handoff(h) if h.document.is_none() => {
            open_by_identifiers(ctx, h.identifiers, h.title.as_deref(), h.notices)
        }
        other => {
            // A hand-off's title and notices, which a conversion under a stored name keeps.
            let (title, handed_notices) = match &other {
                OpenRequest::Handoff(h) => (h.title.clone(), h.notices.clone()),
                _ => (None, Vec::new()),
            };
            let mut doc = convert_request(ctx, other)?;
            if doc.ids == Identifiers::default() && doc.source.file_name.is_none() {
                let digest = sha256_hex(&doc.source.bytes);
                if let Some((_, Some(name))) =
                    holder_of_source(&mut *ctx.db, doc.source.kind, &digest)?
                {
                    log::info!(
                        "a paste matches the stored file {name:?}; converting it under that name"
                    );
                    let s = doc.source;
                    doc = convert(
                        ctx,
                        s.kind,
                        &s.origin,
                        Some(name),
                        title.as_deref(),
                        s.bytes,
                        &PdfLimit::default(),
                    )?;
                    doc.notices.splice(0..0, handed_notices);
                }
            }
            let keep = ctx.keep_mismatched;
            let new_place = ctx.library.new_place();
            transaction_with(&mut *ctx.db, |tx| {
                let target = if doc.ids == Identifiers::default() {
                    let digest = sha256_hex(&doc.source.bytes);
                    let held = holder_of_source(tx, doc.source.kind, &digest)?;
                    if let Some((p, _)) = &held {
                        log::info!(
                            "a document without identifiers matches paper {} by its bytes",
                            p.0
                        );
                    }
                    held.map(|(p, _)| p)
                } else {
                    find_paper(tx, &doc.ids)?
                };
                let (paper, notices) = place(tx, target, &doc, keep)?;
                persist(tx, &paper, doc, notices, new_place)
            })
        }
    })()
    .and_then(|outcome| finish(ctx, outcome));
    log_failure("open", result)
}

/// Adds a file or paste to an existing paper (the fallback for `NeedsFullText`).
/// An identifier or hand-off request is refused with
/// `OpenError::Intake(Unsupported)`, and a JATS file whose own identifiers
/// name another paper with `WrongPaper`. New sources are moved into a chosen
/// library by `finish`, as an open's are.
pub fn attach(
    ctx: &mut OpenContext,
    paper_id: &str,
    request: OpenRequest,
) -> Result<OpenOutcome, OpenError> {
    let result = (|| {
        let refused = match request {
            OpenRequest::Identifier { .. } => Some("an identifier"),
            OpenRequest::Handoff(_) => Some("a hand-off"),
            _ => None,
        };
        if let Some(what) = refused {
            return Err(OpenError::Intake(IntakeError::Unsupported(format!(
                "{what} cannot be attached to a paper"
            ))));
        }
        let paper = existing_paper(&mut *ctx.db, paper_id)?;
        let doc = convert_request(ctx, request)?;
        let new_place = ctx.library.new_place();
        transaction_with(&mut *ctx.db, |tx| {
            if let Some(why) = identifier_conflict(tx, &paper, &doc.ids)? {
                return Err(OpenError::WrongPaper(why));
            }
            let notices = add_identifiers(tx, &paper, &doc.ids)?;
            persist(tx, &paper, doc, notices, new_place)
        })
    })()
    .and_then(|outcome| finish(ctx, outcome));
    log_failure("attach", result)
}

/// Reopens a stored paper, re-deriving its canonical text from the stored
/// original when the converter has a newer version it has not already failed
/// with; a failed re-derive opens the old text with a notice. The original
/// is read from the app's storage or the paper library; the network is never
/// touched. Like an open, a `Ready` reopen then moves the paper's waiting
/// sources into the library (see `finish`).
pub fn reopen(ctx: &mut OpenContext, paper_id: &str) -> Result<OpenOutcome, OpenError> {
    let result = (|| {
        let paper = existing_paper(&mut *ctx.db, paper_id)?;
        reopen_paper(ctx, &paper, Vec::new())
    })()
    .and_then(|outcome| finish(ctx, outcome));
    log_failure("reopen", result)
}

/// After an open, attach or reopen has committed: moves the paper's waiting
/// sources into the library when it is usable (each failure noted on the
/// source), and ends a `Ready` outcome with the banner's notice while one
/// still waits ([`waiting_notice`]). An unreachable library never stops an
/// open (design 2026-11-13, §4.6), and since the paper is already stored, a
/// failure here is logged and becomes a notice, not a failed open.
fn finish(ctx: &mut OpenContext, outcome: OpenOutcome) -> Result<OpenOutcome, OpenError> {
    let OpenOutcome::Ready {
        paper_id,
        mut notices,
    } = outcome
    else {
        return Ok(outcome);
    };
    let paper = existing_paper(&mut *ctx.db, &paper_id)?;
    let waiting = settle_paper(&mut *ctx.db, ctx.library, &paper)
        .and_then(|()| waiting_notice(&mut *ctx.db, &paper));
    match waiting {
        Ok(notice) => notices.extend(notice),
        Err(e) => {
            log::error!(
                "paper {}: its files could not be put in the library: {e}",
                paper.0
            );
            notices.push(format!(
                "This paper's files could not be put in the paper library: {e}."
            ));
        }
    }
    Ok(OpenOutcome::Ready { paper_id, notices })
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
    /// Notices for the user about the conversion (an invalid identifier
    /// dropped, what the PDF engine read past).
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

/// Converts and assesses raw bytes, a PDF's extraction stopping at
/// `pdf_limit`. The converter's title hint is the file name without
/// extension, the only hint a re-derive has too (see [`reopen_paper`]). The
/// title is the canonical title, else `record_title` (a looked-up record's),
/// else the hint, else [`UNTITLED_PAPER`].
fn convert(
    ctx: &OpenContext,
    kind: SourceKind,
    origin: &str,
    file_name: Option<String>,
    record_title: Option<&str>,
    bytes: Vec<u8>,
    pdf_limit: &PdfLimit,
) -> Result<Converted, OpenError> {
    let stem = file_name.as_deref().and_then(file_stem);
    let (text, decoded, engine) = to_canonical(
        kind,
        &bytes,
        stem.as_deref(),
        ctx.pdf,
        ctx.scratch_dir,
        pdf_limit,
    )?;
    let assessment = assess(&text, engine.as_deref())?;
    if let Some(n) = &engine {
        log::warn!("the PDF from {origin} ({file_name:?}): {n}");
    }
    let title = text
        .title_span
        .and_then(|s| text.span_text(s))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or(record_title.map(str::to_string))
        .or(stem)
        .unwrap_or_else(|| UNTITLED_PAPER.to_string());
    let (ids, mut notices) = if kind == SourceKind::Jats {
        jats_identifiers(&bytes)
    } else {
        Default::default()
    };
    notices.extend(engine);
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

/// Converts a file, paste or hand-off request; refuses an identifier and a
/// hand-off without full text. None has a cancel switch (only a lookup does),
/// so a PDF is read without a limit. A hand-off's document is converted under
/// [`Handoff::origin`] with the hand-off's title as the record title; the
/// identifiers sent with it are the claim, its own (JATS) fill the gaps (see
/// [`with_claim`]), and the hand-off's notices come before its own. Its `pdf`
/// that is not one is `NotPdf`, named by its file name, else as the document
/// its sender handed over.
fn convert_request(ctx: &OpenContext, request: OpenRequest) -> Result<Converted, OpenError> {
    match request {
        OpenRequest::File { name, bytes } => {
            let kind = detect_kind(name.as_deref(), &bytes)?;
            convert(
                ctx,
                kind,
                ORIGIN_UPLOAD,
                name,
                None,
                bytes,
                &PdfLimit::default(),
            )
        }
        OpenRequest::Paste { text } => {
            let bytes = text.into_bytes();
            let kind = detect_kind(None, &bytes)?;
            convert(
                ctx,
                kind,
                ORIGIN_PASTE,
                None,
                None,
                bytes,
                &PdfLimit::default(),
            )
        }
        OpenRequest::Handoff(h) => {
            let origin = h.origin();
            let sender = h.sender.unwrap_or_else(|| LINK_ORIGIN.to_string());
            let Some(d) = h.document else {
                return Err(OpenError::Intake(IntakeError::Unsupported(
                    "a hand-off without full text is not a file".into(),
                )));
            };
            if d.kind == SourceKind::Pdf && !d.bytes.starts_with(PDF_MAGIC) {
                let name = d
                    .file_name
                    .unwrap_or_else(|| format!("the document {sender} handed over"));
                return Err(IntakeError::NotPdf(name).into());
            }
            let mut doc = convert(
                ctx,
                d.kind,
                &origin,
                d.file_name,
                h.title.as_deref(),
                d.bytes,
                &PdfLimit::default(),
            )?;
            doc.notices.splice(0..0, h.notices);
            match with_claim(h.identifiers, doc.ids) {
                Ok(ids) => {
                    doc.ids = ids;
                    Ok(doc)
                }
                Err(why) => {
                    log::warn!(
                        "the document {sender} handed over contradicts its identifiers: {why}; notices: {:?}",
                        doc.notices
                    );
                    Err(OpenError::HandoffMismatch {
                        sender,
                        why,
                        notices: doc.notices,
                    })
                }
            }
        }
        OpenRequest::Identifier { .. } => Err(OpenError::Intake(IntakeError::Unsupported(
            "an identifier is not a file".into(),
        ))),
    }
}

/// The identifiers `claim`ed for a document, with the gaps filled from those
/// it carries itself (`own`). A kind present in both with different values
/// is the reason they disagree, naming the kind and both values.
fn with_claim(claim: Identifiers, own: Identifiers) -> Result<Identifiers, String> {
    let pick = |label: &str, claimed: Option<String>, found: Option<String>| match (claimed, found)
    {
        (Some(c), Some(f)) if c != f => Err(format!(
            "its {label} is {f}, but the {label} sent with it is {c}"
        )),
        (c, f) => Ok(c.or(f)),
    };
    Ok(Identifiers {
        pmid: pick("PMID", claim.pmid, own.pmid)?,
        doi: pick("DOI", claim.doi, own.doi)?,
        pmc_id: pick("PMC ID", claim.pmc_id, own.pmc_id)?,
    })
}

/// Writes the source (with the notices, so later opens show them again),
/// canonical text and title, and marks the paper opened. The outcome carries
/// the document's notices, then `notices`. The same document again (see
/// [`stored_copy`]) stores no second source; this open's notices replace the
/// stored source's (even with none), and the title and opened time are
/// written either way (#27). Its text is stored on that source only when the
/// stored one is another (in practice older) converter's, keeping the user's
/// "this is the whole paper" as a re-derive does (#43). A document stored
/// anew over a confirmed text says so in the outcome only (the confirmation
/// does not carry over). A stored text that cannot be read does not block
/// the open: the document is stored afresh, with a notice in the outcome
/// only. A new source is stored at `place`; so are the bytes of a stored
/// copy kept in the library, which may have lost its file, so a reopen puts
/// it back. Runs inside the caller's transaction.
fn persist(
    db: &mut dyn Db,
    paper: &PaperId,
    doc: Converted,
    notices: Vec<String>,
    place: SourcePlace,
) -> Result<OpenOutcome, OpenError> {
    let notices: Vec<String> = doc.notices.into_iter().chain(notices).collect();
    // A notice for this outcome only, not stored with the source.
    let mut once = None;
    let copy = match stored_copy(db, paper, &doc.source, &doc.text) {
        Ok(copy) => copy,
        Err(e) => {
            log::warn!(
                "paper {}: the stored text could not be read ({e}); storing this document afresh",
                paper.0
            );
            once = Some(format!(
                "The stored text of this paper could not be read ({e}); this document was stored in its place."
            ));
            None
        }
    };
    // A text a conversation or a PICO run points into is kept as it is: a new reading would move every span.
    let copy = match copy {
        Some(StoredCopy::Older { source_id, .. }) if latest_has_dependents(db, paper)? => {
            log::info!(
                "paper {}: the stored text has conversations or PICO citations; keeping it instead of storing a new reading",
                paper.0
            );
            Some(StoredCopy::Same(source_id))
        }
        other => other,
    };
    if let Some(StoredCopy::Same(source_id) | StoredCopy::Older { source_id, .. }) = copy {
        // A library file may have gone since (a manual clear, a deletion by hand): the bytes in hand go back, settled after the commit (a file still there is found and kept).
        if bring_back(db, source_id, &doc.source.bytes, place)? {
            log::info!(
                "paper {}: its library source {source_id} is kept here again until it is settled",
                paper.0
            );
        }
    }
    match copy {
        Some(StoredCopy::Same(source_id)) => {
            log::info!(
                "paper {} already holds this document; no second copy stored",
                paper.0
            );
            record_source_notices(db, source_id, &notices)?;
        }
        Some(StoredCopy::Older {
            source_id,
            confirmed,
        }) => {
            log::info!(
                "paper {} already holds this document, read by another converter; storing the new text on it",
                paper.0
            );
            record_source_notices(db, source_id, &notices)?;
            let id = save_canonical(db, paper, source_id, &doc.text, &doc.assessment)?;
            if confirmed {
                confirm_full_text(db, paper, id)?;
            }
        }
        None => {
            log::info!("paper {}: storing this document as a new source", paper.0);
            // Read again only after `stored_copy` read it without fault.
            let was_confirmed = once.is_none()
                && latest_canonical(db, paper)?.is_some_and(|t| t.confirmed_full_text);
            if was_confirmed {
                log::info!(
                    "paper {}: the confirmed text is replaced by an unconfirmed one",
                    paper.0
                );
                once = Some(REPLACED_CONFIRMED_NOTICE.to_string());
            }
            let source_id = save_source(db, paper, doc.source, place)?;
            if !notices.is_empty() {
                record_source_notices(db, source_id, &notices)?;
            }
            save_canonical(db, paper, source_id, &doc.text, &doc.assessment)?;
        }
    }
    set_title(db, paper, &doc.title)?;
    touch_opened(db, paper)?;
    Ok(OpenOutcome::Ready {
        paper_id: paper.0.clone(),
        notices: notices.into_iter().chain(once).collect(),
    })
}

/// Whether spans point into canonical text `canonical_id`: a conversation turn
/// or a PICO citation. Such a text is never re-derived or replaced.
fn has_dependents(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError> {
    Ok(has_turns(db, canonical_id)? || has_citations(db, canonical_id)?)
}

/// Whether the paper's latest canonical text has dependents ([`has_dependents`]).
fn latest_has_dependents(db: &mut dyn Db, paper: &PaperId) -> Result<bool, StoreError> {
    match latest_canonical(db, paper)? {
        Some(t) => has_dependents(db, t.id),
        None => Ok(false),
    }
}

/// A stored source holding the very document being opened (see [`stored_copy`]).
enum StoredCopy {
    /// The paper's latest text already is the document's; the source's row id.
    Same(i64),
    /// The paper's latest text is another (in practice older) converter's
    /// reading of the same source; the document's text is stored on that
    /// source as a new canonical text, which becomes the latest.
    Older {
        /// The stored source's row id.
        source_id: i64,
        /// Whether the user confirmed the older text as the whole paper.
        confirmed: bool,
    },
}

/// The stored source to reuse instead of storing `source` again, or `None`
/// to store it. Only the source of the paper's latest canonical text is
/// considered, and only when it has the same kind and digest. It is
/// [`StoredCopy::Same`] when that text equals `text` (same converter version,
/// title and sections), and [`StoredCopy::Older`] when it was made by another
/// (in practice older) converter from the same file stem, the converter's only
/// title hint, as for a re-derive (#43). With no converter bump, only a rename
/// that changes the text (a plain text's name is its title) is stored anew;
/// after a bump, a renamed file of any kind is, unconfirmed, even where its
/// name would not change its text (shortcut: the stem is compared for every
/// kind; compare only kinds whose converter uses the hint if this bites). Only
/// the latest text is compared: A, then B, then A again stores A a second
/// time.
fn stored_copy(
    db: &mut dyn Db,
    paper: &PaperId,
    source: &NewSource,
    text: &CanonicalText,
) -> Result<Option<StoredCopy>, StoreError> {
    let Some(stored) = latest_canonical(db, paper)? else {
        return Ok(None);
    };
    let same_text = stored.text == *text;
    if !same_text && stored.text.converter_version == text.converter_version {
        return Ok(None);
    }
    let src = source_by_id(db, stored.source_id)?;
    if src.kind != source.kind || src.sha256 != sha256_hex(&source.bytes) {
        return Ok(None);
    }
    let stem = |name: &Option<String>| name.as_deref().and_then(file_stem);
    Ok(if same_text {
        Some(StoredCopy::Same(src.id))
    } else if stem(&src.file_name) == stem(&source.file_name) {
        Some(StoredCopy::Older {
            source_id: src.id,
            confirmed: stored.confirmed_full_text,
        })
    } else {
        None
    })
}

/// The paper `doc` lands on, and the notices about its identifiers. `target`
/// is the paper it was matched to; with none, a new paper holding its
/// identifiers. A document whose identifiers contradict the target's is
/// refused with [`OpenError::Mismatch`], or, with `keep_mismatched`, lands on
/// a paper of its own, with a notice naming the target: the first other stored
/// paper one of its identifiers names that it does not contradict (where a
/// repeat of it landed), else a new one. Otherwise its identifiers are added
/// to the target, with a notice for each not recorded.
fn place(
    db: &mut dyn Db,
    target: Option<PaperId>,
    doc: &Converted,
    keep_mismatched: bool,
) -> Result<(PaperId, Vec<String>), OpenError> {
    let ids = &doc.ids;
    let Some(target) = target else {
        return Ok((create_paper(db, ids, None)?, Vec::new()));
    };
    let Some(why) = contradiction(db, &target, ids)? else {
        let notices = add_identifiers(db, &target, ids)?;
        return Ok((target, notices));
    };
    let paper = quoted_title(db, &target)?;
    if !keep_mismatched {
        log::warn!("document refused for paper {}: {why}", target.0);
        return Err(OpenError::Mismatch {
            paper,
            why,
            notices: doc.notices.clone(),
        });
    }
    log::warn!(
        "document contradicting paper {} ({why}) kept as a paper of its own",
        target.0
    );
    let mut own = None;
    for p in holders(db, ids)? {
        if p != target && contradiction(db, &p, ids)?.is_none() {
            own = Some(p);
            break;
        }
    }
    let own = match own {
        Some(p) => p,
        None => create_paper(db, &Identifiers::default(), None)?,
    };
    // Past tense: the notice is stored, and the setting may be off when it is read again.
    let mut notices = vec![format!(
        "This document does not match the stored paper {paper} ({why}). It was opened as a paper of its own because \u{201c}{KEEP_MISMATCHED_LABEL}\u{201d} was on in Settings."
    )];
    notices.extend(add_identifiers(db, &own, ids)?);
    Ok((own, notices))
}

/// Opens by identifier: a stored paper with text reopens offline (a cancel
/// does not stop that); otherwise the paper's record is looked up, then its
/// full text (both outside any transaction; see `resolve` and `retrieve`),
/// and, unless the user cancelled (before or while the document
/// found was converted; a cancel stops a PDF extraction), the paper of the
/// identifier typed is stored (created if new). The document found is stored
/// on it, with the PDF found beside a JATS document for the Original tab, or a
/// notice of why there is none ([`keep_pdf_beside`]); one contradicting it is refused, leaving it `NeedsFullText` with
/// the refusal as `reason`, or under `ctx.keep_mismatched` lands on a paper
/// of its own (see `place`). Either way the paper keeps a note of it. A found
/// document at fault itself (unreadable, encrypted) also leaves it
/// `NeedsFullText` ("could not be read"); a fault of ours converting it (the
/// PDF engine, the scratch file) is an error and stores nothing. The record's
/// title names a new paper (or one still untitled) and a document without a
/// title of its own. A stored paper found by one identifier records the
/// others (a hand-off sends up to three), each not recorded noted as the
/// record's notes are. The record lookup's notes (see `Resolved::notes`) are
/// notices of the document stored on the paper typed, or end the
/// `NeedsFullText` reason and are kept with the paper's note, so a reopen
/// shows them again.
fn open_identifier(ctx: &mut OpenContext, input: &str) -> Result<OpenOutcome, OpenError> {
    open_by_identifiers(ctx, parse_identifier(input)?, None, Vec::new())
}

/// Opens by validated identifiers, as [`open_identifier`] describes. `title`
/// names the paper only when the record has none; `notes` come before the
/// record lookup's notes (and before a reopened paper's identifier notices).
fn open_by_identifiers(
    ctx: &mut OpenContext,
    ids: Identifiers,
    title: Option<&str>,
    notes: Vec<String>,
) -> Result<OpenOutcome, OpenError> {
    if let Some(paper) = find_paper(&mut *ctx.db, &ids)? {
        if latest_canonical(&mut *ctx.db, &paper)?.is_some() {
            let added = transaction_with(&mut *ctx.db, |tx| add_identifiers(tx, &paper, &ids))?;
            return reopen_paper(ctx, &paper, notes.into_iter().chain(added).collect());
        }
    }
    let resolved = resolve(ctx.source, &ids, ctx.attempts, ctx.base_delay, ctx.limit);
    let retrieved = retrieve(
        ctx.source,
        &resolved,
        ctx.attempts,
        ctx.base_delay,
        ctx.limit,
    );
    let title = resolved.title.as_deref().or(title);
    let notes: Vec<String> = notes
        .into_iter()
        .chain(resolved.notes.iter().cloned())
        .collect();
    let notes = &notes;
    let mut pdf_beside = None;
    let (doc, reason, unreachable) = match retrieved {
        Retrieved::Found {
            kind,
            origin,
            bytes,
            pdf_beside: beside,
        } => {
            pdf_beside = beside;
            match convert(
                ctx,
                kind,
                &origin,
                None,
                title,
                bytes,
                &ctx.limit.pdf_limit(),
            ) {
                Ok(doc) => (Some(doc), String::new(), false),
                // Stopped by the user's cancel. A cancel the switch does not show
                // is not the user's, so it falls through to an error below.
                Err(OpenError::Intake(IntakeError::Pdf(e)))
                    if e.kind == PdfErrorKind::Cancelled
                        && ctx.limit.remaining() == Err(Stop::Cancelled) =>
                {
                    log::info!(
                        "lookup cancelled by the user while the PDF from {origin} was read: {e}"
                    );
                    return Err(OpenError::Cancelled);
                }
                Err(OpenError::Intake(e)) if e.is_the_documents() => {
                    // Keep the paper (and its identifiers) so the user can add the text by hand.
                    log::warn!("the document from {origin} could not be read: {e}");
                    (
                        None,
                        format!("The document from {origin} could not be read: {e}"),
                        false,
                    )
                }
                // Our fault, not the document's (a PDF engine fault, a scratch
                // file, a canonical text bmlib refused): an error, as on upload,
                // never "could not be read" (#11).
                Err(e) => {
                    log::error!(
                        "the document from {origin} for {ids:?} could not be converted: {e}"
                    );
                    // A cancel pressed meanwhile is what the user sees.
                    if ctx.limit.remaining() == Err(Stop::Cancelled) {
                        return Err(OpenError::Cancelled);
                    }
                    return Err(e);
                }
            }
        }
        Retrieved::NotFound { reason } => (None, reason, false),
        Retrieved::Unreachable { reason } => (None, reason, true),
        Retrieved::Cancelled => return Err(OpenError::Cancelled),
    };
    // A cancel pressed while the found document was converted still stores nothing.
    if ctx.limit.remaining() == Err(Stop::Cancelled) {
        log::info!("lookup cancelled by the user after the sources answered");
        return Err(OpenError::Cancelled);
    }
    let keep = ctx.keep_mismatched;
    let new_place = ctx.library.new_place();
    transaction_with(&mut *ctx.db, |tx| {
        // The paper of the identifier typed, whatever the document's other
        // identifiers name: so a repeated lookup lands in the same place (#25).
        // A hand-off's other identifiers, and why any was not recorded.
        let mut added = Vec::new();
        let paper = match find_paper(tx, &ids)? {
            Some(p) => {
                // A paper an earlier lookup left untitled takes the record's title.
                if let Some(t) = title {
                    if paper_title(tx, &p)? == UNTITLED_PAPER {
                        set_title(tx, &p, t)?;
                    }
                }
                added = add_identifiers(tx, &p, &ids)?;
                p
            }
            None => create_paper(tx, &ids, title)?,
        };
        let notes: Vec<String> = notes.iter().cloned().chain(added).collect();
        let notes = &notes;
        let refused = match doc {
            Some(doc) => {
                let origin = doc.source.origin.clone();
                match place(tx, Some(paper.clone()), &doc, keep) {
                    Ok((landed, mut notices)) => {
                        // The notes, and the PDF bmlib sought by the identifiers typed,
                        // are the paper typed's, not one kept apart from it.
                        let pdf_beside = if landed == paper {
                            notices.extend(notes.iter().cloned());
                            match pdf_beside {
                                // Stored with the source, so a reopen says it again.
                                Some(Err(why)) => {
                                    notices.push(why);
                                    None
                                }
                                Some(Ok(pdf)) => Some(pdf),
                                None => None,
                            }
                        } else {
                            if pdf_beside.is_some() {
                                log::info!(
                                    "paper {}: the PDF sought for the identifiers typed is not kept beside a document kept apart from it",
                                    landed.0
                                );
                            }
                            None
                        };
                        let outcome = persist(tx, &landed, doc, notices, new_place)?;
                        if let Some(pdf) = pdf_beside {
                            keep_pdf_beside(tx, &landed, pdf, new_place)?;
                        }
                        if landed != paper {
                            // Kept apart: the paper typed says where it went.
                            let note = format!(
                                "From {origin}: the document found does not match this paper, so it was opened as a paper of its own, {}.",
                                quoted_title(tx, &landed)?
                            );
                            set_refusal(tx, &paper, Some(&with_notes(&note, notes)))?;
                        }
                        return Ok(outcome);
                    }
                    // Keep the paper so the user can add the right text by hand.
                    Err(e @ OpenError::Mismatch { .. }) => Some(format!("From {origin}: {e}")),
                    Err(e) => return Err(e),
                }
            }
            None => None,
        };
        // This lookup's note replaces the last one's, or clears it when it has none.
        let note = (refused.is_some() || !notes.is_empty())
            .then(|| with_notes(refused.as_deref().unwrap_or(NO_FULL_TEXT_YET), notes));
        set_refusal(tx, &paper, note.as_deref())?;
        touch_opened(tx, &paper)?;
        Ok(OpenOutcome::NeedsFullText {
            paper_id: paper.0,
            reason: with_notes(&refused.unwrap_or(reason), notes),
            unreachable,
        })
    })
}

/// Keeps the PDF found beside a JATS document (see [`Retrieved::Found`]) with
/// the source of `paper`'s latest text, for the *Original* tab, unless that
/// source is a PDF itself or has one already. Runs after [`persist`], inside
/// its transaction; why there is no PDF to keep is one of the notices given
/// to [`persist`], so it is stored with the source. It is stored at `place`.
fn keep_pdf_beside(
    db: &mut dyn Db,
    paper: &PaperId,
    pdf: PdfBeside,
    place: SourcePlace,
) -> Result<(), OpenError> {
    let PdfBeside { bytes, origin } = pdf;
    let Some(text) = latest_canonical(db, paper)? else {
        // `persist` has just stored a text, so this is a broken invariant.
        log::error!(
            "paper {}: no text after storing one; the PDF from {origin} is not kept",
            paper.0
        );
        return Ok(());
    };
    let (kind, _) = source_info(db, text.source_id)?;
    if kind == SourceKind::Pdf {
        log::info!(
            "paper {}: its text is read from a PDF; the PDF from {origin} is not kept beside it",
            paper.0
        );
    } else if save_pdf_beside(db, paper, text.source_id, &origin, bytes, place)? {
        log::info!(
            "paper {}: the PDF from {origin} kept for the Original tab",
            paper.0
        );
    }
    Ok(())
}

/// `text`, then each of `notes`, as one passage.
fn with_notes(text: &str, notes: &[String]) -> String {
    std::iter::once(text)
        .chain(notes.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when a failed re-derive would fail again for the same bytes and
/// converter, so it is recorded (#22): the file's fault, or bmlib refusing our
/// text. A scratch-file fault (disk full, folder purged), a PDF engine fault
/// (out of memory), an HTML parse deadline (it depends on the machine's
/// speed, not only on the bytes) or the library not giving the file may
/// pass, so the next open tries again.
fn rederive_failure_recurs(e: &OpenError) -> bool {
    match e {
        OpenError::Intake(i) => {
            (i.is_the_documents() && !matches!(i, IntakeError::Html(HtmlError::TimedOut { .. })))
                || matches!(i, IntakeError::Canonical(_))
        }
        // The library may be back, or the file put right, by the next open.
        OpenError::Library(_) => false,
        _ => true,
    }
}

/// Reopens a known paper: `NeedsFullText` when it has no text (the reason
/// being the refusal stored by its last lookup, if any), else `Ready` after
/// an optional re-derive, carrying the notices stored with the text's source
/// (a re-derive putting its own [`engine_notice`](crate::intake::engine_notice),
/// or none, in place of the stored one, there from then on), then `notices`,
/// then, while a re-derive with the current converter has failed, a notice
/// saying so (the old text and its engine notice stay). A failed attempt is
/// recorded so it is not repeated until the converter changes again (#22),
/// unless the fault may pass on a later open: the scratch file's, the PDF
/// engine's, an HTML parse deadline, or the library not giving the file
/// ([`rederive_failure_recurs`]).
fn reopen_paper(
    ctx: &mut OpenContext,
    paper: &PaperId,
    mut notices: Vec<String>,
) -> Result<OpenOutcome, OpenError> {
    let Some(stored) = latest_canonical(&mut *ctx.db, paper)? else {
        // Callers with notices reach here only for a paper that has a text.
        debug_assert!(notices.is_empty(), "notices dropped: {notices:?}");
        touch_opened(&mut *ctx.db, paper)?;
        let reason = refusal(&mut *ctx.db, paper)?.unwrap_or_else(|| NO_FULL_TEXT_YET.into());
        return Ok(OpenOutcome::NeedsFullText {
            paper_id: paper.0.clone(),
            reason,
            unreachable: false,
        });
    };
    // The source the stored text was derived from, not merely the newest one.
    let src = source_by_id(&mut *ctx.db, stored.source_id)?;
    let stored_notices = source_notices(&mut *ctx.db, src.id)?;
    let current = current_converter_version(src.kind);
    // A failure under an older converter is stale: the current one tries afresh.
    let mut failure = stored
        .rederive_failure
        .clone()
        .filter(|f| f.converter_version == current);
    let mut rederived = None;
    // The source's notices after a re-derive, when it changes them.
    let mut renoticed = None;
    // True when this open's failure is to be recorded (see the `Err` arm).
    let mut record = false;
    if needs_rederive(
        &stored.text.converter_version,
        &current,
        failure.as_ref().map(|f| f.converter_version.as_str()),
        has_dependents(&mut *ctx.db, stored.id)?,
    ) {
        // The same title hint as the first conversion, or the text (and every offset) changes.
        let stem = src.file_name.as_deref().and_then(file_stem);
        let bytes = match read_source(&mut *ctx.db, ctx.library, &src) {
            Ok(bytes) => Ok(bytes),
            Err(LibraryFault::Store(e)) => return Err(e.into()),
            Err(LibraryFault::Unavailable(why)) => Err(why),
        };
        match bytes.map_err(OpenError::Library).and_then(|bytes| {
            to_canonical(
                src.kind,
                &bytes,
                stem.as_deref(),
                ctx.pdf,
                ctx.scratch_dir,
                // Only a lookup is cancelled, and a cancel must not read as a failed re-derive.
                &PdfLimit::default(),
            )
            .map_err(OpenError::from)
            .and_then(|(t, _, n)| Ok((assess(&t, n.as_deref())?, t, n)))
        }) {
            Ok((a, t, n)) => {
                // An engine notice describes one reading of the bytes, so the
                // re-derive's replaces the stored one, or removes it if none.
                if let Some(n) = &n {
                    log::warn!("re-deriving paper {}: {n}", paper.0);
                }
                let updated: Vec<String> = stored_notices
                    .iter()
                    .filter(|s| !is_engine_notice(s))
                    .cloned()
                    .chain(n)
                    .collect();
                if updated != stored_notices {
                    renoticed = Some(updated);
                }
                rederived = Some((src.id, t, a));
            }
            Err(e) => {
                // The old text is unchanged and correct for its converter, so the
                // paper stays Ready, with a notice.
                log::error!(
                    "re-deriving paper {} with {current} failed, keeping old text: {e}",
                    paper.0
                );
                record = rederive_failure_recurs(&e);
                failure = Some(RederiveFailure {
                    converter_version: current.clone(),
                    error: e.to_string(),
                });
            }
        }
    }
    // What the user was told when that source was stored comes first.
    let shown = renoticed.as_ref().unwrap_or(&stored_notices);
    notices.splice(0..0, shown.iter().cloned());
    // `failure` and `rederived` are never both set: a recorded failure blocks the attempt.
    if let Some(f) = &failure {
        notices.push(format!(
            "This text was made by an older converter ({}). Reading the original again with {current} failed, so the older text is shown: {}",
            stored.text.converter_version, f.error
        ));
    }
    transaction_with(&mut *ctx.db, |tx| {
        if let Some(all) = &renoticed {
            record_source_notices(tx, src.id, all)?;
        }
        if let Some((source_id, text, assessment)) = &rederived {
            let id = save_canonical(tx, paper, *source_id, text, assessment)?;
            // Same source, same paper: the user's "this is the whole paper" still holds.
            if stored.confirmed_full_text {
                confirm_full_text(tx, paper, id)?;
            }
        }
        if let Some(f) = failure.as_ref().filter(|_| record) {
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
    use crate::config::ENGINE_NOTICE_LEAD;
    use crate::handoff::{HandedDocument, Handoff};
    use crate::intake::Decoded;
    use crate::retrieval::no_record;
    use crate::store::{
        has_original_pdf, latest_canonical, latest_source, open_memory_store, original_pdf,
        recent_papers, save_source, source_bytes,
    };
    use bmlib::db::{execute, fetch_one, Value};
    use bmlib::fulltext::models::{ContentKind, FullTextResult, TextBlock};
    use bmlib::fulltext::pdf_text::PdfError;
    use bmlib::fulltext::service::{FullTextError, FullTextRequest};
    use bmlib::publications::lookup::{LookupError, PublishedVersion, RecordLookup};
    use bmlib::publications::FetchedRecord;
    use rusqlite::Connection;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// A fault of the file or of bmlib's check recurs; a scratch file's, the
    /// PDF engine's or an HTML parse deadline may pass; a page too long to
    /// parse stays too long.
    #[test]
    fn which_rederive_failures_recur() {
        let intake = |i| rederive_failure_recurs(&OpenError::Intake(i));
        assert!(intake(IntakeError::Jats("x".into())));
        assert!(intake(IntakeError::Canonical("x".into())));
        assert!(!intake(IntakeError::Io("x".into())));
        assert!(!intake(IntakeError::Pdf(PdfError::engine("x"))));
        let timeout = HtmlError::TimedOut {
            parsed: 1,
            length: 2,
        };
        assert!(!intake(IntakeError::Html(timeout)));
        assert!(intake(IntakeError::Html(HtmlError::TooLarge { tags: 3 })));
        let too_many = HtmlError::TooManyElements {
            created: 9,
            parsed: 1,
            length: 2,
            tags: 3,
        };
        assert!(intake(IntakeError::Html(too_many)));
    }

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

    /// A full-text source that replays a queue and counts calls; its record
    /// lookup answers `lookup` (by default nothing found).
    struct FakeSource {
        queue: RefCell<VecDeque<Result<FullTextResult, FullTextError>>>,
        calls: RefCell<usize>,
        lookup: RecordLookup,
        /// The identifiers each record lookup was asked with.
        looked_up: RefCell<Vec<Identifiers>>,
    }
    impl FakeSource {
        /// A source that answers each `fetch` with the next of `items`.
        fn new(items: Vec<Result<FullTextResult, FullTextError>>) -> Self {
            Self {
                queue: RefCell::new(items.into()),
                calls: RefCell::new(0),
                lookup: no_record(),
                looked_up: RefCell::new(Vec::new()),
            }
        }
        /// How many times `fetch` was called.
        fn calls(&self) -> usize {
            *self.calls.borrow()
        }
    }
    impl FullTextSource for FakeSource {
        /// The record set in `lookup`, every time, keeping what it was asked.
        fn lookup(&self, ids: &Identifiers) -> Result<RecordLookup, LookupError> {
            self.looked_up.borrow_mut().push(ids.clone());
            Ok(self.lookup.clone())
        }
        /// The next queued result, counting the call.
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
        fn extract_blocks_within(
            &self,
            _: &Path,
            _: &PdfLimit,
        ) -> Result<Vec<TextBlock>, PdfError> {
            Ok(vec![])
        }
        fn page_texts_within(&self, _: &Path, _: &PdfLimit) -> Result<Vec<String>, PdfError> {
            Ok(vec![])
        }
    }

    /// A PDF backend whose extraction is `F`, given the limit it was passed.
    struct FnPdf<F>(F);
    impl<F: Fn(&PdfLimit) -> Result<Vec<TextBlock>, PdfError>> PdfTextExtractor for FnPdf<F> {
        fn name(&self) -> &'static str {
            "fn"
        }
        fn extract_blocks_within(
            &self,
            _: &Path,
            l: &PdfLimit,
        ) -> Result<Vec<TextBlock>, PdfError> {
            (self.0)(l)
        }
        fn page_texts_within(&self, _: &Path, l: &PdfLimit) -> Result<Vec<String>, PdfError> {
            (self.0)(l).map(|_| vec![])
        }
    }

    /// Looks up a PMID whose source answers with a PDF, read by `pdf` under
    /// `limit`, through `scratch` (else the PDF's own folder).
    fn open_found_pdf(
        c: &mut Connection,
        pdf: &dyn PdfTextExtractor,
        limit: &Limit,
        scratch: Option<&Path>,
    ) -> Result<OpenOutcome, OpenError> {
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
        let mut ctx = OpenContext {
            db: c,
            source: &src,
            pdf,
            scratch_dir: scratch.unwrap_or(&dir),
            attempts: 1,
            base_delay: Duration::ZERO,
            limit,
            keep_mismatched: false,
            library: &Configured::None,
        };
        let r = open(&mut ctx, ident("12345678"));
        let _ = std::fs::remove_dir_all(&dir);
        r
    }

    /// A JATS hit as bmlib would return it.
    fn jats_hit() -> Result<FullTextResult, FullTextError> {
        Ok(FullTextResult {
            source: "europepmc".into(),
            html: None,
            pdf_url: None,
            pdf_download_failure: None,
            pdf_extraction_failure: None,
            web_url: None,
            file_path: None,
            content_kind: ContentKind::Fulltext,
            jats_xml: Some(JATS.to_vec()),
            tier_faults: vec![],
            library_files: vec![],
            library_failure: None,
        })
    }

    /// Runs `f` with a context over `conn` and `source`.
    fn with_ctx<T>(
        conn: &mut Connection,
        source: &FakeSource,
        f: impl FnOnce(&mut OpenContext) -> T,
    ) -> T {
        with_keep(conn, source, false, f)
    }

    /// [`with_ctx`] with the debugging setting `keep_mismatched` as given.
    fn with_keep<T>(
        conn: &mut Connection,
        source: &FakeSource,
        keep_mismatched: bool,
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
            keep_mismatched,
            library: &Configured::None,
        };
        f(&mut ctx)
    }

    /// [`with_ctx`] with `pdf` as the PDF backend.
    fn with_pdf<T>(
        conn: &mut Connection,
        source: &FakeSource,
        pdf: &dyn PdfTextExtractor,
        f: impl FnOnce(&mut OpenContext) -> T,
    ) -> T {
        let scratch = std::env::temp_dir();
        let limit = Limit::new(Duration::from_secs(3600));
        let mut ctx = OpenContext {
            db: conn,
            source,
            pdf,
            scratch_dir: &scratch,
            attempts: 1,
            base_delay: Duration::ZERO,
            limit: &limit,
            keep_mismatched: false,
            library: &Configured::None,
        };
        f(&mut ctx)
    }

    /// A lookup request for the identifier typed as `s`.
    fn ident(s: &str) -> OpenRequest {
        OpenRequest::Identifier { input: s.into() }
    }

    /// The paper id of a `Ready` outcome; panics on any other.
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

    /// A pasted Markdown text opens ready, with no notices, its own converter
    /// version and no call to the network.
    #[test]
    fn paste_opens_ready() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let o = with_ctx(&mut c, &src, |x| {
            open(x, OpenRequest::Paste { text: MD.into() })
        })
        .unwrap();
        let (id, notices) = ready(o);
        assert!(notices.is_empty(), "{notices:?}");
        let id = PaperId(id);
        let canon = latest_canonical(&mut c, &id).unwrap().unwrap();
        assert_eq!(
            canon.text.converter_version,
            crate::intake::current_converter_version(SourceKind::Markdown)
        );
        assert_eq!(src.calls(), 0);
    }

    /// A PMID whose JATS is found opens ready; opening it again lands on the same
    /// paper from storage, without a second fetch.
    #[test]
    fn identifier_found_then_reopened_offline() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![jats_hit()]);
        let (a, first) = ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        let (b, second) = ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!(a, b);
        assert!(
            first.is_empty() && second.is_empty(),
            "{first:?} {second:?}"
        );
        assert_eq!(src.calls(), 1);
        let s = latest_source(&mut c, &PaperId(a)).unwrap().unwrap();
        assert_eq!(s.kind, SourceKind::Jats);
    }

    /// A JATS hit with a PDF beside it opens from the JATS and keeps the PDF
    /// for the Original tab; a second lookup reopens from storage and keeps no
    /// second copy (#739).
    #[test]
    fn a_pdf_beside_the_jats_is_kept_for_the_original_tab() {
        let dir = std::env::temp_dir().join(format!("sa-open-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("a.pdf");
        std::fs::write(&pdf, b"%PDF-1.7 beside").unwrap();
        let hit = || {
            Ok(FullTextResult {
                pdf_url: Some("https://europepmc.org/a.pdf".into()),
                file_path: Some(pdf.to_string_lossy().into()),
                ..jats_hit().unwrap()
            })
        };
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![hit()]);
        let (id, notices) = ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        let paper = PaperId(id);
        let text = latest_canonical(&mut c, &paper).unwrap().unwrap();
        let (kind, _) = source_info(&mut c, text.source_id).unwrap();
        assert_eq!(kind, SourceKind::Jats, "the text is the JATS's");
        assert!(has_original_pdf(&mut c, text.source_id).unwrap());
        assert_eq!(
            original_pdf(&mut c, &paper).unwrap(),
            Some(b"%PDF-1.7 beside".to_vec())
        );
        let n = |c: &mut Connection| {
            fetch_one(
                c,
                "SELECT count(*) AS n FROM sources WHERE pdf_beside IS NOT NULL",
                &[],
            )
            .unwrap()
            .unwrap()
            .get_i64("n")
            .unwrap()
        };
        assert_eq!(n(&mut c), 1);
        // A second lookup of a paper with text reopens it from storage.
        ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!(n(&mut c), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A JATS hit whose PDF could not be downloaded opens, and says why the
    /// Original tab has no PDF, again when the paper is reopened from storage.
    #[test]
    fn a_pdf_beside_the_jats_that_failed_is_a_notice() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Ok(FullTextResult {
            pdf_url: Some("https://www.mdpi.com/x/pdf".into()),
            pdf_download_failure: Some("http-403".into()),
            ..jats_hit().unwrap()
        })]);
        let (id, notices) = ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!(
            notices,
            ["A PDF of this paper was found at https://www.mdpi.com/x/pdf but its server answered HTTP 403, so the Original tab has no PDF to show."]
        );
        assert_eq!(original_pdf(&mut c, &PaperId(id)).unwrap(), None);
        let (_, again) = ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!(again, notices);
        assert_eq!(src.calls(), 1, "reopened from storage");
    }

    /// With the debugging setting on, a JATS kept apart from the paper typed
    /// keeps no PDF: bmlib sought it by the identifiers typed, so it may be
    /// the other paper's (#739).
    #[test]
    fn a_pdf_beside_a_jats_kept_apart_is_not_kept() {
        let dir = std::env::temp_dir().join(format!("sa-open-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("a.pdf");
        std::fs::write(&pdf, b"%PDF-1.7 typed").unwrap();
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Ok(FullTextResult {
            pdf_url: Some("https://unpaywall.example/a.pdf".into()),
            file_path: Some(pdf.to_string_lossy().into()),
            ..jats_hit().unwrap()
        })]);
        let (own, _) =
            ready(with_keep(&mut c, &src, true, |x| open(x, ident("10.1234/other"))).unwrap());
        assert_eq!(count(&mut c, "papers"), 2, "kept apart");
        assert_eq!(original_pdf(&mut c, &PaperId(own)).unwrap(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A looked-up record's title names the new paper, and a preprint's
    /// journal version ends the reason it needs full text, also on a reopen,
    /// until a later lookup notes nothing.
    #[test]
    fn identifier_record_titles_the_paper() {
        let mut c = open_memory_store().unwrap();
        let mut src = FakeSource::new(vec![Err(FullTextError::Unavailable("none.".into()))]);
        let mut record = FetchedRecord::new("A preprint", "europepmc");
        record.doi = Some("10.1101/p".into());
        src.lookup.record = Some(record);
        src.lookup.published_version = Some(PublishedVersion {
            pmid: "9".into(),
            doi: "10.1/j".into(),
            pmc_id: None,
        });
        let o = with_ctx(&mut c, &src, |x| open(x, ident("PPR123"))).unwrap();
        let OpenOutcome::NeedsFullText {
            reason, paper_id, ..
        } = o
        else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        assert!(reason.starts_with("none. This is a preprint."), "{reason}");
        assert!(reason.contains("PMID 9"), "{reason}");
        assert_eq!(recent_papers(&mut c).unwrap()[0].title, "A preprint");
        assert_eq!(src.calls(), 1);
        // Reopened from Papers, the journal version is still named.
        let again = with_ctx(&mut c, &src, |x| reopen(x, &paper_id)).unwrap();
        let OpenOutcome::NeedsFullText { reason, .. } = again else {
            panic!("expected NeedsFullText, got {again:?}")
        };
        assert!(reason.contains("This is a preprint."), "{reason}");
        // A later lookup with nothing to note clears it.
        src.lookup = no_record();
        with_ctx(&mut c, &src, |x| open(x, ident("PPR123"))).unwrap();
        let last = with_ctx(&mut c, &src, |x| reopen(x, &paper_id)).unwrap();
        assert!(
            matches!(&last, OpenOutcome::NeedsFullText { reason, .. } if reason == NO_FULL_TEXT_YET),
            "{last:?}"
        );
    }

    /// A PMID with no free full text keeps the paper and asks for the text;
    /// reopening it asks again without a fetch, and an attached file opens it.
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

    /// A fetch that could not reach the sources asks for the text with
    /// `unreachable` set, so the user is told to check the connection.
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
            keep_mismatched: false,
            library: &Configured::None,
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
            keep_mismatched: false,
            library: &Configured::None,
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

    /// A cancel pressed while a found PDF is converted reaches the
    /// extraction's PdfLimit (the fake stops on it, as MuPDF would) and
    /// stores nothing.
    #[test]
    fn cancel_during_conversion_stores_nothing() {
        let limit = Limit::new(Duration::from_secs(3600));
        let pdf = FnPdf(|l: &PdfLimit| {
            limit.cancel();
            Err(l
                .reached()
                .expect("the lookup's cancel reaches the PDF limit"))
        });
        let mut c = open_memory_store().unwrap();
        let r = open_found_pdf(&mut c, &pdf, &limit, None);
        assert!(matches!(r, Err(OpenError::Cancelled)), "{r:?}");
        assert!(recent_papers(&mut c).unwrap().is_empty());
    }

    /// A PDF that says it was cancelled when the user did not cancel is an
    /// error, never a paper stored with an empty reason.
    #[test]
    fn cancel_the_user_did_not_press_is_an_error() {
        let limit = Limit::new(Duration::from_secs(3600));
        let pdf = FnPdf(|_: &PdfLimit| Err(PdfError::cancelled()));
        let mut c = open_memory_store().unwrap();
        let r = open_found_pdf(&mut c, &pdf, &limit, None);
        assert!(
            matches!(&r, Err(OpenError::Intake(IntakeError::Pdf(e))) if e.kind == PdfErrorKind::Cancelled),
            "{r:?}"
        );
        assert!(recent_papers(&mut c).unwrap().is_empty());
    }

    /// A scratch-file fault converting a found PDF is an error, not "could
    /// not be read", and stores nothing (#11).
    #[test]
    fn found_pdf_scratch_fault_is_an_error() {
        let limit = Limit::new(Duration::from_secs(3600));
        let missing = std::env::temp_dir().join(format!("sa-missing-{}", uuid::Uuid::new_v4()));
        let mut c = open_memory_store().unwrap();
        let r = open_found_pdf(&mut c, &FakePdf, &limit, Some(&missing));
        assert!(
            matches!(r, Err(OpenError::Intake(IntakeError::Io(_)))),
            "{r:?}"
        );
        assert!(recent_papers(&mut c).unwrap().is_empty());
    }

    /// A found PDF the engine fails on is an error, not "could not be read",
    /// and stores nothing; a found PDF that is unreadable keeps the paper (#11).
    #[test]
    fn found_pdf_engine_fault_is_an_error() {
        let limit = Limit::new(Duration::from_secs(3600));
        let mut c = open_memory_store().unwrap();
        let engine = FnPdf(|_: &PdfLimit| Err(PdfError::engine("out of memory")));
        let r = open_found_pdf(&mut c, &engine, &limit, None);
        assert!(
            matches!(&r, Err(OpenError::Intake(IntakeError::Pdf(e))) if e.kind == PdfErrorKind::Engine),
            "{r:?}"
        );
        assert!(recent_papers(&mut c).unwrap().is_empty());
        let bad = FnPdf(|_: &PdfLimit| Err(PdfError::unreadable("not a PDF")));
        let o = open_found_pdf(&mut c, &bad, &limit, None).unwrap();
        let OpenOutcome::NeedsFullText { reason, .. } = o else {
            panic!("expected NeedsFullText")
        };
        assert!(reason.contains("could not be read"), "{reason}");
    }

    /// An identifier of no known format is refused, and no paper is stored.
    #[test]
    fn bad_identifier_creates_nothing() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let r = with_ctx(&mut c, &src, |x| open(x, ident("abc")));
        assert!(matches!(r, Err(OpenError::Identifier(_))));
        assert!(recent_papers(&mut c).unwrap().is_empty());
    }

    /// A file intake refuses (here binary) stores neither a paper nor a source.
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

    /// A write that fails midway (a trigger refuses the canonical text) rolls back
    /// the paper and source too.
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

    /// A fetched JATS that does not parse keeps the paper with its PMID and asks
    /// for the text, naming the source; no canonical text is stored.
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

    /// A paper looked up by PMID records its JATS's DOI, so a lookup by that DOI
    /// lands on it without a fetch.
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

    /// An uploaded JATS file records its own PMID and DOI, and uploading it again
    /// lands on the same paper.
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

    /// A re-derive reads the source its stale text came from, not the paper's
    /// newest source.
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
            SourcePlace::App,
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

    /// The same file opened again stores no second copy, but keeps the
    /// "this is the whole paper" given on the first (#27); once the stored
    /// text is an older converter's, the new text is stored on the same
    /// source and keeps it too, as a re-derive does (#43).
    #[test]
    fn the_same_file_again_is_stored_once() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let file = || OpenRequest::File {
            name: Some("article.xml".into()),
            bytes: JATS.to_vec(),
        };
        let id = ready_id(with_ctx(&mut c, &src, |x| open(x, file())).unwrap());
        let paper = PaperId(id.clone());
        let first = latest_canonical(&mut c, &paper).unwrap().unwrap();
        confirm_full_text(&mut c, &paper, first.id).unwrap();
        assert_eq!(
            ready_id(with_ctx(&mut c, &src, |x| open(x, file())).unwrap()),
            id
        );
        assert_eq!(count(&mut c, "sources"), 1);
        let kept = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(kept.id, first.id);
        assert!(kept.confirmed_full_text);
        age_canonical(&mut c, SourceKind::Jats);
        with_ctx(&mut c, &src, |x| open(x, file())).unwrap();
        assert_eq!(count(&mut c, "sources"), 1);
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_ne!(after.id, first.id);
        assert_eq!(after.source_id, first.source_id);
        assert_eq!(
            after.text.converter_version,
            current_converter_version(SourceKind::Jats)
        );
        assert!(after.confirmed_full_text);
    }

    /// After a converter bump, the same bytes under another name are another
    /// text (the name is a plain text's title), so they are stored anew and
    /// not confirmed, with a notice saying so; the paper is found by its
    /// bytes (#43, #44).
    #[test]
    fn a_renamed_file_after_a_converter_bump_is_stored_anew() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "draft.txt", TXT.as_bytes());
        let paper = PaperId(id.clone());
        let first = latest_canonical(&mut c, &paper).unwrap().unwrap();
        confirm_full_text(&mut c, &paper, first.id).unwrap();
        age_canonical(&mut c, SourceKind::PlainText);
        let renamed = with_ctx(&mut c, &src, |x| {
            open(
                x,
                OpenRequest::File {
                    name: Some("smith rct.txt".into()),
                    bytes: TXT.as_bytes().to_vec(),
                },
            )
        })
        .unwrap();
        assert_eq!(ready(renamed), (id, vec![REPLACED_CONFIRMED_NOTICE.into()]));
        assert_eq!(count(&mut c, "sources"), 2);
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_ne!(after.source_id, first.source_id);
        assert!(!after.confirmed_full_text);
        assert_eq!(paper_title(&mut c, &paper).unwrap(), "smith rct");
    }

    /// A document without identifiers opened again, as a file or a paste of
    /// the same bytes, lands on the same paper and stores no copy; other
    /// bytes make another paper (#44).
    #[test]
    fn the_same_bytes_without_identifiers_find_their_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "notes.md", MD.as_bytes());
        assert_eq!(open_file(&mut c, &src, "notes.md", MD.as_bytes()), id);
        assert_eq!(ready_id(paste(&mut c, &src, MD)), id);
        assert_eq!(count(&mut c, "papers"), 1);
        assert_eq!(count(&mut c, "sources"), 1);
        let other = format!("{MD}\nOne more line.\n");
        assert_ne!(open_file(&mut c, &src, "notes.md", other.as_bytes()), id);
        assert_eq!(count(&mut c, "papers"), 2);
        assert_eq!(count(&mut c, "sources"), 2);
    }

    /// A paste of a plain text stored as a file takes the file's name as its
    /// title hint, so it is the stored text: no copy, and the paper keeps its
    /// title and its "this is the whole paper" (#44).
    #[test]
    fn a_paste_of_a_stored_file_keeps_its_title_and_confirmation() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "draft.txt", TXT.as_bytes());
        let paper = PaperId(id.clone());
        let first = latest_canonical(&mut c, &paper).unwrap().unwrap();
        confirm_full_text(&mut c, &paper, first.id).unwrap();
        assert_eq!(ready(paste(&mut c, &src, TXT)), (id, vec![]));
        assert_eq!(count(&mut c, "sources"), 1);
        let kept = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(kept.id, first.id);
        assert!(kept.confirmed_full_text);
        assert_eq!(paper_title(&mut c, &paper).unwrap(), "draft");
    }

    /// A paste again after a converter bump stores the new text on the same
    /// source (neither has a file name), and an unconfirmed text stays
    /// unconfirmed (#43).
    #[test]
    fn a_paste_again_after_a_converter_bump_stays_unconfirmed() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = ready_id(paste(&mut c, &src, TXT));
        let paper = PaperId(id.clone());
        let first = latest_canonical(&mut c, &paper).unwrap().unwrap();
        age_canonical(&mut c, SourceKind::PlainText);
        assert_eq!(ready_id(paste(&mut c, &src, TXT)), id);
        assert_eq!(count(&mut c, "sources"), 1);
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_ne!(after.id, first.id);
        assert_eq!(after.source_id, first.source_id);
        assert_eq!(
            after.text.converter_version,
            current_converter_version(SourceKind::PlainText)
        );
        assert!(!after.confirmed_full_text);
    }

    /// A text attached to a paper with identifiers, opened later as a file,
    /// lands on that paper by its bytes and stores no copy (#44).
    #[test]
    fn an_attached_text_opened_again_finds_its_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Unavailable("none".into()))]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap();
        let OpenOutcome::NeedsFullText { paper_id, .. } = o else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        let file = || OpenRequest::File {
            name: Some("paper.txt".into()),
            bytes: TXT.as_bytes().to_vec(),
        };
        with_ctx(&mut c, &src, |x| attach(x, &paper_id, file())).unwrap();
        let again = with_ctx(&mut c, &src, |x| open(x, file())).unwrap();
        assert_eq!(ready(again), (paper_id, vec![]));
        assert_eq!(count(&mut c, "papers"), 1);
        assert_eq!(count(&mut c, "sources"), 1);
    }

    /// A changed file with the same identifiers is stored anew and shown; A,
    /// then B, then A again ends on A's text, since only the latest is
    /// compared (#27).
    #[test]
    fn a_changed_file_is_stored_anew() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let changed = String::from_utf8_lossy(JATS)
            .replace("invented for testing only", "corrected for testing only");
        let a = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(a.clone());
        let text_a = latest_canonical(&mut c, &paper).unwrap().unwrap().text;
        assert_eq!(
            open_file(&mut c, &src, "article.xml", changed.as_bytes()),
            a
        );
        assert_eq!(count(&mut c, "sources"), 2);
        let b = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_ne!(b.text, text_a);
        assert_eq!(
            source_bytes(&mut c, b.source_id).unwrap().unwrap(),
            changed.as_bytes()
        );
        open_file(&mut c, &src, "article.xml", JATS);
        assert_eq!(count(&mut c, "sources"), 3);
        assert_eq!(
            latest_canonical(&mut c, &paper).unwrap().unwrap().text,
            text_a
        );
    }

    /// The same plain text under another name is another text (the name is
    /// its title), so attaching it stores it, and the paper's title follows.
    #[test]
    fn a_renamed_text_file_is_stored_anew() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "draft.txt", TXT.as_bytes());
        with_ctx(&mut c, &src, |x| {
            attach(
                x,
                &id,
                OpenRequest::File {
                    name: Some("smith rct.txt".into()),
                    bytes: TXT.as_bytes().to_vec(),
                },
            )
        })
        .unwrap();
        assert_eq!(count(&mut c, "sources"), 2);
        assert_eq!(paper_title(&mut c, &PaperId(id)).unwrap(), "smith rct");
    }

    /// A repeat open shows the same notices as the first, and they replace,
    /// not add to, the stored ones a reopen shows.
    #[test]
    fn the_same_file_again_keeps_its_notices() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let bad = String::from_utf8_lossy(JATS).replace("10.1234/fixture.1", "not-a-doi");
        let file = || OpenRequest::File {
            name: Some("article.xml".into()),
            bytes: bad.clone().into_bytes(),
        };
        let (id, first) = ready(with_ctx(&mut c, &src, |x| open(x, file())).unwrap());
        assert_eq!(first.len(), 1, "{first:?}");
        let (_, again) = ready(with_ctx(&mut c, &src, |x| open(x, file())).unwrap());
        assert_eq!(again, first);
        assert_eq!(count(&mut c, "sources"), 1);
        assert_eq!(stored_notices(&mut c), first);
        let (_, reopened) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert_eq!(reopened, first);
    }

    /// A stored text that cannot be read does not block opening the document
    /// again: it is stored afresh, with a notice that is not stored with it.
    #[test]
    fn an_unreadable_stored_text_is_replaced() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        execute(
            &mut c,
            "UPDATE canonical_texts SET canonical_json = 'not json'",
            &[],
        )
        .unwrap();
        let (_, notices) = ready(
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
        assert!(
            notices.iter().any(|n| n.contains("could not be read")),
            "{notices:?}"
        );
        assert_eq!(count(&mut c, "sources"), 2);
        assert!(latest_canonical(&mut c, &PaperId(id)).unwrap().is_some());
        let latest = fetch_one(
            &mut c,
            "SELECT notices_json FROM sources ORDER BY id DESC LIMIT 1",
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(latest.get_str("notices_json").unwrap(), "[]");
    }

    /// A guessed encoding is stored with the source, flagged as a guess.
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

    /// A text re-derives when its converter is not the current one, nothing points
    /// into it, and no attempt under the current converter has failed.
    #[test]
    fn rederive_rule() {
        assert!(needs_rederive("jats-1", "jats-2", None, false));
        assert!(!needs_rederive("jats-1", "jats-1", None, false));
        assert!(!needs_rederive("jats-1", "jats-2", None, true));
        // A failure under the current version blocks; one under an older version does not.
        assert!(!needs_rederive("jats-1", "jats-2", Some("jats-2"), false));
        assert!(needs_rederive("jats-1", "jats-3", Some("jats-2"), false));
    }

    /// Reopening a paper whose text an older converter made stores a new text
    /// under the current one, without notices.
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
        let (_, notices) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        let after = latest_canonical(&mut c, &PaperId(id)).unwrap().unwrap();
        assert_ne!(after.id, before.id);
        assert_eq!(after.text.converter_version, current);
    }

    /// Starts a conversation on the paper's latest text and saves one turn in it;
    /// returns the conversation's id.
    fn add_turn(c: &mut Connection, paper: &PaperId) -> i64 {
        use crate::conversations::{create_conversation, save_turn, Mode, NewTurn, TurnStatus};
        let text = latest_canonical(c, paper).unwrap().unwrap();
        let conv = create_conversation(c, paper, text.id).unwrap();
        let turn = NewTurn {
            question: "Q?".into(),
            answer: "A.".into(),
            status: TurnStatus::Complete,
            error: None,
            model: "m".into(),
            mode: Mode::WholePaper,
            passages: vec![],
            quotes: vec![],
            input_tokens: 1,
            output_tokens: 1,
        };
        save_turn(c, conv, &turn).unwrap();
        conv
    }

    /// A text with a saved turn is not re-derived on reopen: its offsets stay.
    #[test]
    fn text_with_turns_is_not_rederived() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        age_canonical(&mut c, SourceKind::Jats);
        add_turn(&mut c, &paper);
        let before = latest_canonical(&mut c, &paper).unwrap().unwrap();
        let (_, notices) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(count(&mut c, "canonical_texts"), 1);
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(
            (after.id, after.text.converter_version.as_str()),
            (before.id, "old-0")
        );
    }

    /// The same file opened after a converter bump stores no new text while a
    /// conversation is on the stored one.
    #[test]
    fn same_file_after_bump_keeps_text() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        age_canonical(&mut c, SourceKind::Jats);
        add_turn(&mut c, &paper);
        assert_eq!(open_file(&mut c, &src, "article.xml", JATS), id);
        assert_eq!(count(&mut c, "canonical_texts"), 1);
        assert_eq!(count(&mut c, "sources"), 1);
    }

    /// Stores a PICO run on the paper's latest text: a forced extraction with
    /// one citation of the text's first character when `cited`, else a run
    /// that stopped at a failed check.
    fn add_pico_run(c: &mut Connection, paper: &PaperId, cited: bool) {
        use crate::pico_store::{save_run, NewPicoRun, PicoCheck, PicoOutcome};
        use bmlib::fulltext::canonical::TextSpan;
        use bmlib::quality::{
            PicoCitation, PicoExtraction, PicoPart, PicoPartState, PicoSuitability,
        };
        let text = latest_canonical(c, paper).unwrap().unwrap();
        let part = |citations: Vec<PicoCitation>| PicoPart {
            description: "d".into(),
            state: if citations.is_empty() {
                PicoPartState::NotReported
            } else {
                PicoPartState::Verified
            },
            confidence: None,
            citations,
            quotes_not_found: vec![],
            quotes_contradicted: vec![],
            quotes_dropped: 0,
        };
        let outcome = if cited {
            let first = text.text.span_text(TextSpan { start: 0, end: 1 }).unwrap();
            PicoOutcome::Extracted {
                check: None,
                extraction: PicoExtraction {
                    population: part(vec![PicoCitation {
                        start: 0,
                        end: 1,
                        text: first.to_string(),
                        similarity: 1.0,
                        exact: true,
                    }]),
                    intervention: part(vec![]),
                    comparison: part(vec![]),
                    outcome: part(vec![]),
                    study_type: None,
                    sample_size: None,
                    overall_confidence: None,
                    condensed_from_chars: None,
                    condensation_status: None,
                    notes: vec![],
                },
            }
        } else {
            PicoOutcome::NotSuitable(PicoCheck {
                suitability: PicoSuitability {
                    is_suitable: false,
                    is_intervention_study: None,
                    has_comparison: None,
                    study_type: None,
                    confidence: None,
                    rationale: None,
                    condensed_from_chars: None,
                    condensation_status: None,
                    notes: vec![],
                },
                from_abstract: true,
            })
        };
        let run = NewPicoRun {
            model: "m".into(),
            outcome,
        };
        save_run(c, paper, text.id, &run).unwrap();
    }

    /// A text with a PICO citation is not re-derived on reopen (the
    /// `reopen_paper` path): its spans stay.
    #[test]
    fn text_with_pico_citations_not_rederived() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        age_canonical(&mut c, SourceKind::Jats);
        add_pico_run(&mut c, &paper, true);
        let before = latest_canonical(&mut c, &paper).unwrap().unwrap();
        let (_, notices) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(count(&mut c, "canonical_texts"), 1);
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(
            (after.id, after.text.converter_version.as_str()),
            (before.id, "old-0")
        );
    }

    /// The same file opened after a converter bump (the `persist` path)
    /// stores no new text while a PICO run cites the stored one.
    #[test]
    fn same_file_after_bump_keeps_text_with_pico_run() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        age_canonical(&mut c, SourceKind::Jats);
        add_pico_run(&mut c, &paper, true);
        assert_eq!(open_file(&mut c, &src, "article.xml", JATS), id);
        assert_eq!(count(&mut c, "canonical_texts"), 1);
        assert_eq!(count(&mut c, "sources"), 1);
    }

    /// A PICO run that cites nothing does not keep its text: it re-derives.
    #[test]
    fn run_without_citations_is_no_dependent() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        age_canonical(&mut c, SourceKind::Jats);
        add_pico_run(&mut c, &paper, false);
        let before = latest_canonical(&mut c, &paper).unwrap().unwrap();
        ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        let after = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_ne!(after.id, before.id);
        assert_ne!(after.text.converter_version, "old-0");
    }

    /// A different document for the paper becomes its latest text, and the
    /// old conversation still names the text it was held on.
    #[test]
    fn new_document_keeps_old_conversation() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let changed = String::from_utf8_lossy(JATS)
            .replace("invented for testing only", "corrected for testing only");
        let id = open_file(&mut c, &src, "article.xml", JATS);
        let paper = PaperId(id.clone());
        let old = latest_canonical(&mut c, &paper).unwrap().unwrap();
        let conv = add_turn(&mut c, &paper);
        assert_eq!(
            open_file(&mut c, &src, "article.xml", changed.as_bytes()),
            id
        );
        let new = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_ne!(new.id, old.id);
        assert_eq!(
            crate::conversations::conversation_text(&mut c, conv).unwrap(),
            Some((paper, old.id))
        );
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

    /// Opens `text` as a paste.
    fn paste(c: &mut Connection, src: &FakeSource, text: &str) -> OpenOutcome {
        with_ctx(c, src, |x| {
            open(
                x,
                OpenRequest::Paste {
                    text: text.to_string(),
                },
            )
        })
        .unwrap()
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
        confirm_full_text(&mut c, &paper, before.id).unwrap();
        age_canonical(&mut c, SourceKind::Jats);
        let (_, notices) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
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
        execute(&mut c, "UPDATE source_bytes SET bytes = x'00'", &[]).unwrap();
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
        let failure = after.rederive_failure.expect("failure recorded");
        assert_eq!(failure.converter_version, current);
        // The notice gives the cause, not only the fact.
        assert!(notices[0].ends_with(&failure.error), "{notices:?}");
        // Even with a readable source now, the same converter is not tried again.
        execute(
            &mut c,
            "UPDATE source_bytes SET bytes = ?",
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

    /// A stale failure is replaced when the current converter fails too.
    #[test]
    fn new_failure_replaces_a_stale_one() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        age_canonical(&mut c, SourceKind::Jats);
        execute(
            &mut c,
            "UPDATE canonical_texts SET rederive_failed_version = 'old-1', rederive_error = 'e'",
            &[],
        )
        .unwrap();
        execute(&mut c, "UPDATE source_bytes SET bytes = x'00'", &[]).unwrap();
        let (_, notices) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert_eq!(notices.len(), 1);
        let failure = latest_canonical(&mut c, &PaperId(id))
            .unwrap()
            .unwrap()
            .rederive_failure
            .unwrap();
        assert_eq!(
            failure.converter_version,
            current_converter_version(SourceKind::Jats)
        );
        assert_ne!(failure.error, "e");
    }

    /// A PDF engine fault during a re-derive is shown but not recorded, so
    /// the next open tries again.
    #[test]
    fn engine_fault_is_not_recorded() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        age_canonical(&mut c, SourceKind::Jats);
        execute(&mut c, "UPDATE sources SET kind = 'pdf'", &[]).unwrap();
        let engine = FnPdf(|_: &PdfLimit| Err(PdfError::engine("out of memory")));
        let limit = Limit::new(Duration::from_secs(3600));
        let mut ctx = OpenContext {
            db: &mut c,
            source: &src,
            pdf: &engine,
            scratch_dir: &std::env::temp_dir(),
            attempts: 1,
            base_delay: Duration::ZERO,
            limit: &limit,
            keep_mismatched: false,
            library: &Configured::None,
        };
        let (_, notices) = ready(reopen(&mut ctx, &id).unwrap());
        assert!(notices[0].contains("engine failed"), "{notices:?}");
        let after = latest_canonical(&mut c, &PaperId(id)).unwrap().unwrap();
        assert_eq!(after.rederive_failure, None);
    }

    /// What MuPDF reads past (#599) is a notice on the open that stores the
    /// PDF and on every reopen; a re-derive stores its own with the source
    /// once, in place of an older reading's.
    #[test]
    fn engine_messages_are_a_stored_notice() {
        const PDF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pdf_warnings_cut_short.pdf"
        ));
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let pdf = bmlib::fulltext::mupdf_backend::MupdfExtractor::new();
        let file = OpenRequest::File {
            name: Some("damaged.pdf".into()),
            bytes: PDF.to_vec(),
        };
        let (id, first) = ready(with_pdf(&mut c, &src, &pdf, |x| open(x, file)).unwrap());
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(first[0].contains("ignoring rest of page"), "{first:?}");
        let (_, again) = ready(with_pdf(&mut c, &src, &pdf, |x| reopen(x, &id)).unwrap());
        assert_eq!(again, first);
        // A source stored before the engine's messages were read, re-derived.
        execute(&mut c, "UPDATE sources SET notices_json = '[]'", &[]).unwrap();
        age_canonical(&mut c, SourceKind::Pdf);
        let (_, rederived) = ready(with_pdf(&mut c, &src, &pdf, |x| reopen(x, &id)).unwrap());
        assert_eq!(rederived, first);
        let (_, later) = ready(with_pdf(&mut c, &src, &pdf, |x| reopen(x, &id)).unwrap());
        assert_eq!(later, first, "stored once, not again");
        // Re-derived again, the same messages are still one notice.
        age_canonical(&mut c, SourceKind::Pdf);
        let (_, same) = ready(with_pdf(&mut c, &src, &pdf, |x| reopen(x, &id)).unwrap());
        assert_eq!(same, first);
        assert_eq!(stored_notices(&mut c), first);
        // An engine notice from an older reading is replaced; others stay.
        let old = format!("{ENGINE_NOTICE_LEAD}: an older message.");
        set_notices(&mut c, &[old, "A note kept.".into()]);
        age_canonical(&mut c, SourceKind::Pdf);
        let (_, replaced) = ready(with_pdf(&mut c, &src, &pdf, |x| reopen(x, &id)).unwrap());
        let want = vec!["A note kept.".to_string(), first[0].clone()];
        assert_eq!(replaced, want);
        assert_eq!(stored_notices(&mut c), want);
    }

    /// A re-derive that reads cleanly removes the stored engine notice, which
    /// described a text no longer shown.
    #[test]
    fn a_clean_rederive_drops_the_engine_notice() {
        const PDF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/paper.pdf"
        ));
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let pdf = bmlib::fulltext::mupdf_backend::MupdfExtractor::new();
        let file = OpenRequest::File {
            name: Some("paper.pdf".into()),
            bytes: PDF.to_vec(),
        };
        let (id, first) = ready(with_pdf(&mut c, &src, &pdf, |x| open(x, file)).unwrap());
        assert!(first.is_empty(), "{first:?}");
        set_notices(&mut c, &[format!("{ENGINE_NOTICE_LEAD}: stale.")]);
        age_canonical(&mut c, SourceKind::Pdf);
        let (_, notices) = ready(with_pdf(&mut c, &src, &pdf, |x| reopen(x, &id)).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        assert!(stored_notices(&mut c).is_empty());
    }

    /// The notices stored with the only source.
    fn stored_notices(c: &mut Connection) -> Vec<String> {
        let row = fetch_one(c, "SELECT notices_json FROM sources", &[])
            .unwrap()
            .unwrap();
        serde_json::from_str(row.get_str("notices_json").unwrap()).unwrap()
    }

    /// Stores `notices` with every source.
    fn set_notices(c: &mut Connection, notices: &[String]) {
        let json = serde_json::to_string(notices).unwrap();
        execute(
            c,
            "UPDATE sources SET notices_json = ?",
            &[Value::from(json)],
        )
        .unwrap();
    }

    /// An unreadable PDF during a re-derive is recorded, so the next open
    /// does not run the engine on it again (#22).
    #[test]
    fn unreadable_pdf_is_recorded() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        age_canonical(&mut c, SourceKind::Jats);
        execute(&mut c, "UPDATE sources SET kind = 'pdf'", &[]).unwrap();
        let calls = std::cell::Cell::new(0);
        let bad = FnPdf(|_: &PdfLimit| {
            calls.set(calls.get() + 1);
            Err(PdfError::unreadable("not a PDF"))
        });
        let limit = Limit::new(Duration::from_secs(3600));
        let mut ctx = OpenContext {
            db: &mut c,
            source: &src,
            pdf: &bad,
            scratch_dir: &std::env::temp_dir(),
            attempts: 1,
            base_delay: Duration::ZERO,
            limit: &limit,
            keep_mismatched: false,
            library: &Configured::None,
        };
        reopen(&mut ctx, &id).unwrap();
        let (_, notices) = ready(reopen(&mut ctx, &id).unwrap());
        assert_eq!(calls.get(), 1);
        assert_eq!(notices.len(), 1, "{notices:?}");
        let failure = latest_canonical(&mut c, &PaperId(id))
            .unwrap()
            .unwrap()
            .rederive_failure
            .unwrap();
        assert_eq!(
            failure.converter_version,
            current_converter_version(SourceKind::Pdf)
        );
    }

    /// A scratch-file fault during a re-derive is shown but not recorded, so
    /// the next open tries again.
    #[test]
    fn scratch_fault_is_not_recorded() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "article.xml", JATS);
        age_canonical(&mut c, SourceKind::Jats);
        // A PDF source is converted through a scratch file, here in a missing folder.
        execute(&mut c, "UPDATE sources SET kind = 'pdf'", &[]).unwrap();
        let missing = std::env::temp_dir().join(format!("sa-missing-{}", uuid::Uuid::new_v4()));
        let limit = Limit::new(Duration::from_secs(3600));
        let mut ctx = OpenContext {
            db: &mut c,
            source: &src,
            pdf: &FakePdf,
            scratch_dir: &missing,
            attempts: 1,
            base_delay: Duration::ZERO,
            limit: &limit,
            keep_mismatched: false,
            library: &Configured::None,
        };
        let (_, notices) = ready(reopen(&mut ctx, &id).unwrap());
        assert_eq!(notices.len(), 1, "{notices:?}");
        let after = latest_canonical(&mut c, &PaperId(id)).unwrap().unwrap();
        assert_eq!(after.text.converter_version, "old-0");
        assert_eq!(after.rederive_failure, None);
    }

    /// Opening by identifier lands on the paper holding it, with a notice for
    /// a fetched identifier that another paper holds.
    #[test]
    fn fetched_clash_is_a_notice() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![jats_hit()]);
        let a = create_paper(&mut c, &parse_identifier("12345678").unwrap(), None).unwrap();
        create_paper(
            &mut c,
            &parse_identifier("10.1234/fixture.1").unwrap(),
            Some("Paper B"),
        )
        .unwrap();
        let (got, notices) = ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!(got, a.0);
        assert_eq!(
            notices,
            ["DOI 10.1234/fixture.1 was not recorded: it belongs to another stored paper, \u{201c}Paper B\u{201d}."]
        );
    }

    /// An attached JATS file's invalid identifier is dropped with a notice.
    #[test]
    fn attach_carries_the_documents_notices() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Unavailable("none".into()))]);
        let OpenOutcome::NeedsFullText { paper_id, .. } =
            with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap()
        else {
            panic!("expected NeedsFullText")
        };
        let bad = String::from_utf8_lossy(JATS).replace("10.1234/fixture.1", "not-a-doi");
        let (got, notices) = ready(
            with_ctx(&mut c, &src, |x| {
                attach(
                    x,
                    &paper_id,
                    OpenRequest::File {
                        name: Some("article.xml".into()),
                        bytes: bad.into_bytes(),
                    },
                )
            })
            .unwrap(),
        );
        assert_eq!(got, paper_id);
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("DOI"), "{notices:?}");
    }

    /// A JATS file whose PMID contradicts the stored paper its DOI names is
    /// refused, naming that paper, and nothing is stored.
    #[test]
    fn contradicting_file_is_refused() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let doi = Identifiers {
            pmid: Some("999".into()),
            ..parse_identifier("10.1234/fixture.1").unwrap()
        };
        create_paper(&mut c, &doi, Some("Paper A")).unwrap();
        let r = with_ctx(&mut c, &src, |x| {
            open(
                x,
                OpenRequest::File {
                    name: Some("article.xml".into()),
                    bytes: JATS.to_vec(),
                },
            )
        });
        match r {
            Err(e @ OpenError::Mismatch { .. }) => {
                let m = e.to_string();
                assert!(m.contains("\u{201c}Paper A\u{201d}"), "{m}");
                assert!(
                    m.contains("its PMID is 12345678, but this paper's PMID is 999"),
                    "{m}"
                );
                assert!(m.contains(KEEP_MISMATCHED_LABEL), "{m}");
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
        assert_eq!(count(&mut c, "papers"), 1);
        assert_eq!(count(&mut c, "sources"), 0);
    }

    /// With the debugging setting on, the contradicting file opens as a paper
    /// of its own, says which paper it contradicts, and leaves that paper alone.
    #[test]
    fn contradicting_file_kept_as_its_own_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let doi = Identifiers {
            pmid: Some("999".into()),
            ..parse_identifier("10.1234/fixture.1").unwrap()
        };
        let a = create_paper(&mut c, &doi, Some("Paper A")).unwrap();
        let (got, notices) = ready(
            with_keep(&mut c, &src, true, |x| {
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
        assert_ne!(got, a.0);
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert!(
            notices[0].contains("\u{201c}Paper A\u{201d}"),
            "{notices:?}"
        );
        assert!(
            notices[1].starts_with("DOI 10.1234/fixture.1 was not recorded"),
            "{notices:?}"
        );
        let own = recent_papers(&mut c).unwrap();
        let own = own.iter().find(|p| p.id == got).unwrap();
        assert_eq!(own.identifiers.pmid.as_deref(), Some("12345678"));
        assert!(latest_canonical(&mut c, &a).unwrap().is_none());
    }

    /// A lookup lands on the paper of the identifier typed, even when the
    /// fetched DOI belongs to another paper; the notice names that paper, is
    /// stored, and a repeated lookup reopens the same paper offline (#25).
    #[test]
    fn lookup_lands_on_the_typed_identifiers_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![jats_hit()]);
        let b = Identifiers {
            pmid: Some("2".into()),
            ..parse_identifier("10.1234/fixture.1").unwrap()
        };
        let b = create_paper(&mut c, &b, Some("Paper B")).unwrap();
        let (first, notices) =
            ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert_ne!(first, b.0);
        assert_eq!(
            notices,
            ["DOI 10.1234/fixture.1 was not recorded: it belongs to another stored paper, \u{201c}Paper B\u{201d}."]
        );
        let (again, stored) =
            ready(with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!((again, stored), (first, notices));
        assert_eq!(src.calls(), 1);
        assert!(latest_canonical(&mut c, &b).unwrap().is_none());
    }

    /// A fetched document whose DOI differs from the one typed is not stored:
    /// the paper needs full text and the reason says why. With the debugging
    /// setting on it opens as a paper of its own instead.
    #[test]
    fn fetched_mismatch_is_refused_or_kept() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![jats_hit(), jats_hit(), jats_hit()]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("10.1234/other"))).unwrap();
        let OpenOutcome::NeedsFullText {
            paper_id,
            reason,
            unreachable,
        } = o
        else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        assert!(!unreachable);
        assert!(
            reason.starts_with("From europepmc: This document does not match"),
            "{reason}"
        );
        assert!(
            reason.contains("its DOI is 10.1234/fixture.1, but this paper's DOI is 10.1234/other"),
            "{reason}"
        );
        assert_eq!(count(&mut c, "sources"), 0);
        // Reopened from Papers, it still says why.
        let again = with_ctx(&mut c, &src, |x| reopen(x, &paper_id)).unwrap();
        assert!(
            matches!(&again, OpenOutcome::NeedsFullText { reason: r, .. } if *r == reason),
            "{again:?}"
        );
        let (own, notices) =
            ready(with_keep(&mut c, &src, true, |x| open(x, ident("10.1234/other"))).unwrap());
        assert_ne!(own, paper_id);
        assert!(notices[0].contains("does not match"), "{notices:?}");
        assert!(notices[0].contains("was on in Settings"), "{notices:?}");
        // The paper typed says where the document went.
        let typed = with_ctx(&mut c, &src, |x| reopen(x, &paper_id)).unwrap();
        let OpenOutcome::NeedsFullText { reason, .. } = typed else {
            panic!("expected NeedsFullText, got {typed:?}")
        };
        assert!(reason.contains("opened as a paper of its own"), "{reason}");
        // Looked up again, it lands on that same paper of its own.
        let (again, _) =
            ready(with_keep(&mut c, &src, true, |x| open(x, ident("10.1234/other"))).unwrap());
        assert_eq!(again, own);
        assert_eq!(count(&mut c, "papers"), 2);
        assert!(latest_canonical(&mut c, &PaperId(paper_id))
            .unwrap()
            .is_none());
    }

    /// With the debugging setting on, a document matched by its PMID but
    /// contradicting on its DOI lands on the same paper of its own each time.
    #[test]
    fn kept_mismatch_matched_by_pmid_repeats_onto_the_same_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let a = Identifiers {
            pmid: Some("12345678".into()),
            ..parse_identifier("10.1234/other").unwrap()
        };
        let a = create_paper(&mut c, &a, Some("Paper A")).unwrap();
        let file = || OpenRequest::File {
            name: Some("article.xml".into()),
            bytes: JATS.to_vec(),
        };
        let first = ready_id(with_keep(&mut c, &src, true, |x| open(x, file())).unwrap());
        let again = ready_id(with_keep(&mut c, &src, true, |x| open(x, file())).unwrap());
        assert_eq!(again, first);
        assert_ne!(first, a.0);
        assert_eq!(count(&mut c, "papers"), 2);
        // The same document again stores no second source or text (#27).
        assert_eq!(count(&mut c, "sources"), 1);
        assert_eq!(count(&mut c, "canonical_texts"), 1);
        assert!(latest_canonical(&mut c, &a).unwrap().is_none());
    }

    /// The same by lookup: the PMID typed names a paper without text whose DOI
    /// differs; each lookup lands on one paper of its own.
    #[test]
    fn kept_lookup_matched_by_pmid_repeats_onto_the_same_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![jats_hit(), jats_hit()]);
        let a = Identifiers {
            pmid: Some("12345678".into()),
            ..parse_identifier("10.1234/other").unwrap()
        };
        let a = create_paper(&mut c, &a, Some("Paper A")).unwrap();
        let first =
            ready_id(with_keep(&mut c, &src, true, |x| open(x, ident("12345678"))).unwrap());
        let again =
            ready_id(with_keep(&mut c, &src, true, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!(again, first);
        assert_eq!(count(&mut c, "papers"), 2);
        assert_eq!(count(&mut c, "sources"), 1);
        assert_eq!(count(&mut c, "canonical_texts"), 1);
        assert!(latest_canonical(&mut c, &a).unwrap().is_none());
    }

    /// A kept document does not land on another stored paper its PMID names
    /// when it contradicts that one too: it gets a new paper instead.
    #[test]
    fn kept_lookup_skips_a_contradicted_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![jats_hit()]);
        let other = Identifiers {
            pmid: Some("12345678".into()),
            ..parse_identifier("10.9999/c").unwrap()
        };
        let other = create_paper(&mut c, &other, Some("Paper C")).unwrap();
        let got =
            ready_id(with_keep(&mut c, &src, true, |x| open(x, ident("10.1234/other"))).unwrap());
        assert_ne!(got, other.0);
        assert_eq!(count(&mut c, "papers"), 3);
        assert!(latest_canonical(&mut c, &other).unwrap().is_none());
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
            Some("Paper B"),
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
            ["DOI 10.1234/fixture.1 was not recorded: it belongs to another stored paper, \u{201c}Paper B\u{201d}."]
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
            matches!(r, Err(OpenError::Intake(IntakeError::Empty(None)))),
            "{r:?}"
        );
        assert_eq!(count(&mut c, "papers"), 0);
    }

    /// A reopen or attach of an unknown paper id is `UnknownPaper`, and an
    /// identifier cannot be attached as a document.
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

    /// A journal version, for the record lookup's notes.
    fn journal_version() -> PublishedVersion {
        PublishedVersion {
            pmid: "9".into(),
            doi: "10.1/j".into(),
            pmc_id: None,
        }
    }

    /// A record's title names the paper but is no title hint for the
    /// converter: a re-derive has only the file name, none for a lookup, so
    /// a hint would change the text and every offset into it.
    #[test]
    fn a_record_title_is_no_title_hint() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let (with, without) = with_ctx(&mut c, &src, |x| {
            let conv = |t| {
                convert(
                    x,
                    SourceKind::PlainText,
                    "europepmc",
                    None,
                    t,
                    TXT.as_bytes().to_vec(),
                    &PdfLimit::default(),
                )
                .unwrap()
            };
            (conv(Some("A record title")), conv(None))
        });
        assert_eq!(with.text, without.text);
        assert_eq!(without.text.title_span, None);
        assert_eq!(with.title, "A record title");
    }

    /// The record lookup's notes are notices of the document found.
    #[test]
    fn record_notes_are_notices_of_the_document() {
        let mut c = open_memory_store().unwrap();
        let mut src = FakeSource::new(vec![jats_hit()]);
        src.lookup.published_version = Some(journal_version());
        let (_, notices) =
            ready(with_ctx(&mut c, &src, |x| open(x, ident("10.1234/fixture.1"))).unwrap());
        assert!(
            notices.iter().any(|n| n.starts_with("This is a preprint.")),
            "{notices:?}"
        );
    }

    /// A document kept apart does not take the notes, which are about the
    /// paper typed; that paper keeps them with its note of where it went.
    #[test]
    fn record_notes_stay_with_the_paper_typed() {
        let mut c = open_memory_store().unwrap();
        let mut src = FakeSource::new(vec![jats_hit()]);
        src.lookup.published_version = Some(journal_version());
        let (own, notices) =
            ready(with_keep(&mut c, &src, true, |x| open(x, ident("10.1234/other"))).unwrap());
        assert!(
            !notices.iter().any(|n| n.contains("preprint")),
            "{notices:?}"
        );
        let typed = recent_papers(&mut c)
            .unwrap()
            .into_iter()
            .find(|p| p.id != own)
            .unwrap();
        let o = with_ctx(&mut c, &src, |x| reopen(x, &typed.id)).unwrap();
        let OpenOutcome::NeedsFullText { reason, .. } = o else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        assert!(reason.contains("opened as a paper of its own"), "{reason}");
        assert!(reason.contains("This is a preprint."), "{reason}");
    }

    /// A paper an earlier lookup left untitled takes the title of a record
    /// found later.
    #[test]
    fn an_untitled_paper_takes_a_later_record_title() {
        let mut c = open_memory_store().unwrap();
        let mut src = FakeSource::new(vec![Err(FullTextError::Unavailable("none.".into()))]);
        with_ctx(&mut c, &src, |x| open(x, ident("PPR123"))).unwrap();
        assert_eq!(recent_papers(&mut c).unwrap()[0].title, UNTITLED_PAPER);
        let mut record = FetchedRecord::new("A preprint", "europepmc");
        record.doi = Some("10.1101/p".into());
        src.lookup.record = Some(record);
        with_ctx(&mut c, &src, |x| open(x, ident("PPR123"))).unwrap();
        assert_eq!(recent_papers(&mut c).unwrap()[0].title, "A preprint");
    }

    /// A hand-off from "MFC" claiming `ids`, carrying `document` (if any).
    fn handoff(ids: Identifiers, document: Option<HandedDocument>) -> OpenRequest {
        OpenRequest::Handoff(Handoff {
            sender: Some("MFC".into()),
            title: None,
            identifiers: ids,
            notices: Vec::new(),
            document,
        })
    }

    /// A handed document of `kind`, named `file_name`, from Europe PMC.
    fn handed(kind: SourceKind, file_name: Option<&str>, bytes: &[u8]) -> HandedDocument {
        HandedDocument {
            kind,
            file_name: file_name.map(str::to_string),
            source: Some("Europe PMC".into()),
            bytes: bytes.to_vec(),
        }
    }

    /// Identifiers holding only this PMID.
    fn pmid(p: &str) -> Identifiers {
        Identifiers {
            pmid: Some(p.into()),
            ..Default::default()
        }
    }

    /// A hand-off's JATS opens ready on a paper holding the hand-off's
    /// identifiers and the JATS's own, its origin naming sender and source.
    #[test]
    fn handoff_with_jats_opens() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let ids = Identifiers {
            pmc_id: Some("PMC1".into()),
            ..Default::default()
        };
        let doc = handed(SourceKind::Jats, Some("article.xml"), JATS);
        let (_, notices) =
            ready(with_ctx(&mut c, &src, |x| open(x, handoff(ids, Some(doc)))).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        let held = recent_papers(&mut c).unwrap()[0].identifiers.clone();
        assert_eq!(held.pmid.as_deref(), Some("12345678"));
        assert_eq!(held.doi.as_deref(), Some("10.1234/fixture.1"));
        assert_eq!(held.pmc_id.as_deref(), Some("PMC1"));
        let row = fetch_one(&mut c, "SELECT origin FROM sources", &[])
            .unwrap()
            .unwrap();
        assert_eq!(row.get_str("origin").unwrap(), "MFC, from Europe PMC");
        assert_eq!(src.calls(), 0);
    }

    /// A plain text without a title of its own takes the hand-off's.
    #[test]
    fn handoff_title_names_untitled_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let mut req = handoff(
            pmid("42"),
            Some(handed(SourceKind::PlainText, None, TXT.as_bytes())),
        );
        if let OpenRequest::Handoff(h) = &mut req {
            h.title = Some("Handed title".into());
        }
        let id = ready_id(with_ctx(&mut c, &src, |x| open(x, req)).unwrap());
        assert_eq!(paper_title(&mut c, &PaperId(id)).unwrap(), "Handed title");
    }

    /// The same hand-off twice lands on the same paper with one source.
    #[test]
    fn same_handoff_twice_one_copy() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let req = || {
            handoff(
                pmid("12345678"),
                Some(handed(SourceKind::Jats, Some("article.xml"), JATS)),
            )
        };
        let a = ready_id(with_ctx(&mut c, &src, |x| open(x, req())).unwrap());
        let b = ready_id(with_ctx(&mut c, &src, |x| open(x, req())).unwrap());
        assert_eq!(a, b);
        assert_eq!(count(&mut c, "papers"), 1);
        assert_eq!(count(&mut c, "sources"), 1);
    }

    /// A paper opened by PMID without full text takes the text of a later
    /// hand-off naming that PMID (spec §8).
    #[test]
    fn handoff_reopens_paper_by_id() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Unavailable("none".into()))]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("777"))).unwrap();
        let OpenOutcome::NeedsFullText { paper_id, .. } = o else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        let doc = handed(SourceKind::PlainText, Some("paper.txt"), TXT.as_bytes());
        let got =
            ready_id(with_ctx(&mut c, &src, |x| open(x, handoff(pmid("777"), Some(doc)))).unwrap());
        assert_eq!(got, paper_id);
        assert_eq!(count(&mut c, "papers"), 1);
        assert!(latest_canonical(&mut c, &PaperId(got)).unwrap().is_some());
    }

    /// An identifiers-only hand-off for a paper that has text reopens it
    /// without a lookup; the hand-off's notices are shown on that open only,
    /// never stored with the text (#67).
    #[test]
    fn ids_only_handoff_reopens_paper_with_text() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let doc = handed(SourceKind::Jats, Some("article.xml"), JATS);
        let first = ready_id(
            with_ctx(&mut c, &src, |x| {
                open(x, handoff(pmid("12345678"), Some(doc)))
            })
            .unwrap(),
        );
        let dropped = "A dropped identifier.".to_string();
        let mut req = handoff(pmid("12345678"), None);
        if let OpenRequest::Handoff(h) = &mut req {
            h.notices = vec![dropped.clone()];
        }
        let (id, notices) = ready(with_ctx(&mut c, &src, |x| open(x, req)).unwrap());
        assert_eq!(id, first);
        assert!(notices.contains(&dropped), "{notices:?}");
        assert_eq!(src.calls(), 0);
        let (_, again) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert!(!again.contains(&dropped), "{again:?}");
    }

    /// A hand-off whose JATS names another PMID than the one sent with it is
    /// refused, naming the sender and both values, keeping the hand-off's
    /// notices (#68), and nothing is stored.
    #[test]
    fn handoff_text_contradicts_its_ids() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let doc = handed(SourceKind::Jats, Some("article.xml"), JATS);
        let mut req = handoff(pmid("1"), Some(doc));
        if let OpenRequest::Handoff(h) = &mut req {
            h.notices = vec!["Invalid DOI dropped.".into()];
        }
        let r = with_ctx(&mut c, &src, |x| open(x, req));
        match r {
            Err(e @ OpenError::HandoffMismatch { .. }) => {
                let m = e.to_string();
                assert!(m.starts_with("The document MFC handed over is a different paper than the identifiers sent with it ("), "{m}");
                assert!(
                    m.contains("12345678") && m.contains("PMID") && m.contains(" 1)"),
                    "{m}"
                );
                assert!(
                    m.ends_with("so it was not stored. Invalid DOI dropped."),
                    "{m}"
                );
            }
            other => panic!("expected HandoffMismatch, got {other:?}"),
        }
        assert_eq!(count(&mut c, "papers"), 0);
        assert_eq!(count(&mut c, "sources"), 0);
    }

    /// A contradicted hand-off's message ends with the hand-off's notices,
    /// then the document's own (here its invalid PMID) (#68).
    #[test]
    fn handoff_mismatch_keeps_the_documents_notices() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let bad = String::from_utf8_lossy(JATS).replace("12345678", "not-a-pmid");
        let doc = handed(SourceKind::Jats, Some("article.xml"), bad.as_bytes());
        let ids = Identifiers {
            doi: Some("10.1/other".into()),
            ..Default::default()
        };
        let mut req = handoff(ids, Some(doc));
        if let OpenRequest::Handoff(h) = &mut req {
            h.notices = vec!["Invalid PMCID dropped.".into()];
        }
        let m = with_ctx(&mut c, &src, |x| open(x, req))
            .unwrap_err()
            .to_string();
        assert!(
            m.ends_with(" Invalid PMCID dropped. The document's own PMID was not recorded: \"not-a-pmid\" is not a valid PMID."),
            "{m}"
        );
    }

    /// A hand-off contradicting the stored paper it matches is a `Mismatch`,
    /// as a file's is, and nothing is stored.
    #[test]
    fn handoff_contradicts_stored_paper() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let stored = Identifiers {
            pmid: Some("999".into()),
            ..parse_identifier("10.1234/fixture.1").unwrap()
        };
        create_paper(&mut c, &stored, Some("Paper A")).unwrap();
        let claim = parse_identifier("10.1234/fixture.1").unwrap();
        let doc = handed(SourceKind::Jats, Some("article.xml"), JATS);
        let r = with_ctx(&mut c, &src, |x| open(x, handoff(claim, Some(doc))));
        assert!(matches!(r, Err(OpenError::Mismatch { .. })), "{r:?}");
        assert_eq!(count(&mut c, "papers"), 1);
        assert_eq!(count(&mut c, "sources"), 0);
    }

    /// A dropped identifier's notice is shown on open, and again on a reopen.
    #[test]
    fn handoff_notices_shown() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let mut req = handoff(
            pmid("42"),
            Some(handed(SourceKind::PlainText, Some("p.txt"), TXT.as_bytes())),
        );
        let dropped = "The DOI sent was not valid and was dropped.".to_string();
        if let OpenRequest::Handoff(h) = &mut req {
            h.notices = vec![dropped.clone()];
        }
        let (id, notices) = ready(with_ctx(&mut c, &src, |x| open(x, req)).unwrap());
        assert_eq!(notices, vec![dropped.clone()]);
        let (_, again) = ready(with_ctx(&mut c, &src, |x| reopen(x, &id)).unwrap());
        assert_eq!(again, vec![dropped]);
    }

    /// A hand-off without full text is a lookup by all its identifiers; the
    /// record-less paper takes the hand-off's title, and its notices end the
    /// reason it needs full text, kept for a reopen.
    #[test]
    fn ids_only_handoff_is_lookup() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![Err(FullTextError::Unavailable("none.".into()))]);
        let ids = Identifiers {
            pmid: Some("42".into()),
            doi: Some("10.1/x".into()),
            pmc_id: Some("PMC2".into()),
        };
        let dropped = "A dropped identifier.".to_string();
        let req = OpenRequest::Handoff(Handoff {
            sender: Some("MFC".into()),
            title: Some("Handed title".into()),
            identifiers: ids.clone(),
            notices: vec![dropped.clone()],
            document: None,
        });
        let o = with_ctx(&mut c, &src, |x| open(x, req)).unwrap();
        let OpenOutcome::NeedsFullText {
            paper_id, reason, ..
        } = o
        else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        assert!(reason.ends_with(&dropped), "{reason}");
        assert_eq!(*src.looked_up.borrow(), vec![ids]);
        assert_eq!(src.calls(), 1);
        assert_eq!(recent_papers(&mut c).unwrap()[0].title, "Handed title");
        let again = with_ctx(&mut c, &src, |x| reopen(x, &paper_id)).unwrap();
        let OpenOutcome::NeedsFullText { reason, .. } = again else {
            panic!("expected NeedsFullText, got {again:?}")
        };
        assert!(reason.ends_with(&dropped), "{reason}");
    }

    /// A hand-off's `pdf` that is not one is `NotPdf`, naming its file name,
    /// else the document its sender handed over; nothing is stored.
    #[test]
    fn handoff_pdf_that_is_not_pdf() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        for (name, shown) in [
            (Some("paper.pdf"), "paper.pdf"),
            (None, "the document MFC handed over"),
        ] {
            let doc = handed(SourceKind::Pdf, name, b"<html>not a pdf</html>");
            let r = with_ctx(&mut c, &src, |x| open(x, handoff(pmid("42"), Some(doc))));
            assert!(
                matches!(&r, Err(OpenError::Intake(IntakeError::NotPdf(n))) if n == shown),
                "{r:?}"
            );
        }
        assert_eq!(count(&mut c, "papers"), 0);
    }

    /// A hand-off without identifiers or file name whose bytes a stored file
    /// holds is read under that file's name, as a paste is: no second copy,
    /// and its notices still shown.
    #[test]
    fn nameless_handoff_of_a_stored_file_is_stored_once() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let id = open_file(&mut c, &src, "draft.txt", TXT.as_bytes());
        let mut req = handoff(
            Identifiers::default(),
            Some(handed(SourceKind::PlainText, None, TXT.as_bytes())),
        );
        if let OpenRequest::Handoff(h) = &mut req {
            h.notices = vec!["A dropped identifier.".into()];
        }
        let o = with_ctx(&mut c, &src, |x| open(x, req)).unwrap();
        assert_eq!(ready(o), (id, vec!["A dropped identifier.".into()]));
        assert_eq!(count(&mut c, "sources"), 1);
    }

    /// An identifiers-only hand-off for a paper found by its PMID, still
    /// without text, records its other identifiers on it, so a later
    /// hand-off by DOI alone finds the same paper (spec §8).
    #[test]
    fn handoff_keeps_its_other_identifiers() {
        let mut c = open_memory_store().unwrap();
        let none = || Err(FullTextError::Unavailable("none".into()));
        let src = FakeSource::new(vec![none(), none(), none()]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("42"))).unwrap();
        let OpenOutcome::NeedsFullText { paper_id, .. } = o else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        let both = Identifiers {
            doi: Some("10.1/x".into()),
            pmc_id: Some("PMC7".into()),
            ..pmid("42")
        };
        with_ctx(&mut c, &src, |x| open(x, handoff(both, None))).unwrap();
        let held = recent_papers(&mut c).unwrap()[0].identifiers.clone();
        assert_eq!(held.doi.as_deref(), Some("10.1/x"));
        assert_eq!(held.pmc_id.as_deref(), Some("PMC7"));
        let doi = Identifiers {
            doi: Some("10.1/x".into()),
            ..Default::default()
        };
        let o = with_ctx(&mut c, &src, |x| open(x, handoff(doi, None))).unwrap();
        assert!(
            matches!(&o, OpenOutcome::NeedsFullText { paper_id: p, .. } if *p == paper_id),
            "{o:?}"
        );
        assert_eq!(count(&mut c, "papers"), 1);
    }

    /// A hand-off's identifier another paper holds is not recorded on the
    /// paper it finds, and the user is told: in the reason it needs full
    /// text, or with the document found.
    #[test]
    fn handoff_id_held_by_another_paper_is_noticed() {
        let mut c = open_memory_store().unwrap();
        let none = || Err(FullTextError::Unavailable("none".into()));
        let src = FakeSource::new(vec![none(), none(), jats_hit()]);
        let o = with_ctx(&mut c, &src, |x| open(x, ident("12345678"))).unwrap();
        let OpenOutcome::NeedsFullText { paper_id, .. } = o else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        let held_by_b = Identifiers {
            doi: Some("10.1/b".into()),
            ..Default::default()
        };
        create_paper(&mut c, &held_by_b, Some("Paper B")).unwrap();
        let claim = || Identifiers {
            doi: Some("10.1/b".into()),
            ..pmid("12345678")
        };
        let notice =
            "DOI 10.1/b was not recorded: it belongs to another stored paper, \u{201c}Paper B\u{201d}.";
        let o = with_ctx(&mut c, &src, |x| open(x, handoff(claim(), None))).unwrap();
        let OpenOutcome::NeedsFullText { reason, .. } = &o else {
            panic!("expected NeedsFullText, got {o:?}")
        };
        assert!(reason.contains(notice), "{reason}");
        let (got, notices) =
            ready(with_ctx(&mut c, &src, |x| open(x, handoff(claim(), None))).unwrap());
        assert_eq!(got, paper_id);
        assert!(notices.iter().any(|n| n == notice), "{notices:?}");
    }

    /// A hand-off cannot be attached to a paper, as an identifier cannot.
    #[test]
    fn attach_refuses_handoff() {
        let mut c = open_memory_store().unwrap();
        let src = FakeSource::new(vec![]);
        let paper = create_paper(&mut c, &pmid("42"), None).unwrap();
        let doc = handed(SourceKind::PlainText, Some("p.txt"), TXT.as_bytes());
        let r = with_ctx(&mut c, &src, |x| {
            attach(x, &paper.0, handoff(pmid("42"), Some(doc)))
        });
        assert!(
            matches!(r, Err(OpenError::Intake(IntakeError::Unsupported(_)))),
            "{r:?}"
        );
        assert_eq!(count(&mut c, "sources"), 0);
    }

    /// [`with_ctx`] with `library` as the shared library.
    fn with_lib<T>(
        conn: &mut Connection,
        source: &FakeSource,
        library: &Configured,
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
            keep_mismatched: false,
            library,
        };
        f(&mut ctx)
    }

    /// Names `folder` as `c`'s library and opens it.
    fn library_at(c: &mut Connection, folder: &Path) -> Configured {
        crate::store::set_setting(c, crate::config::LIBRARY_PATH_KEY, folder.to_str().unwrap())
            .unwrap();
        crate::library::configured(c, "StudyAnalyzer on a test").unwrap()
    }

    /// A new, empty folder under the temp dir.
    fn library_folder() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sa-openlib-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The library file holding `sha256`, if any.
    fn library_file(lib: &Configured, sha256: &str) -> Option<bmlib::library::FileRef> {
        let Configured::Ready(at) = lib else {
            panic!("no usable library: {lib:?}")
        };
        bmlib::library::find_digest(&at.library.scan().unwrap(), sha256, None)
    }

    /// A file opened with a library set goes into it: the database keeps its
    /// digest and no bytes, and opening it again stores no second copy.
    #[test]
    fn a_file_opened_with_a_library_goes_into_it() {
        let mut c = open_memory_store().unwrap();
        let dir = library_folder();
        let lib = library_at(&mut c, &dir);
        let src = FakeSource::new(vec![]);
        let file = || OpenRequest::File {
            name: Some("article.xml".into()),
            bytes: JATS.to_vec(),
        };
        let (id, notices) = ready(with_lib(&mut c, &src, &lib, |x| open(x, file())).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(count(&mut c, "source_bytes"), 0);
        let s = latest_source(&mut c, &PaperId(id.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(s.place, SourcePlace::Library);
        assert!(library_file(&lib, &s.sha256).is_some());
        let again = ready_id(with_lib(&mut c, &src, &lib, |x| open(x, file())).unwrap());
        assert_eq!(again, id);
        assert_eq!(count(&mut c, "sources"), 1);
        assert_eq!(count(&mut c, "source_bytes"), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    /// A file the library has lost (the manual clear, a deletion by hand) is
    /// put back when the same file is opened again, still one source.
    #[test]
    fn reopening_a_file_the_library_lost_puts_it_back() {
        let mut c = open_memory_store().unwrap();
        let dir = library_folder();
        let lib = library_at(&mut c, &dir);
        let src = FakeSource::new(vec![]);
        let file = || OpenRequest::File {
            name: Some("article.xml".into()),
            bytes: JATS.to_vec(),
        };
        let id = ready_id(with_lib(&mut c, &src, &lib, |x| open(x, file())).unwrap());
        let s = latest_source(&mut c, &PaperId(id.clone()))
            .unwrap()
            .unwrap();
        let Configured::Ready(at) = &lib else {
            panic!("no usable library")
        };
        at.library
            .remove_file(&library_file(&lib, &s.sha256).unwrap())
            .unwrap();
        assert!(library_file(&lib, &s.sha256).is_none());
        let (again, notices) = ready(with_lib(&mut c, &src, &lib, |x| open(x, file())).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(again, id);
        assert_eq!(count(&mut c, "sources"), 1);
        assert_eq!(count(&mut c, "source_bytes"), 0);
        assert_eq!(
            source_by_id(&mut c, s.id).unwrap().place,
            SourcePlace::Library
        );
        assert!(library_file(&lib, &s.sha256).is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    /// With the library out of reach, a paste opens now, waiting, and says
    /// so; once the folder is back, the next open moves it in.
    #[test]
    fn an_unreachable_library_opens_now_and_moves_in_later() {
        let mut c = open_memory_store().unwrap();
        let dir = library_folder().join("drive");
        let gone = library_at(&mut c, &dir);
        assert!(matches!(gone, Configured::Unreachable(_)));
        let src = FakeSource::new(vec![]);
        let paste = OpenRequest::Paste {
            text: "Para one.\n\nPara two.".into(),
        };
        let (id, notices) = ready(with_lib(&mut c, &src, &gone, |x| open(x, paste)).unwrap());
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(
            notices[0].contains("kept in the app for now"),
            "{notices:?}"
        );
        assert!(notices[0].contains("cannot be used"), "{notices:?}");
        assert_eq!(count(&mut c, "source_bytes"), 1);
        std::fs::create_dir_all(&dir).unwrap();
        let back = library_at(&mut c, &dir);
        let (_, notices) = ready(with_lib(&mut c, &src, &back, |x| reopen(x, &id)).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(count(&mut c, "source_bytes"), 0);
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    /// A re-derive reads its source from the library; with the file gone it
    /// keeps the older text, says why, and records nothing (it may pass).
    #[test]
    fn a_rederive_reads_the_library() {
        let mut c = open_memory_store().unwrap();
        let dir = library_folder();
        let lib = library_at(&mut c, &dir);
        let src = FakeSource::new(vec![]);
        let upload = OpenRequest::File {
            name: Some("notes.txt".into()),
            bytes: b"Para one.\n\nPara two.".to_vec(),
        };
        let id = ready_id(with_lib(&mut c, &src, &lib, |x| open(x, upload)).unwrap());
        let paper = PaperId(id.clone());
        age_canonical(&mut c, SourceKind::PlainText);
        let (_, notices) = ready(with_lib(&mut c, &src, &lib, |x| reopen(x, &id)).unwrap());
        assert!(notices.is_empty(), "{notices:?}");
        let text = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(
            text.text.converter_version,
            current_converter_version(SourceKind::PlainText)
        );
        age_canonical(&mut c, SourceKind::PlainText);
        let s = latest_source(&mut c, &paper).unwrap().unwrap();
        let f = library_file(&lib, &s.sha256).unwrap();
        std::fs::remove_file(dir.join("papers").join(&f.folder).join(&f.name)).unwrap();
        let (_, notices) = ready(with_lib(&mut c, &src, &lib, |x| reopen(x, &id)).unwrap());
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(
            notices[0].contains("no longer in the paper library"),
            "{notices:?}"
        );
        let text = latest_canonical(&mut c, &paper).unwrap().unwrap();
        assert_eq!(text.text.converter_version, "old-0");
        assert_eq!(text.rederive_failure, None);
        std::fs::remove_dir_all(dir).ok();
    }

    /// A lookup with a library set keeps its JATS and the PDF found beside it
    /// there; the Original tab's PDF is a library file.
    #[test]
    fn a_lookup_with_a_library_keeps_its_files_there() {
        let mut c = open_memory_store().unwrap();
        let dir = library_folder();
        let lib = library_at(&mut c, &dir);
        let pdf_path = dir.with_extension("pdf");
        std::fs::write(&pdf_path, b"%PDF-1.4 beside").unwrap();
        let src = FakeSource::new(vec![Ok(FullTextResult {
            file_path: Some(pdf_path.to_string_lossy().into_owned()),
            ..jats_hit().unwrap()
        })]);
        let id = ready_id(with_lib(&mut c, &src, &lib, |x| open(x, ident("12345678"))).unwrap());
        assert_eq!(count(&mut c, "sources"), 2);
        assert_eq!(count(&mut c, "source_bytes"), 0);
        let pdf = crate::store::original_pdf_source(&mut c, &PaperId(id))
            .unwrap()
            .unwrap();
        assert_eq!(pdf.place, SourcePlace::Library);
        let bytes = crate::library::read_source(&mut c, &lib, &pdf).unwrap();
        assert_eq!(bytes, b"%PDF-1.4 beside");
        std::fs::remove_file(pdf_path).ok();
        std::fs::remove_dir_all(dir).ok();
    }

    /// A paste with no identifiers finds the paper that stored the same bytes,
    /// though they are kept only in the library.
    #[test]
    fn a_paste_finds_its_paper_by_digest_in_the_library() {
        let mut c = open_memory_store().unwrap();
        let dir = library_folder();
        let lib = library_at(&mut c, &dir);
        let src = FakeSource::new(vec![]);
        let paste = || OpenRequest::Paste {
            text: "Para one.\n\nPara two.".into(),
        };
        let a = ready_id(with_lib(&mut c, &src, &lib, |x| open(x, paste())).unwrap());
        assert_eq!(count(&mut c, "source_bytes"), 0);
        let b = ready_id(with_lib(&mut c, &src, &lib, |x| open(x, paste())).unwrap());
        assert_eq!(a, b);
        assert_eq!(count(&mut c, "papers"), 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
