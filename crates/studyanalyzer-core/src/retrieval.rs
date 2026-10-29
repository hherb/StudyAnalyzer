//! Fetching a paper's free full text by identifier, through bmlib.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bmlib::fulltext::cache::FullTextCache;
use bmlib::fulltext::models::{ContentKind, FullTextResult};
use bmlib::fulltext::service::{FullTextError, FullTextRequest, FullTextService};
use bmlib::http::UreqClient;

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
        |e| matches!(e, FullTextError::Other(_)),
        || source.fetch(&request),
    );
    match result {
        Err(FullTextError::Unavailable(reason)) => {
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

/// Turns a bmlib result into `Found` only for real full text.
fn classify(r: FullTextResult) -> Retrieved {
    match (r.content_kind, r.jats_xml, r.file_path) {
        (ContentKind::Fulltext, Some(bytes), _) => Retrieved::Found {
            kind: SourceKind::Jats,
            origin: r.source,
            bytes,
        },
        // The path is inside bmlib's cache, which we point at our own cache
        // dir; it is not an outside path.
        (ContentKind::Extracted, _, Some(path)) => match std::fs::read(&path) {
            Ok(bytes) => Retrieved::Found {
                kind: SourceKind::Pdf,
                origin: r.source,
                bytes,
            },
            Err(e) => {
                log::error!("cannot read cached PDF {path}: {e}");
                Retrieved::Unreachable {
                    reason: format!("The retrieved PDF could not be read: {e}"),
                }
            }
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
        let dir = std::env::temp_dir().join("sa-cache-test");
        let s = build_service(dir.clone(), "a@b.org");
        assert_eq!(s.email(), "a@b.org");
        assert!(s.cache().unwrap().pdf_dir().starts_with(&dir));
    }
}
