//! Fetching a paper's free full text by identifier, through bmlib.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bmlib::fulltext::cache::FullTextCache;
use bmlib::fulltext::models::{ContentKind, FullTextResult, FullTextSourceEntry};
use bmlib::fulltext::service::{
    FullTextError, FullTextRequest, FullTextService, LogLevel, LogLine,
};
use bmlib::http::UreqClient;
use bmlib::publications::fetchers::registry::HttpClient;
use bmlib::publications::lookup::{
    lookup_record, LookupError, LookupRequest, PublishedVersion, RecordLookup, SourceOutcome,
};
use bmlib::publications::FetchedRecord;

use crate::config::{
    BMLIB_EXHAUSTED_PREFIX, BMLIB_FAULT_MARKER, BMLIB_NOT_FOUND_PREFIX, BMLIB_NO_IDENTIFIERS,
    HTTP_RETRYABLE_CLIENT_ERRORS, HTTP_SERVER_ERROR_MIN, PDF_MAGIC, PREPRINT_PREFIX,
};
use crate::identifiers::Identifiers;
use crate::intake::SourceKind;
use crate::limit::{Limit, LimitedClient, Stop};
use crate::retry::with_backoff;

/// Anything that can look a paper's record and full text up ([`Sources`], or a
/// fake in tests).
pub trait FullTextSource {
    /// Looks the paper's record up once by `ids` (bmlib's `lookup_record`); no
    /// retrying here.
    fn lookup(&self, ids: &Identifiers) -> Result<RecordLookup, LookupError>;
    /// Looks the paper's full text up once; no retrying here.
    fn fetch(&self, request: &FullTextRequest) -> Result<FullTextResult, FullTextError>;
}

/// bmlib's full-text service and record lookup, sharing one HTTP client.
pub struct Sources {
    /// The full-text chain.
    service: FullTextService,
    /// The client `service` was built with, for the record lookup.
    http: Arc<dyn HttpClient + Send + Sync>,
}

impl Sources {
    /// Both over `http`, the service caching in `cache_dir` and extracting no
    /// PDF text, and both sending `email` as the contact address.
    fn new(http: Arc<dyn HttpClient + Send + Sync>, email: &str, cache_dir: PathBuf) -> Self {
        let service = FullTextService::new(http.clone(), email)
            .with_cache(FullTextCache::new(Some(cache_dir)))
            .with_convert_pdfs(false);
        Self { service, http }
    }
}

impl FullTextSource for Sources {
    /// Asks PubMed, OpenAlex and Europe PMC once, a preprint accession going
    /// in as `ppr_id`.
    fn lookup(&self, ids: &Identifiers) -> Result<RecordLookup, LookupError> {
        let (pmid, ppr_id) = split_pmid(ids);
        let request = LookupRequest {
            pmid,
            ppr_id,
            doi: ids.doi.as_deref(),
            pmc_id: ids.pmc_id.as_deref(),
            email: self.service.email(),
            ..Default::default()
        };
        lookup_record(&*self.http, &request)
    }

    /// Looks the paper up once and copies bmlib's log lines into the app log.
    /// A link-only result from a chain where a tier faulted becomes an error,
    /// so an outage is retried and reported as unreachable, not as "no full text".
    fn fetch(&self, request: &FullTextRequest) -> Result<FullTextResult, FullTextError> {
        let service = &self.service;
        // The service is built per command, so clearing leaves exactly this call's lines.
        service.clear_log();
        let result = service.fetch_fulltext(request);
        let lines = service.log_lines();
        forward_log(&lines);
        match (result, faulted_exhaustion(&lines)) {
            (Ok(_), Some(message)) => Err(FullTextError::Other(message)),
            (result, _) => result,
        }
    }
}

/// Copies bmlib's log lines into the app log (bmlib itself only prints them
/// to stderr, which a packaged app does not keep).
fn forward_log(lines: &[LogLine]) {
    for line in lines {
        match line.level {
            LogLevel::Debug => log::debug!("bmlib: {}", line.message),
            LogLevel::Info => log::info!("bmlib: {}", line.message),
            LogLevel::Warning => log::warn!("bmlib: {}", line.message),
        }
    }
}

/// bmlib's "nothing found" warning when at least one tier faulted, if this
/// call logged one. bmlib returns `Ok` with only a link in that case whenever
/// a DOI or PMID was given, so the warning is the only sign of the fault.
// shortcut: parses bmlib's log wording, upgrade when bmlib #565 types it
fn faulted_exhaustion(lines: &[LogLine]) -> Option<String> {
    lines
        .iter()
        .find(|l| {
            l.is_warning()
                && l.message.starts_with(BMLIB_NOT_FOUND_PREFIX)
                && l.message.contains(BMLIB_FAULT_MARKER)
        })
        .map(|l| l.message.clone())
}

/// The outcome of looking a paper up by identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retrieved {
    /// Real full text was found; `origin` names the bmlib tier that served it.
    Found {
        /// Whether `bytes` are JATS XML or a PDF.
        kind: SourceKind,
        /// The bmlib source name, e.g. `europepmc`.
        origin: String,
        /// The raw document.
        bytes: Vec<u8>,
    },
    /// Any answer a retry would not change: the sources have no free full
    /// text we can use (an abstract is not full text and is never returned),
    /// a free PDF's download failed for good (the reason names the link), or
    /// a PDF they served could not be read from the cache.
    NotFound {
        /// Why, for the user.
        reason: String,
    },
    /// The sources could not be reached or failed; says nothing about whether
    /// full text exists.
    Unreachable {
        /// The last error, for the user.
        reason: String,
    },
    /// The user cancelled the lookup; nothing is known.
    Cancelled,
}

/// Builds bmlib's service and record lookup, the service's cache inside the
/// app's own storage. Every request they make honours `limit` (see
/// [`LimitedClient`]). bmlib extracts no PDF text: the app reads the cached PDF
/// itself (`convert`), so bmlib's own extraction would only warn that it has
/// no backend.
pub fn build_service(cache_dir: PathBuf, email: &str, limit: Limit) -> Sources {
    Sources::new(
        Arc::new(LimitedClient::new(UreqClient::new(), limit)),
        email,
        cache_dir,
    )
}

/// `ids.pmid` as (a PubMed ID, a preprint accession): 2a stores a PPR
/// accession in `pmid`, and only the first may be sent to bmlib as a PMID.
fn split_pmid(ids: &Identifiers) -> (Option<&str>, Option<&str>) {
    match ids.pmid.as_deref() {
        Some(p) if p.starts_with(PREPRINT_PREFIX) => (None, Some(p)),
        pmid => (pmid, None),
    }
}

