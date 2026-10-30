//! A total time limit and a cancel switch for a lookup that makes many network
//! requests, and an HTTP client that honours both.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bmlib::http::UreqClient;
use bmlib::publications::fetchers::registry::{FetchError, HttpClient, HttpResponse};

/// Why a lookup stopped before it finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The user pressed *Cancel*.
    Cancelled,
    /// The lookup ran past its deadline.
    TimedOut,
}

/// One lookup's deadline and cancel switch. Clones share the switch, so the
/// UI can cancel through one clone while the lookup checks another.
#[derive(Debug, Clone)]
pub struct Limit {
    /// When the lookup must give up.
    deadline: Instant,
    /// Set once by [`Limit::cancel`]; never cleared.
    cancelled: Arc<AtomicBool>,
    /// The total time allowed, for messages.
    total: Duration,
}

impl Limit {
    /// A limit of `total` from now, not cancelled.
    pub fn new(total: Duration) -> Self {
        Limit {
            deadline: Instant::now() + total,
            cancelled: Arc::new(AtomicBool::new(false)),
            total,
        }
    }

    /// Cancels the lookup; every clone sees it.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// The time left, or why there is none. A cancel wins over a timeout.
    pub fn remaining(&self) -> Result<Duration, Stop> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(Stop::Cancelled);
        }
        match self.deadline.checked_duration_since(Instant::now()) {
            Some(left) if !left.is_zero() => Ok(left),
            _ => Err(Stop::TimedOut),
        }
    }

    /// The total time this limit allowed.
    pub fn total(&self) -> Duration {
        self.total
    }

    /// True when both handles share one cancel switch (one lookup).
    pub fn same_lookup(&self, other: &Limit) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }
}

/// An HTTP client that refuses to start a request once its [`Limit`] has
/// stopped, and gives each request no more time than is left. A request that
/// is already running is not interrupted, but its timeout ends by the deadline
/// (rounded up to a whole second), so a cancel waits for at most one request.
pub struct LimitedClient {
    /// The real client; its `timeout_seconds` is the most any request gets.
    inner: UreqClient,
    /// The lookup this client works for.
    limit: Limit,
}

impl LimitedClient {
    /// Wraps `inner` so that it honours `limit`.
    pub fn new(inner: UreqClient, limit: Limit) -> Self {
        LimitedClient { inner, limit }
    }
}

impl HttpClient for LimitedClient {
    /// A GET through the inner client with its timeout cut to the time left;
    /// a `Transport` error without any request when the limit has stopped.
    fn get(&self, url: &str) -> Result<HttpResponse, FetchError> {
        let left = self.limit.remaining().map_err(|stop| {
            FetchError::Transport(format!("lookup stopped ({stop:?}) before GET {url}"))
        })?;
        let client = UreqClient {
            timeout_seconds: request_timeout(left, self.inner.timeout_seconds),
            ..self.inner.clone()
        };
        client.get(url)
    }
}

/// Whole seconds a request may take: the time left rounded up, but never more
/// than `max_seconds` (the client's own timeout).
fn request_timeout(left: Duration, max_seconds: u64) -> u64 {
    let up = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    up.min(max_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh limit has time left; a cancel is seen through a clone.
    #[test]
    fn cancel_is_shared_by_clones() {
        let a = Limit::new(Duration::from_secs(60));
        let b = a.clone();
        assert!(a.remaining().is_ok());
        assert!(a.same_lookup(&b));
        assert!(!a.same_lookup(&Limit::new(Duration::from_secs(60))));
        b.cancel();
        assert_eq!(a.remaining(), Err(Stop::Cancelled));
    }

    /// A zero limit is timed out at once; cancel wins over the timeout.
    #[test]
    fn zero_limit_is_timed_out() {
        let l = Limit::new(Duration::ZERO);
        assert_eq!(l.remaining(), Err(Stop::TimedOut));
        l.cancel();
        assert_eq!(l.remaining(), Err(Stop::Cancelled));
    }

    /// The timeout rounds up and is capped at the client's own.
    #[test]
    fn request_timeout_rounds_up_and_caps() {
        assert_eq!(request_timeout(Duration::from_millis(1), 45), 1);
        assert_eq!(request_timeout(Duration::from_millis(2_500), 45), 3);
        assert_eq!(request_timeout(Duration::from_secs(10), 45), 10);
        assert_eq!(request_timeout(Duration::from_secs(600), 45), 45);
    }

    /// A stopped limit refuses the request without touching the network.
    #[test]
    fn stopped_client_refuses() {
        let limit = Limit::new(Duration::from_secs(60));
        limit.cancel();
        let client = LimitedClient::new(UreqClient::new(), limit);
        match client.get("http://192.0.2.1/never") {
            Err(FetchError::Transport(m)) => assert!(m.contains("Cancelled"), "{m}"),
            other => panic!("expected Transport, got {other:?}"),
        }
    }
}
