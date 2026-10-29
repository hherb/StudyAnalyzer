//! Fetching a paper's free full text by identifier, through bmlib.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bmlib::fulltext::cache::FullTextCache;
use bmlib::fulltext::models::{ContentKind, FullTextResult};
use bmlib::fulltext::service::{FullTextError, FullTextRequest, FullTextService};
use bmlib::http::UreqClient;

use crate::config::{BMLIB_EXHAUSTED_PREFIX, BMLIB_FAULT_MARKER, BMLIB_NO_IDENTIFIERS, PDF_MAGIC};
use crate::identifiers::Identifiers;
use crate::intake::SourceKind;
use crate::retry::with_backoff;

/// Anything that can look up full text for a request (bmlib's service, or a
/// fake in tests).
pub trait FullTextSource {
    /// Looks the paper up once; no retrying here.
    fn fetch(&self, request: &FullTextRequest) -> Result<FullTextResult, FullTextError>;
}

impl FullTextSource for FullTextService {
    fn fetch(&self, request: &FullTextRequest) -> Result<FullTextResult, FullTextError> {
        self.fetch_fulltext(request)
    }
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
    /// The sources answered and have no free full text (an abstract is not
    /// full text and is never returned).
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
}

/// Builds bmlib's service with its cache inside the app's own storage.
pub fn build_service(cache_dir: PathBuf, email: &str) -> FullTextService {
    FullTextService::new(Arc::new(UreqClient::new()), email)
        .with_cache(FullTextCache::new(Some(cache_dir)))
}

/// Looks the paper up, retrying transport failures with backoff.
/// `Unavailable` (the sources have nothing) is final and not retried.
pub fn retrieve(
    source: &dyn FullTextSource,
    ids: &Identifiers,
    attempts: u32,
    base_delay: Duration,
) -> Retrieved {
    let request = FullTextRequest {
        pmid: ids.pmid.clone().unwrap_or_default(),
        doi: ids.doi.clone(),
        pmc_id: ids.pmc_id.clone(),
        identifier: ids.doi.clone().or(ids.pmid.clone()).or(ids.pmc_id.clone()),
        ..Default::default()
    };
    let result = with_backoff(
        attempts,
        base_delay,
        |e| classify_error(e) == ErrorClass::Retry,
        || source.fetch(&request),
    );
    match result {
        Err(FullTextError::Unavailable(reason)) => {
            log::info!("no free full text: {reason}");
            Retrieved::NotFound { reason }
        }
        Err(e @ FullTextError::Other(_)) if classify_error(&e) == ErrorClass::Nothing => {
            let reason = e.to_string();
            log::info!("no free full text: {reason}");
            Retrieved::NotFound { reason }
        }
        Err(FullTextError::Other(m)) => {
            log::error!("full-text sources unreachable: {m}");
            Retrieved::Unreachable {
                reason: format!("The full-text sources could not be reached: {m}"),
            }
        }
        Ok(r) => classify(r),
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
/// that starts with the PDF magic is a PDF whatever `content_kind` says.
fn classify(r: FullTextResult) -> Retrieved {
    // Structured JATS is the better source, so it wins over a cached PDF.
    if r.content_kind == ContentKind::Fulltext {
        if let Some(bytes) = r.jats_xml.clone() {
            return Retrieved::Found {
                kind: SourceKind::Jats,
                origin: r.source,
                bytes,
            };
        }
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
            Ok(_) => log::info!("cached file {path} is not a PDF; ignoring it"),
            Err(e) if r.content_kind == ContentKind::Extracted => {
                log::error!("cannot read cached PDF {path}: {e}");
                return Retrieved::Unreachable {
                    reason: format!("The retrieved PDF could not be read: {e}"),
                };
            }
            Err(e) => log::warn!("cannot read cached file {path}: {e}"),
        }
    }
    match (r.content_kind, r.jats_xml) {
        (ContentKind::Fulltext, Some(bytes)) => Retrieved::Found {
            kind: SourceKind::Jats,
            origin: r.source,
            bytes,
        },
        _ => Retrieved::NotFound {
            reason: "Only an abstract or a link is available, not the full text.".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RETRIEVAL_ATTEMPTS;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct FakeSource {
        queue: RefCell<VecDeque<Result<FullTextResult, FullTextError>>>,
        seen: RefCell<Vec<FullTextRequest>>,
    }

    impl FakeSource {
        fn new(items: Vec<Result<FullTextResult, FullTextError>>) -> Self {
            Self {
                queue: RefCell::new(items.into()),
                seen: RefCell::new(vec![]),
            }
        }
        fn calls(&self) -> usize {
            self.seen.borrow().len()
        }
    }

    impl FullTextSource for FakeSource {
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
            web_url: None,
            file_path: None,
            content_kind: kind,
            jats_xml: jats.map(|b| b.to_vec()),
        }
    }

    fn run(f: &FakeSource) -> Retrieved {
        retrieve(f, &ids(), RETRIEVAL_ATTEMPTS, Duration::ZERO)
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

    #[test]
    fn abstract_is_not_found() {
        let f = FakeSource::new(vec![Ok(result(ContentKind::Abstract, Some(b"<article/>")))]);
        assert!(matches!(run(&f), Retrieved::NotFound { .. }));
    }

    #[test]
    fn extracted_pdf_is_read_from_cache_path() {
        let dir = std::env::temp_dir().join(format!("sa-retr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.pdf");
        std::fs::write(&p, b"%PDF-x").unwrap();
        let mut r = result(ContentKind::Extracted, None);
        r.file_path = Some(p.to_string_lossy().into());
        let f = FakeSource::new(vec![Ok(r)]);
        assert!(
            matches!(run(&f), Retrieved::Found { kind: SourceKind::Pdf, bytes, .. } if bytes == b"%PDF-x")
        );
        let mut r = result(ContentKind::Extracted, None);
        r.file_path = Some(dir.join("missing.pdf").to_string_lossy().into());
        let f = FakeSource::new(vec![Ok(r)]);
        assert!(matches!(run(&f), Retrieved::Unreachable { .. }));
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
        assert!(matches!(run(&f), Retrieved::NotFound { .. }));
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

    #[test]
    fn cache_key_prefers_doi() {
        let f = FakeSource::new(vec![Err(FullTextError::Unavailable("n".into()))]);
        run(&f);
        let seen = f.seen.borrow();
        assert_eq!(seen[0].identifier.as_deref(), Some("10.1000/abc"));
        assert_eq!(seen[0].pmid, "123");
    }

    #[test]
    fn build_service_uses_cache_dir_and_email() {
        let dir = std::env::temp_dir().join(format!("sa-cache-{}", uuid::Uuid::new_v4()));
        let s = build_service(dir.clone(), "a@b.org");
        assert_eq!(s.email(), "a@b.org");
        assert!(s.cache().unwrap().pdf_dir().starts_with(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