/// What the record lookup found, ready for the full-text chain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    /// The request to make of the full-text chain.
    pub request: FullTextRequest,
    /// The record's title, when it has one.
    pub title: Option<String>,
    /// The journal version of a preprint, when the record names one.
    pub published_version: Option<PublishedVersion>,
    /// The record sources that could not be reached, named with their
    /// messages; `None` when every source answered.
    pub unreached: Option<String>,
}

/// Why a record lookup is not taken as it came back.
#[derive(Debug)]
enum Unanswered {
    /// A source could not be reached (worth a retry); what the others found,
    /// and the failed sources named.
    Partial(Box<RecordLookup>, String),
    /// bmlib refused the identifiers before asking anyone.
    Refused(LookupError),
}

impl std::fmt::Display for Unanswered {
    /// The failed sources, or bmlib's refusal.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unanswered::Partial(_, failed) => write!(f, "record sources unreachable: {failed}"),
            Unanswered::Refused(e) => write!(f, "record lookup refused: {e}"),
        }
    }
}

/// The sources of `lookup` whose request failed, each with its message, or
/// `None` when every source answered.
fn failed_sources(lookup: &RecordLookup) -> Option<String> {
    let failed: Vec<String> = [
        ("PubMed", &lookup.pubmed),
        ("OpenAlex", &lookup.openalex),
        ("Europe PMC", &lookup.europepmc),
    ]
    .into_iter()
    .filter(|(_, r)| r.outcome == SourceOutcome::RequestFailed)
    .map(|(name, r)| format!("{name} ({})", r.message))
    .collect();
    (!failed.is_empty()).then(|| failed.join("; "))
}

/// Looks the paper's record up, retrying with backoff while a source could
/// not be reached and `limit` has time left, and builds the full-text request
/// from it (see [`request_for`]). A source still unreachable at the end leaves
/// what the others found; a refusal (not expected for validated identifiers)
/// leaves the identifiers alone. Neither stops the full-text lookup.
pub fn resolve(
    source: &dyn FullTextSource,
    ids: &Identifiers,
    attempts: u32,
    base_delay: Duration,
    limit: &Limit,
) -> Resolved {
    let looked = with_backoff(
        attempts,
        base_delay,
        |e| matches!(e, Unanswered::Partial(..)) && limit.remaining().is_ok(),
        || match source.lookup(ids) {
            Ok(l) => match failed_sources(&l) {
                Some(failed) => Err(Unanswered::Partial(Box::new(l), failed)),
                None => Ok(l),
            },
            Err(e) => Err(Unanswered::Refused(e)),
        },
    );
    let (lookup, unreached) = match looked {
        Ok(l) => (Some(l), None),
        Err(Unanswered::Partial(l, failed)) => {
            log::warn!("record sources unreachable: {failed}");
            (Some(*l), Some(failed))
        }
        Err(Unanswered::Refused(e)) => {
            log::error!("record lookup refused {ids:?}: {e}");
            (None, None)
        }
    };
    let (record, published_version) = match lookup {
        Some(l) => (l.record, l.published_version),
        None => (None, None),
    };
    Resolved {
        request: request_for(ids, record.as_ref()),
        title: record
            .as_ref()
            .map(|r| r.title.trim().to_string())
            .filter(|t| !t.is_empty()),
        published_version,
        unreached,
    }
}

/// The full-text request for `ids`, filled from `record`: the identifiers
/// typed win, the record's fill what they lack, its full-text sources go along,
/// and a preprint accession is never sent as a PMID (only as the cache key,
/// when nothing else names the paper).
pub fn request_for(ids: &Identifiers, record: Option<&FetchedRecord>) -> FullTextRequest {
    let (pmid, ppr_id) = split_pmid(ids);
    let pmid = pmid.map(str::to_string).or_else(|| {
        record
            .and_then(|r| r.pmid.clone())
            .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    });
    let doi = ids
        .doi
        .clone()
        .or_else(|| record.and_then(|r| r.doi.clone()));
    let pmc_id = ids
        .pmc_id
        .clone()
        .or_else(|| record.and_then(|r| r.pmc_id.clone()));
    let identifier = doi
        .clone()
        .or(pmid.clone())
        .or(pmc_id.clone())
        .or(ppr_id.map(str::to_string));
    FullTextRequest {
        fulltext_sources: record.map(source_entries).unwrap_or_default(),
        pmc_id,
        doi,
        pmid: pmid.unwrap_or_default(),
        identifier,
    }
}

/// The record's full-text sources as the chain takes them. bmlib's record
/// holds them as JSON; one that does not read as an entry is logged and left out.
// shortcut: a dropped entry is only logged, upgrade when bmlib #624 types the record's entries
fn source_entries(record: &FetchedRecord) -> Vec<FullTextSourceEntry> {
    record
        .fulltext_sources
        .iter()
        .filter_map(|v| match serde_json::from_value(v.clone()) {
            Ok(entry) => Some(entry),
            Err(e) => {
                log::warn!("a full-text source of the record is left out ({e}): {v}");
                None
            }
        })
        .collect()
}

/// Whether `request` gives bmlib's chain nothing to look the paper up by.
fn nothing_to_ask(request: &FullTextRequest) -> bool {
    request.fulltext_sources.is_empty()
        && request.pmc_id.is_none()
        && request.doi.is_none()
        && request.pmid.is_empty()
}

/// A notice that the preprint opened has a journal version, naming it.
pub fn published_version_notice(v: &PublishedVersion) -> String {
    let pmc = v
        .pmc_id
        .as_deref()
        .map(|p| format!(", {p}"))
        .unwrap_or_default();
    format!(
        "This is a preprint. Its journal version is PMID {} (DOI {}{pmc}); open that to read the published paper.",
        v.pmid, v.doi
    )
}

