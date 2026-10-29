//! Retry with exponential backoff for calls that can fail or time out.

use std::time::Duration;

/// Calls `f` up to `attempts` times (at least once). After a failure that
/// `retryable` accepts, waits `base_delay * 2^n` (n = failures so far minus one)
/// and tries again; a non-retryable error, or the last attempt's error, is
/// returned at once. Each retry is logged as a warning.
pub fn with_backoff<T, E>(
    attempts: u32,
    base_delay: Duration,
    retryable: impl Fn(&E) -> bool,
    mut f: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    let attempts = attempts.max(1);
    let mut attempt = 0;
    loop {
        match f() {
            Ok(value) => return Ok(value),
            Err(e) if attempt + 1 < attempts && retryable(&e) => {
                let delay = base_delay.saturating_mul(2u32.saturating_pow(attempt));
                log::warn!(
                    "attempt {} of {attempts} failed; retrying in {delay:?}",
                    attempt + 1
                );
                std::thread::sleep(delay);
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_retries_then_succeeds() {
        let mut calls = 0;
        let r: Result<u32, &str> = with_backoff(
            3,
            Duration::ZERO,
            |_| true,
            || {
                calls += 1;
                if calls < 3 {
                    Err("down")
                } else {
                    Ok(calls)
                }
            },
        );
        assert_eq!(r, Ok(3));
        assert_eq!(calls, 3);
    }

    #[test]
    fn non_retryable_error_is_tried_once() {
        let mut calls = 0;
        let r: Result<(), &str> = with_backoff(
            3,
            Duration::ZERO,
            |_| false,
            || {
                calls += 1;
                Err("no")
            },
        );
        assert_eq!(r, Err("no"));
        assert_eq!(calls, 1);
    }

    #[test]
    fn gives_up_after_attempts() {
        let mut calls = 0;
        let r: Result<(), &str> = with_backoff(
            2,
            Duration::ZERO,
            |_| true,
            || {
                calls += 1;
                Err("x")
            },
        );
        assert!(r.is_err());
        assert_eq!(calls, 2);
    }
}