/// Looks the paper's full text up as `resolved` says, retrying with backoff
/// whatever `classify_error` calls `Retry` (a transport or tier fault) while
/// `limit` has time left. With nothing to ask by (a preprint accession no
/// record source named), bmlib is not called: `Unreachable` when a record
/// source could not be reached, else `NotFound`.
/// `Unavailable`, "no identifiers" and an exhausted chain with no fault are
/// final and become `NotFound`. A free PDF whose download failed is a fault
/// when a retry may fetch it, else a final `NotFound` naming the link (see
/// [`download_failure_as_error`]). A cancel ends as `Cancelled` whatever
/// came back, even full text. After the deadline, full text is still `Found`,
/// a definite "nothing" is still `NotFound`, and a fault is `Unreachable`
/// naming the limit.
pub fn retrieve(
    source: &dyn FullTextSource,
    resolved: &Resolved,
    attempts: u32,
    base_delay: Duration,
    limit: &Limit,
) -> Retrieved {
    let request = &resolved.request;
    let result = if nothing_to_ask(request) {
        // Only a preprint accession no source could name: nothing for bmlib to ask by.
        Err(match &resolved.unreached {
            Some(failed) => FullTextError::Other(failed.clone()),
            None => FullTextError::Unavailable(
                "No source names this paper by a PubMed ID, PMC ID or DOI, or links its full text, so there is nothing to look the full text up by."
                    .into(),
            ),
        })
    } else {
        with_backoff(
            attempts,
            base_delay,
            |e| classify_error(e) == ErrorClass::Retry && limit.remaining().is_ok(),
            || source.fetch(request).and_then(download_failure_as_error),
        )
    };
    let stopped = limit.remaining().err();
    if stopped == Some(Stop::Cancelled) {
        let had = if result.is_ok() {
            "; its result is discarded"
        } else {
            ""
        };
        log::info!("full-text lookup cancelled by the user{had}");
        return Retrieved::Cancelled;
    }
    match result {
        Err(FullTextError::Unavailable(reason)) => {
            log::info!("no full text retrieved: {reason}");
            Retrieved::NotFound { reason }
        }
        Err(e @ FullTextError::Other(_)) if classify_error(&e) == ErrorClass::Nothing => {
            let reason = e.to_string();
            log::info!("no free full text: {reason}");
            Retrieved::NotFound { reason }
        }
        Err(FullTextError::Other(m)) if stopped == Some(Stop::TimedOut) => timed_out(limit, &m),
        Err(FullTextError::Other(m)) => {
            log::error!("full-text sources unreachable: {m}");
            Retrieved::Unreachable {
                reason: format!("The full-text sources could not be reached: {m}"),
            }
        }
        Ok(r) => classify(r),
    }
}

/// Turns a result whose free PDF failed to download (bmlib's
/// `pdf_download_failure`) into an error, unless it carries JATS. A failure a
/// retry may get past (see [`download_may_recover`]) is a fault, so the lookup
/// is retried and, if the download keeps failing, ends as `Unreachable`; a
/// download the deadline cut short is one too. Any other is a final
/// `Unavailable` that names the link, so the user can fetch the PDF by hand.
fn download_failure_as_error(r: FullTextResult) -> Result<FullTextResult, FullTextError> {
    let Some(cause) = r.pdf_download_failure.as_deref() else {
        return Ok(r);
    };
    if jats(&r).is_some() {
        return Ok(r);
    }
    let url = r.pdf_url.as_deref().unwrap_or("an unnamed address");
    let words = describe_download_failure(cause);
    Err(if download_may_recover(cause) {
        FullTextError::Other(format!("a free PDF was found at {url} but {words}"))
    } else {
        FullTextError::Unavailable(format!(
            "A free PDF was found at {url} but {words}. Download it from there and upload it here."
        ))
    })
}

/// Whether a retry may get past one of bmlib's `pdf_download_failure` causes:
/// a failed request, a server error, 408 or 429, or a cause bmlib has added
/// since. A web page served instead of a PDF, another refusal or a storage
/// that cannot save the file fails the same way again.
fn download_may_recover(cause: &str) -> bool {
    match cause {
        "not-a-pdf" | "write-failed" => false,
        _ => match cause.strip_prefix("http-").map(str::parse::<u16>) {
            Some(Ok(status)) => {
                status >= HTTP_SERVER_ERROR_MIN || HTTP_RETRYABLE_CLIENT_ERRORS.contains(&status)
            }
            _ => true,
        },
    }
}

/// Words for one of bmlib's `pdf_download_failure` causes (`http-<status>`,
/// `not-a-pdf`, `request-failed`, `write-failed`); an unknown one is shown as is.
fn describe_download_failure(cause: &str) -> String {
    match cause {
        "not-a-pdf" => "what it served was not a PDF".into(),
        "request-failed" => "its download failed (timeout, reset or no network)".into(),
        "write-failed" => "it could not be saved in the app's storage".into(),
        _ => match cause.strip_prefix("http-") {
            Some(status) => format!("its server answered HTTP {status}"),
            None => format!("its download failed ({cause})"),
        },
    }
}

/// The result's JATS bytes, if it holds real full text as JATS.
fn jats(r: &FullTextResult) -> Option<&Vec<u8>> {
    r.jats_xml
        .as_ref()
        .filter(|_| r.content_kind == ContentKind::Fulltext)
}

/// The `Unreachable` of a lookup that ran past `limit`, naming the time limit
/// and the last thing that went wrong (`last`), which may be the real cause.
fn timed_out(limit: &Limit, last: &str) -> Retrieved {
    log::error!("full-text lookup gave up after {:?}: {last}", limit.total());
    Retrieved::Unreachable {
        reason: format!(
            "The full-text sources did not answer within {} seconds. The last error was: {last}",
            limit.total().as_secs()
        ),
    }
}

/// The `NotFound` of a PDF that was retrieved but whose copy in the app's
/// storage cannot be read (`why`), naming its link when bmlib gives one.
fn unreadable_cached_pdf(r: &FullTextResult, why: &str) -> Retrieved {
    let (from, next) = match r.pdf_url.as_deref() {
        Some(url) => (format!(" from {url}"), "Download it and upload it here."),
        None => (String::new(), "Find the PDF and upload it here."),
    };
    Retrieved::NotFound {
        reason: format!(
            "A PDF was retrieved{from}, but the copy saved in the app's storage could not be read ({why}). {next}"
        ),
    }
}

/// How a bmlib error should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorClass {
    /// The sources answered and have nothing: final, not retried.
    Nothing,
    /// A transport or other fault: worth retrying.
    Retry,
}

/// Sorts a bmlib error into [`ErrorClass`].
// shortcut: parses bmlib's exhaustion wording, upgrade when bmlib #565 types it
fn classify_error(err: &FullTextError) -> ErrorClass {
    match err {
        FullTextError::Unavailable(_) => ErrorClass::Nothing,
        FullTextError::Other(m) if m == BMLIB_NO_IDENTIFIERS => ErrorClass::Nothing,
        FullTextError::Other(m)
            if m.starts_with(BMLIB_EXHAUSTED_PREFIX) && !m.contains(BMLIB_FAULT_MARKER) =>
        {
            ErrorClass::Nothing
        }
        FullTextError::Other(_) => ErrorClass::Retry,
    }
}

/// Turns a bmlib result into `Found` only for real full text. A cached file
/// that starts with the PDF magic is a PDF whatever `content_kind` says; bmlib
/// caches only PDFs, so one that cannot be read, or is not a PDF, is a local
/// fault: a final `NotFound` naming it (see [`unreadable_cached_pdf`]), never
/// "check the connection".
fn classify(r: FullTextResult) -> Retrieved {
    // Structured JATS is the better source, so it wins over a cached PDF.
    if let Some(bytes) = jats(&r).cloned() {
        return Retrieved::Found {
            kind: SourceKind::Jats,
            origin: r.source,
            bytes,
        };
    }
    if let Some(path) = &r.file_path {
        // The path is inside bmlib's cache, which we point at our own cache
        // dir; it is not an outside path.
        match std::fs::read(path) {
            Ok(bytes) if bytes.starts_with(PDF_MAGIC) => {
                return Retrieved::Found {
                    kind: SourceKind::Pdf,
                    origin: r.source,
                    bytes,
                };
            }
            Ok(_) => {
                log::error!("cached file {path} is not a PDF");
                return unreadable_cached_pdf(&r, "the saved file is not a PDF");
            }
            Err(e) => {
                log::error!("cannot read cached PDF {path}: {e}");
                return unreadable_cached_pdf(&r, &e.to_string());
            }
        }
    }
    log::info!(
        "no usable full text from {} (content kind {:?}, JATS {})",
        r.source,
        r.content_kind,
        if r.jats_xml.is_some() {
            "present"
        } else {
            "absent"
        }
    );
    Retrieved::NotFound {
        reason: "Only an abstract or a link is available, not the full text.".into(),
    }
}

/// A record lookup that found nothing and asked no source, for test fakes.
#[cfg(test)]
pub(crate) fn no_record() -> RecordLookup {
    let not_asked = || bmlib::publications::lookup::SourceReport {
        outcome: SourceOutcome::NotAsked,
        message: String::new(),
    };
    RecordLookup {
        record: None,
        published_version: None,
        pubmed: not_asked(),
        openalex: not_asked(),
        europepmc: not_asked(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RETRIEVAL_ATTEMPTS;
    use bmlib::publications::fetchers::registry::{FetchError, HttpClient, HttpResponse};
    use bmlib::publications::lookup::SourceReport;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// A source replaying a queue of full-text results and one of record
    /// lookups (`no_record` once that is empty), keeping what it was asked.
    struct FakeSource {
        queue: RefCell<VecDeque<Result<FullTextResult, FullTextError>>>,
        seen: RefCell<Vec<FullTextRequest>>,
        lookups: RefCell<VecDeque<RecordLookup>>,
        lookup_calls: RefCell<usize>,
    }

    impl FakeSource {
        fn new(items: Vec<Result<FullTextResult, FullTextError>>) -> Self {
            Self::with_lookups(items, vec![])
        }
        fn with_lookups(
            items: Vec<Result<FullTextResult, FullTextError>>,
            lookups: Vec<RecordLookup>,
        ) -> Self {
            Self {
                queue: RefCell::new(items.into()),
                seen: RefCell::new(vec![]),
                lookups: RefCell::new(lookups.into()),
                lookup_calls: RefCell::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.seen.borrow().len()
        }
    }

    impl FullTextSource for FakeSource {
        fn lookup(&self, _: &Identifiers) -> Result<RecordLookup, LookupError> {
            *self.lookup_calls.borrow_mut() += 1;
            Ok(self
                .lookups
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(no_record))
        }
        fn fetch(&self, request: &FullTextRequest) -> Result<FullTextResult, FullTextError> {
            self.seen.borrow_mut().push(request.clone());
            self.queue.borrow_mut().pop_front().expect("queue empty")
        }
    }

    fn ids() -> Identifiers {
        Identifiers {
            pmid: Some("123".into()),
            doi: Some("10.1000/abc".into()),
            pmc_id: Some("PMC9".into()),
        }
    }

    fn result(kind: ContentKind, jats: Option<&[u8]>) -> FullTextResult {
        FullTextResult {
            source: "europepmc".into(),
            html: Some("<p>x</p>".into()),
            pdf_url: None,
            pdf_download_failure: None,
            pdf_extraction_failure: None,
            web_url: None,
            file_path: None,
            content_kind: kind,
            jats_xml: jats.map(|b| b.to_vec()),
        }
    }

    fn run(f: &FakeSource) -> Retrieved {
        retrieve(
            f,
            &plain(&ids()),
            RETRIEVAL_ATTEMPTS,
            Duration::ZERO,
            &ample(),
        )
    }

    /// `ids` resolved with no record found.
    fn plain(ids: &Identifiers) -> Resolved {
        Resolved {
            request: request_for(ids, None),
            ..Default::default()
        }
    }

    /// A limit no test runs into.
    fn ample() -> Limit {
        Limit::new(Duration::from_secs(3600))
    }

    /// A source that cancels `limit` during its first call: the lookup ends
    /// as `Cancelled` without a retry, even though the error was retryable.
    #[test]
    fn cancel_during_a_call_is_cancelled() {
        struct Cancelling(Limit, RefCell<usize>);
        impl FullTextSource for Cancelling {
            fn lookup(&self, _: &Identifiers) -> Result<RecordLookup, LookupError> {
                Ok(no_record())
            }
            fn fetch(&self, _: &FullTextRequest) -> Result<FullTextResult, FullTextError> {
                *self.1.borrow_mut() += 1;
                self.0.cancel();
                Err(FullTextError::Other("reset".into()))
            }
        }
        let limit = ample();
        let src = Cancelling(limit.clone(), RefCell::new(0));
        let got = retrieve(
            &src,
            &plain(&ids()),
            RETRIEVAL_ATTEMPTS,
            Duration::ZERO,
            &limit,
        );
        assert_eq!(got, Retrieved::Cancelled);
        assert_eq!(*src.1.borrow(), 1);
    }

    /// Past the deadline a fault is not retried and is reported as a timeout;
    /// a definite "nothing" is still `NotFound`.
    #[test]
    fn deadline_stops_retries() {
        let expired = Limit::new(Duration::ZERO);
        let f = FakeSource::new(vec![Err(FullTextError::Other("reset".into()))]);
        match retrieve(
            &f,
            &plain(&ids()),
            RETRIEVAL_ATTEMPTS,
            Duration::ZERO,
            &expired,
        ) {
            Retrieved::Unreachable { reason } => {
                assert!(reason.contains("within 0 seconds"), "{reason}")
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
        assert_eq!(f.calls(), 1);
        let f = FakeSource::new(vec![Err(FullTextError::Unavailable("none".into()))]);
        assert!(matches!(
            retrieve(
                &f,
                &plain(&ids()),
                RETRIEVAL_ATTEMPTS,
                Duration::ZERO,
                &expired
            ),
            Retrieved::NotFound { .. }
        ));
    }

    /// A result whose free PDF failed to download, with `cause`, shaped as
    /// bmlib's PDF tiers return one when no abstract was found beside it.
    fn failed_download(cause: &str) -> FullTextResult {
        FullTextResult {
            source: "unpaywall".into(),
            html: None,
            pdf_url: Some("https://example.org/a.pdf".into()),
            pdf_download_failure: Some(cause.into()),
            ..result(ContentKind::None, None)
        }
    }

    /// Past the deadline, a PDF download it cut short is a timeout, not "no
    /// full text"; a bare link is still "nothing", and full text that did
    /// arrive is still found.
    #[test]
    fn deadline_failed_download_is_unreachable_full_text_is_found() {
        let expired = Limit::new(Duration::ZERO);
        let f = FakeSource::new(vec![Ok(failed_download("request-failed"))]);
        match retrieve(
            &f,
            &plain(&ids()),
            RETRIEVAL_ATTEMPTS,
            Duration::ZERO,
            &expired,
        ) {
            Retrieved::Unreachable { reason } => {
                assert!(reason.contains("within 0 seconds"), "{reason}");
                assert!(reason.contains("https://example.org/a.pdf"), "{reason}");
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
        let link_only = FullTextResult {
            pdf_url: Some("https://example.org/a.pdf".into()),
            ..result(ContentKind::Abstract, None)
        };
        let f = FakeSource::new(vec![Ok(link_only)]);
        assert!(matches!(
            retrieve(
                &f,
                &plain(&ids()),
                RETRIEVAL_ATTEMPTS,
                Duration::ZERO,
                &expired
            ),
            Retrieved::NotFound { .. }
        ));
        let f = FakeSource::new(vec![Ok(result(ContentKind::Fulltext, Some(b"<article/>")))]);
        assert!(matches!(
            retrieve(
                &f,
                &plain(&ids()),
                RETRIEVAL_ATTEMPTS,
                Duration::ZERO,
                &expired
            ),
            Retrieved::Found { .. }
        ));
    }

    /// A cancel discards even full text that had already arrived.
    #[test]
    fn cancel_discards_found() {
        let cancelled = ample();
        cancelled.cancel();
        let f = FakeSource::new(vec![Ok(result(ContentKind::Fulltext, Some(b"<article/>")))]);
        assert_eq!(
            retrieve(
                &f,
                &plain(&ids()),
                RETRIEVAL_ATTEMPTS,
                Duration::ZERO,
                &cancelled
            ),
            Retrieved::Cancelled
        );
    }

    #[test]
    fn jats_found() {
        let f = FakeSource::new(vec![Ok(result(ContentKind::Fulltext, Some(b"<article/>")))]);
        assert_eq!(
            run(&f),
            Retrieved::Found {
                kind: SourceKind::Jats,
                origin: "europepmc".into(),
                bytes: b"<article/>".to_vec()
            }
        );
    }

    /// A failed PDF download is retried, and one that keeps failing is
    /// `Unreachable` naming the cause; a retry that downloads it is `Found`.
    #[test]
    fn failed_download_is_retried_then_unreachable() {
        let f = FakeSource::new(vec![
            Ok(failed_download("http-503")),
            Ok(failed_download("http-503")),
            Ok(failed_download("http-503")),
        ]);
        match run(&f) {
            Retrieved::Unreachable { reason } => {
                assert!(reason.contains("answered HTTP 503"), "{reason}")
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
        assert_eq!(f.calls(), RETRIEVAL_ATTEMPTS as usize);
        let (cached, dir) = pdf_result(ContentKind::None, "a.pdf", b"%PDF-1.7");
        let f = FakeSource::new(vec![Ok(failed_download("request-failed")), Ok(cached)]);
        assert!(matches!(
            run(&f),
            Retrieved::Found {
                kind: SourceKind::Pdf,
                ..
            }
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A download that will fail the same way again is not retried: it is
    /// `NotFound` naming the link, so the user can fetch the PDF by hand.
    #[test]
    fn lasting_download_failure_is_final_and_names_the_link() {
        for cause in ["not-a-pdf", "http-403", "http-404", "write-failed"] {
            let f = FakeSource::new(vec![Ok(failed_download(cause))]);
            match run(&f) {
                Retrieved::NotFound { reason } => {
                    assert!(reason.contains("https://example.org/a.pdf"), "{reason}");
                    assert!(reason.contains("upload it here"), "{reason}");
                }
                other => panic!("{cause}: expected NotFound, got {other:?}"),
            }
            assert_eq!(f.calls(), 1, "{cause}");
        }
    }

    #[test]
    fn download_failures_that_may_recover() {
        for (cause, may) in [
            ("request-failed", true),
            ("http-500", true),
            ("http-503", true),
            ("http-408", true),
            ("http-429", true),
            ("http-403", false),
            ("http-404", false),
            ("not-a-pdf", false),
            ("write-failed", false),
            ("http-odd", true),
            ("new-cause", true),
        ] {
            assert_eq!(download_may_recover(cause), may, "{cause}");
        }
    }

    /// An abstract's JATS beside a failed download (bmlib's
    /// `with_abstract_fallback`) is no full text: the download is still a fault.
    #[test]
    fn abstract_jats_beside_failed_download_is_retried() {
        let r = FullTextResult {
            content_kind: ContentKind::Abstract,
            jats_xml: Some(b"<article/>".to_vec()),
            ..failed_download("http-503")
        };
        let f = FakeSource::new((0..RETRIEVAL_ATTEMPTS).map(|_| Ok(r.clone())).collect());
        assert!(matches!(run(&f), Retrieved::Unreachable { .. }));
        assert_eq!(f.calls(), RETRIEVAL_ATTEMPTS as usize);
    }

    /// JATS beside a failed PDF download is still found, first time.
    #[test]
    fn jats_beside_failed_download_is_found() {
        let r = FullTextResult {
            pdf_download_failure: Some("not-a-pdf".into()),
            ..result(ContentKind::Fulltext, Some(b"<article/>"))
        };
        let f = FakeSource::new(vec![Ok(r)]);
        assert!(matches!(
            run(&f),
            Retrieved::Found {
                kind: SourceKind::Jats,
                ..
            }
        ));
        assert_eq!(f.calls(), 1);
    }

    #[test]
    fn download_failures_are_worded() {
        for (cause, words) in [
            ("http-403", "its server answered HTTP 403"),
            ("not-a-pdf", "what it served was not a PDF"),
            (
                "request-failed",
                "its download failed (timeout, reset or no network)",
            ),
            ("write-failed", "it could not be saved in the app's storage"),
            ("new-cause", "its download failed (new-cause)"),
        ] {
            assert_eq!(describe_download_failure(cause), words);
        }
    }

    #[test]
    fn abstract_is_not_found() {
        let f = FakeSource::new(vec![Ok(result(ContentKind::Abstract, Some(b"<article/>")))]);
        assert!(matches!(run(&f), Retrieved::NotFound { .. }));
    }

    /// A cached PDF is read from its path; one that cannot be read is a
    /// final `NotFound` whatever `content_kind` says (bmlib extracts nothing
    /// here), not "unreachable", and is not retried.
    #[test]
    fn pdf_is_read_from_cache_path() {
        let dir = std::env::temp_dir().join(format!("sa-retr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.pdf");
        std::fs::write(&p, b"%PDF-x").unwrap();
        let mut r = result(ContentKind::None, None);
        r.file_path = Some(p.to_string_lossy().into());
        let f = FakeSource::new(vec![Ok(r)]);
        assert!(
            matches!(run(&f), Retrieved::Found { kind: SourceKind::Pdf, bytes, .. } if bytes == b"%PDF-x")
        );
        for kind in [ContentKind::None, ContentKind::Abstract] {
            let mut r = result(kind, None);
            r.file_path = Some(dir.join("missing.pdf").to_string_lossy().into());
            r.pdf_url = Some("https://x.org/a.pdf".into());
            let f = FakeSource::new(vec![Ok(r)]);
            match run(&f) {
                Retrieved::NotFound { reason } => {
                    assert!(reason.contains("from https://x.org/a.pdf"), "{reason}");
                    assert!(reason.contains("could not be read"), "{reason}");
                }
                other => panic!("expected NotFound, got {other:?} ({kind:?})"),
            }
            assert_eq!(f.calls(), 1);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn pdf_result(kind: ContentKind, name: &str, bytes: &[u8]) -> (FullTextResult, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sa-retr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        let mut r = result(kind, None);
        r.file_path = Some(p.to_string_lossy().into());
        (r, dir)
    }

    /// A cached PDF is found whatever the kind; a cached file that is not a
    /// PDF is a final `NotFound`, since bmlib caches only PDFs.
    #[test]
    fn pdf_file_is_found_whatever_the_kind() {
        let (r, dir) = pdf_result(ContentKind::None, "a.pdf", b"%PDF-1.7");
        let f = FakeSource::new(vec![Ok(r)]);
        assert!(matches!(
            run(&f),
            Retrieved::Found {
                kind: SourceKind::Pdf,
                ..
            }
        ));
        let (r, dir2) = pdf_result(ContentKind::Abstract, "a.html", b"<html>");
        let f = FakeSource::new(vec![Ok(r)]);
        // With no link to name, the reason does not send the user to one.
        assert!(matches!(run(&f), Retrieved::NotFound { reason }
            if reason.contains("not a PDF") && reason.ends_with("Find the PDF and upload it here.")));
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(dir2).unwrap();
    }

    #[test]
    fn jats_wins_over_pdf_file() {
        let (mut r, dir) = pdf_result(ContentKind::Fulltext, "a.pdf", b"%PDF-1.7");
        r.jats_xml = Some(b"<article/>".to_vec());
        let f = FakeSource::new(vec![Ok(r)]);
        assert!(matches!(
            run(&f),
            Retrieved::Found {
                kind: SourceKind::Jats,
                ..
            }
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn extracted_without_file_is_not_found() {
        let f = FakeSource::new(vec![Ok(result(ContentKind::Extracted, None))]);
        assert!(matches!(run(&f), Retrieved::NotFound { .. }));
    }

    #[test]
    fn exhaustion_with_only_absences_is_not_found() {
        let m = "Nothing retrieved and no DOI or PMID to fall back on — 3 sources had nothing";
        let f = FakeSource::new(vec![Err(FullTextError::Other(m.into()))]);
        assert!(matches!(run(&f), Retrieved::NotFound { .. }));
        assert_eq!(f.calls(), 1);
    }

    #[test]
    fn exhaustion_with_faults_is_retried_then_unreachable() {
        let m = "Nothing retrieved and no DOI or PMID to fall back on — 2 attempts failed (Transport); 3 sources had nothing";
        let f = FakeSource::new(
            (0..RETRIEVAL_ATTEMPTS)
                .map(|_| Err(FullTextError::Other(m.into())))
                .collect(),
        );
        assert!(matches!(run(&f), Retrieved::Unreachable { .. }));
        assert_eq!(f.calls(), RETRIEVAL_ATTEMPTS as usize);
    }

    #[test]
    fn no_identifiers_is_not_found_once() {
        let f = FakeSource::new(vec![Err(FullTextError::Other(
            "No identifiers provided".into(),
        ))]);
        assert!(matches!(run(&f), Retrieved::NotFound { .. }));
        assert_eq!(f.calls(), 1);
    }

    #[test]
    fn unavailable_not_retried() {
        let f = FakeSource::new(vec![Err(FullTextError::Unavailable("none".into()))]);
        assert!(matches!(run(&f), Retrieved::NotFound { .. }));
        assert_eq!(f.calls(), 1);
    }

    #[test]
    fn network_down_is_unreachable() {
        let f = FakeSource::new(
            (0..RETRIEVAL_ATTEMPTS)
                .map(|_| Err(FullTextError::Other("connection refused".into())))
                .collect(),
        );
        match run(&f) {
            Retrieved::Unreachable { reason } => assert!(reason.contains("connection refused")),
            other => panic!("expected Unreachable, got {other:?}"),
        }
        assert_eq!(f.calls(), RETRIEVAL_ATTEMPTS as usize);
    }

    /// A fault followed by a hit is retried and then found.
    #[test]
    fn fault_then_hit_is_found() {
        let f = FakeSource::new(vec![
            Err(FullTextError::Other("reset".into())),
            Ok(result(ContentKind::Fulltext, Some(b"<article/>"))),
        ]);
        assert!(matches!(run(&f), Retrieved::Found { .. }));
        assert_eq!(f.calls(), 2);
    }

    /// A fault followed by a definite "nothing" ends as `NotFound`, not `Unreachable`.
    #[test]
    fn fault_then_unavailable_is_not_found() {
        let f = FakeSource::new(vec![
            Err(FullTextError::Other("reset".into())),
            Err(FullTextError::Unavailable("none".into())),
        ]);
        assert!(matches!(run(&f), Retrieved::NotFound { .. }));
        assert_eq!(f.calls(), 2);
    }

    /// A log line at `level` with `message`.
    fn line(level: LogLevel, message: &str) -> LogLine {
        LogLine {
            level,
            message: message.into(),
        }
    }

    /// Only bmlib's "nothing found" warning that names a fault counts.
    #[test]
    fn faulted_exhaustion_reads_the_warning() {
        let faulted = "No full text found for doi=Some(\"10.1/x\") pmid= — nothing was retrieved; 3 attempts failed (Transport)";
        assert_eq!(
            faulted_exhaustion(&[line(LogLevel::Info, "x"), line(LogLevel::Warning, faulted)]),
            Some(faulted.to_string())
        );
        let absent =
            "No full text found for doi=None pmid=1 — nothing was retrieved; 2 sources had nothing";
        assert_eq!(faulted_exhaustion(&[line(LogLevel::Warning, absent)]), None);
        assert_eq!(faulted_exhaustion(&[line(LogLevel::Info, faulted)]), None);
        assert_eq!(faulted_exhaustion(&[]), None);
    }

    /// An HTTP client with no network: every request fails in transport.
    struct Offline;
    impl HttpClient for Offline {
        fn get(&self, url: &str) -> Result<HttpResponse, FetchError> {
            Err(FetchError::Transport(format!("offline: {url}")))
        }
    }

    /// With the network down, bmlib's real chain answers `Ok` with a DOI link;
    /// that must come out as `Unreachable` after every attempt, not `NotFound`.
    #[test]
    fn real_service_offline_doi_is_unreachable() {
        let dir = std::env::temp_dir().join(format!("sa-off-{}", uuid::Uuid::new_v4()));
        let service = Sources::new(Arc::new(Offline), "a@b.org", dir.clone());
        let doi_only = Identifiers {
            doi: Some("10.1000/abc".into()),
            ..Default::default()
        };
        match retrieve(&service, &plain(&doi_only), 2, Duration::ZERO, &ample()) {
            Retrieved::Unreachable { reason } => {
                assert!(reason.contains(BMLIB_FAULT_MARKER), "{reason}")
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An HTTP client where Unpaywall lists a free PDF at [`PDF_URL`], which
    /// answers `pdf`; every other request is a 404. Counts the PDF requests.
    struct FreePdf {
        pdf: HttpResponse,
        pdf_requests: std::sync::Mutex<usize>,
    }
    const PDF_URL: &str = "https://example.org/free.pdf";
    impl HttpClient for FreePdf {
        fn get(&self, url: &str) -> Result<HttpResponse, FetchError> {
            let answer = |status: u16, body: &str| HttpResponse {
                status,
                body: body.as_bytes().to_vec(),
            };
            if url == PDF_URL {
                *self.pdf_requests.lock().unwrap() += 1;
                Ok(self.pdf.clone())
            } else if url.contains("api.unpaywall.org") {
                Ok(answer(
                    200,
                    &format!(r#"{{"best_oa_location":{{"url_for_pdf":"{PDF_URL}"}}}}"#),
                ))
            } else {
                Ok(answer(404, ""))
            }
        }
    }

    /// bmlib's real chain with a free PDF that fails to download: a server
    /// error is retried and `Unreachable`, a web page is final `NotFound`;
    /// both name the link.
    #[test]
    fn real_service_failed_pdf_download() {
        let doi_only = Identifiers {
            doi: Some("10.1000/abc".into()),
            ..Default::default()
        };
        for (status, body, attempts) in [(503, "", RETRIEVAL_ATTEMPTS), (200, "<html>", 1)] {
            let dir = std::env::temp_dir().join(format!("sa-pdf-{}", uuid::Uuid::new_v4()));
            let client = Arc::new(FreePdf {
                pdf: HttpResponse {
                    status,
                    body: body.as_bytes().to_vec(),
                },
                pdf_requests: Default::default(),
            });
            let service = Sources::new(client.clone(), "a@b.org", dir.clone());
            let got = retrieve(
                &service,
                &plain(&doi_only),
                RETRIEVAL_ATTEMPTS,
                Duration::ZERO,
                &ample(),
            );
            match (&got, status) {
                (Retrieved::Unreachable { reason }, 503) => {
                    assert!(reason.contains("answered HTTP 503"), "{reason}");
                    assert!(reason.contains(PDF_URL), "{reason}");
                }
                (Retrieved::NotFound { reason }, 200) => {
                    assert!(reason.contains("not a PDF"), "{reason}");
                    assert!(reason.contains(PDF_URL), "{reason}");
                }
                _ => panic!("HTTP {status}: unexpected {got:?}"),
            }
            assert_eq!(*client.pdf_requests.lock().unwrap(), attempts as usize);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn cache_key_prefers_doi() {
        let f = FakeSource::new(vec![Err(FullTextError::Unavailable("n".into()))]);
        run(&f);
        let seen = f.seen.borrow();
        assert_eq!(seen[0].identifier.as_deref(), Some("10.1000/abc"));
        assert_eq!(seen[0].pmid, "123");
    }

    /// bmlib's real chain over the built service honours the limit without
    /// touching the network: cancelled is `Cancelled`, expired is a timeout.
    #[test]
    fn real_service_honours_the_limit() {
        let dir = std::env::temp_dir().join(format!("sa-limit-{}", uuid::Uuid::new_v4()));
        let doi_only = Identifiers {
            doi: Some("10.1000/abc".into()),
            ..Default::default()
        };
        let cancelled = ample();
        cancelled.cancel();
        let service = build_service(dir.clone(), "a@b.org", cancelled.clone());
        assert_eq!(
            retrieve(&service, &plain(&doi_only), 3, Duration::ZERO, &cancelled),
            Retrieved::Cancelled
        );
        let expired = Limit::new(Duration::ZERO);
        let service = build_service(dir.clone(), "a@b.org", expired.clone());
        match retrieve(&service, &plain(&doi_only), 3, Duration::ZERO, &expired) {
            Retrieved::Unreachable { reason } => assert!(reason.contains("within"), "{reason}"),
            other => panic!("expected Unreachable, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_service_uses_cache_dir_and_email() {
        let dir = std::env::temp_dir().join(format!("sa-cache-{}", uuid::Uuid::new_v4()));
        let s = build_service(dir.clone(), "a@b.org", ample());
        assert_eq!(s.service.email(), "a@b.org");
        assert!(s.service.cache().unwrap().pdf_dir().starts_with(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A report of `outcome` with message `m`.
    fn report(outcome: SourceOutcome, m: &str) -> SourceReport {
        SourceReport {
            outcome,
            message: m.into(),
        }
    }

    /// A record titled "A trial", with a DOI, a PMID, a PMC ID and one source.
    fn record() -> FetchedRecord {
        let mut r = FetchedRecord::new(" A trial ", "openalex");
        r.doi = Some("10.1000/rec".into());
        r.pmid = Some("456".into());
        r.pmc_id = Some("PMC7".into());
        r.fulltext_sources = vec![
            serde_json::json!({"url": "https://x.org/a.pdf", "format": "pdf", "source": "publisher"}),
            serde_json::json!({"url": "no format"}),
        ];
        r
    }

    /// A lookup that found `record()`, Europe PMC reporting `epmc`.
    fn found(epmc: SourceOutcome) -> RecordLookup {
        RecordLookup {
            record: Some(record()),
            europepmc: report(epmc, "Europe PMC said so"),
            ..no_record()
        }
    }

    fn ppr() -> Identifiers {
        Identifiers {
            pmid: Some("PPR123".into()),
            ..Default::default()
        }
    }

    /// A preprint accession is never sent as a PMID; it is the cache key only
    /// when nothing else names the paper (#36).
    #[test]
    fn a_preprint_accession_is_not_a_pmid() {
        let r = request_for(&ppr(), None);
        assert_eq!(r.pmid, "");
        assert_eq!(r.identifier.as_deref(), Some("PPR123"));
        assert!(nothing_to_ask(&r));
        assert_eq!(split_pmid(&ppr()), (None, Some("PPR123")));
        assert_eq!(split_pmid(&ids()), (Some("123"), None));
    }

    /// The record fills what the identifiers typed lack and adds its sources;
    /// the typed ones win; a record PMID that is not digits is not sent.
    #[test]
    fn the_record_fills_the_request() {
        let r = request_for(&ppr(), Some(&record()));
        assert_eq!(r.pmid, "456");
        assert_eq!(r.doi.as_deref(), Some("10.1000/rec"));
        assert_eq!(r.pmc_id.as_deref(), Some("PMC7"));
        assert_eq!(r.identifier.as_deref(), Some("10.1000/rec"));
        assert_eq!(r.fulltext_sources.len(), 1);
        let r = request_for(&ids(), Some(&record()));
        assert_eq!(
            (r.pmid.as_str(), r.doi.as_deref(), r.pmc_id.as_deref()),
            ("123", Some("10.1000/abc"), Some("PMC9"))
        );
        let mut odd = record();
        odd.pmid = Some("PPR9".into());
        assert_eq!(request_for(&ppr(), Some(&odd)).pmid, "");
    }

    /// A source that could not be reached is retried; the lookup that
    /// answers gives its title (trimmed) and published version.
    #[test]
    fn resolve_retries_an_unreached_source() {
        let pv = PublishedVersion {
            pmid: "9".into(),
            doi: "10.1/j".into(),
            pmc_id: None,
        };
        let answered = RecordLookup {
            published_version: Some(pv.clone()),
            ..found(SourceOutcome::Found)
        };
        let f =
            FakeSource::with_lookups(vec![], vec![found(SourceOutcome::RequestFailed), answered]);
        let got = resolve(&f, &ppr(), RETRIEVAL_ATTEMPTS, Duration::ZERO, &ample());
        assert_eq!(*f.lookup_calls.borrow(), 2);
        assert_eq!(got.title.as_deref(), Some("A trial"));
        assert_eq!(got.published_version, Some(pv));
        assert_eq!(got.unreached, None);
        assert_eq!(got.request.doi.as_deref(), Some("10.1000/rec"));
    }

    /// A source unreachable on every try leaves what the others found, and
    /// says which; past the deadline it is not retried.
    #[test]
    fn resolve_keeps_a_partial_record() {
        let f = FakeSource::with_lookups(
            vec![],
            (0..RETRIEVAL_ATTEMPTS)
                .map(|_| found(SourceOutcome::RequestFailed))
                .collect(),
        );
        let got = resolve(&f, &ppr(), RETRIEVAL_ATTEMPTS, Duration::ZERO, &ample());
        assert_eq!(*f.lookup_calls.borrow(), RETRIEVAL_ATTEMPTS as usize);
        assert_eq!(
            got.unreached.as_deref(),
            Some("Europe PMC (Europe PMC said so)")
        );
        assert_eq!(got.request.pmid, "456");
        let f = FakeSource::with_lookups(vec![], vec![found(SourceOutcome::RequestFailed)]);
        let expired = Limit::new(Duration::ZERO);
        resolve(&f, &ppr(), RETRIEVAL_ATTEMPTS, Duration::ZERO, &expired);
        assert_eq!(*f.lookup_calls.borrow(), 1);
    }

    /// With nothing to ask by, bmlib is not called: an unreached record source
    /// is `Unreachable` naming it, else `NotFound`.
    #[test]
    fn nothing_to_ask_by_skips_the_chain() {
        let f = FakeSource::new(vec![]);
        let unreached = Resolved {
            unreached: Some("Europe PMC (timeout)".into()),
            ..plain(&ppr())
        };
        match retrieve(&f, &unreached, RETRIEVAL_ATTEMPTS, Duration::ZERO, &ample()) {
            Retrieved::Unreachable { reason } => {
                assert!(reason.contains("Europe PMC (timeout)"), "{reason}")
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
        match retrieve(
            &f,
            &plain(&ppr()),
            RETRIEVAL_ATTEMPTS,
            Duration::ZERO,
            &ample(),
        ) {
            Retrieved::NotFound { reason } => {
                assert!(reason.starts_with("No source names this paper"), "{reason}")
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
        assert_eq!(f.calls(), 0);
    }

    #[test]
    fn published_version_is_worded() {
        let mut v = PublishedVersion {
            pmid: "9".into(),
            doi: "10.1/j".into(),
            pmc_id: Some("PMC3".into()),
        };
        assert_eq!(
            published_version_notice(&v),
            "This is a preprint. Its journal version is PMID 9 (DOI 10.1/j, PMC3); open that to read the published paper."
        );
        v.pmc_id = None;
        assert!(published_version_notice(&v).contains("(DOI 10.1/j);"));
    }

    /// An HTTP client that answers every request 404 and keeps the URLs asked.
    #[derive(Default)]
    struct Recording(std::sync::Mutex<Vec<String>>);
    impl HttpClient for Recording {
        fn get(&self, url: &str) -> Result<HttpResponse, FetchError> {
            self.0.lock().unwrap().push(url.to_string());
            Ok(HttpResponse {
                status: 404,
                body: vec![],
            })
        }
    }

    /// bmlib's real lookup asks Europe PMC for a preprint accession as a
    /// preprint, and never asks PubMed for it (#36).
    #[test]
    fn real_lookup_asks_for_a_preprint_as_one() {
        let dir = std::env::temp_dir().join(format!("sa-ppr-{}", uuid::Uuid::new_v4()));
        let client = Arc::new(Recording::default());
        let sources = Sources::new(client.clone(), "a@b.org", dir.clone());
        let l = sources.lookup(&ppr()).unwrap();
        assert_eq!(l.pubmed.outcome, SourceOutcome::NotAsked);
        let urls = client.0.lock().unwrap();
        assert!(
            urls.iter()
                .any(|u| u.contains("PPR123") && u.contains("SRC") && u.contains("PPR")),
            "{urls:?}"
        );
        assert!(
            !urls
                .iter()
                .any(|u| u.contains("ncbi") || u.contains("pubmed")),
            "{urls:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
