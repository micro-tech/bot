//! Shared retry policy with jittered exponential backoff (tasks 372-374, H3/H4/H5).
//!
//! One helper for every retry in the tree: the LLM HTTP calls (Ollama and
//! Gemini, per-round inside the tool loop), `call_gemini_direct`, the OKF
//! fetcher, and the sync IMAP connect. Policy: up to 5 attempts, base delay
//! 2 s doubling to a 60 s cap, with jitter so many clients do not re-hit
//! the server in lockstep. 429/503 honour `Retry-After`; every other 4xx
//! fails fast (retrying a client error never helps). Network/timeout
//! errors are retryable.
//!
//! The driver is generic over the operation: the caller supplies a closure
//! that returns `Ok(value)` on success or a [`RetryError`] on failure, and
//! the driver handles attempt counting and sleeping. The closure receives
//! the 0-based attempt index so it can compute its own delay (backoff or
//! `Retry-After`) for that attempt.

use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use std::time::Duration;

/// Maximum number of attempts for one operation (first try + retries).
pub const RETRY_MAX_ATTEMPTS: u32 = 5;
/// Base delay before the first retry; doubles each round.
pub const RETRY_BASE_DELAY_MS: u64 = 2_000;
/// Hard cap on any single backoff delay.
pub const RETRY_MAX_DELAY_MS: u64 = 60_000;

/// Outcome of classifying one failed attempt.
///
/// The operation returns this instead of a plain error so the driver knows
/// whether to retry (and for how long) or to fail immediately. `source` is
/// the error returned to the caller when retries are exhausted or the
/// failure is fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryError<E> {
    /// Retryable: try again after `delay`.
    Retryable { source: E, delay: Duration },
    /// Fatal: do not retry.
    Fatal(E),
}

/// Retry policy: attempt budget + backoff shape.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts including the first try.
    pub max_attempts: u32,
    /// Base delay (ms) before the first retry; doubles each round.
    pub base_delay_ms: u64,
    /// Hard cap (ms) on any single backoff delay.
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: RETRY_MAX_ATTEMPTS,
            base_delay_ms: RETRY_BASE_DELAY_MS,
            max_delay_ms: RETRY_MAX_DELAY_MS,
        }
    }
}

impl RetryPolicy {
    /// Pure exponential delay (ms) for a 0-based failed attempt index:
    /// base * 2^attempt, capped at max. No jitter — exact and testable.
    pub fn exponential_delay_ms(&self, attempt: u32) -> u64 {
        let shift = attempt.min(31);
        self.base_delay_ms
            .saturating_mul(1u64 << shift)
            .min(self.max_delay_ms)
    }

    /// Jittered delay for a 0-based failed attempt index: the exponential
    /// value plus up to 25% jitter, capped at max.
    pub fn backoff_delay(&self, attempt: u32) -> Duration {
        let base = self.exponential_delay_ms(attempt);
        Duration::from_millis(base.saturating_add(jitter_ms(base)).min(self.max_delay_ms))
    }

    /// Parse a `Retry-After` header value (seconds) into a delay, capped
    /// at max_delay. Returns None when absent or unparseable. The HTTP-date
    /// form is not parsed — the caller falls back to backoff.
    pub fn retry_after(&self, headers: &HeaderMap) -> Option<Duration> {
        let v = headers.get("retry-after")?.to_str().ok()?;
        let secs: u64 = v.trim().parse().ok()?;
        Some(Duration::from_secs(secs).min(Duration::from_millis(self.max_delay_ms)))
    }

    /// Delay to use after a retryable HTTP response: `Retry-After` when
    /// present, else the backoff for this attempt.
    pub fn delay_for_response(&self, attempt: u32, headers: &HeaderMap) -> Duration {
        self.retry_after(headers)
            .unwrap_or_else(|| self.backoff_delay(attempt))
    }
}

/// Small jitter: 0..=25% of `base`, from a cheap xorshift seeded on the
/// clock. Not cryptographic — it only needs to decorrelate retries.
fn jitter_ms(base: u64) -> u64 {
    if base == 0 {
        return 0;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    // xorshift64* mix of the nanos + base.
    let mut x = nanos ^ base.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    // 0..=25% of base.
    r % (base / 4 + 1)
}

/// Is this HTTP status retryable? 429/503 and 5xx are; every other 4xx
/// (400/401/403/404/...) fails fast — retrying a client error never helps.
pub fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::SERVICE_UNAVAILABLE
        || status.is_server_error()
}

/// Is this reqwest error retryable? Timeouts and connection-phase failures
/// (refused/DNS/TLS during connect) are transient; anything else is not.
pub fn is_retryable_reqwest_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect()
}

/// Async retry driver (tasks 372-374).
///
/// Calls `op(attempt)` until it succeeds, returns a fatal error, or the
/// attempt budget is exhausted. `attempt` is the 0-based index of the
/// failing try, so the closure can compute its delay for that attempt.
/// Sleeps with tokio between attempts; the caller stays async throughout.
///
/// The closure should be `async move` and clone what it needs (a
/// `reqwest::Client` is cheap to clone — Arc internally), so the returned
/// future owns its data.
pub async fn retry_with_backoff<T, E, F, Fut>(
    policy: &RetryPolicy,
    mut op: F,
) -> Result<T, E>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T, RetryError<E>>>,
{
    let mut attempt = 0u32;
    loop {
        match op(attempt).await {
            Ok(v) => return Ok(v),
            Err(RetryError::Fatal(e)) => return Err(e),
            Err(RetryError::Retryable { source, delay }) => {
                attempt += 1;
                if attempt >= policy.max_attempts {
                    return Err(source);
                }
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Sync retry driver for blocking operations (IMAP connect, task 372/374).
///
/// Same contract as [`retry_with_backoff`] but sleeps with
/// `std::thread::sleep`. The closure runs on the calling thread.
pub fn retry_with_backoff_sync<T, E, F>(
    policy: &RetryPolicy,
    mut op: F,
) -> Result<T, E>
where
    F: FnMut(u32) -> Result<T, RetryError<E>>,
{
    let mut attempt = 0u32;
    loop {
        match op(attempt) {
            Ok(v) => return Ok(v),
            Err(RetryError::Fatal(e)) => return Err(e),
            Err(RetryError::Retryable { source, delay }) => {
                attempt += 1;
                if attempt >= policy.max_attempts {
                    return Err(source);
                }
                std::thread::sleep(delay);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn policy() -> RetryPolicy {
        RetryPolicy::default()
    }

    #[test]
    fn exponential_backoff_doubles_to_the_60s_cap() {
        let p = policy();
        assert_eq!(p.exponential_delay_ms(0), 2_000);
        assert_eq!(p.exponential_delay_ms(1), 4_000);
        assert_eq!(p.exponential_delay_ms(2), 8_000);
        assert_eq!(p.exponential_delay_ms(3), 16_000);
        assert_eq!(p.exponential_delay_ms(4), 32_000);
        // 2s * 2^5 = 64s -> capped at 60s.
        assert_eq!(p.exponential_delay_ms(5), 60_000);
        assert_eq!(p.exponential_delay_ms(30), 60_000);
    }

    #[test]
    fn backoff_delay_stays_within_jitter_bounds_and_cap() {
        let p = policy();
        for attempt in 0..8 {
            let d = p.backoff_delay(attempt);
            let base = p.exponential_delay_ms(attempt);
            // Jitter adds 0..=25% of base, capped at max.
            assert!(d.as_millis() as u64 >= base, "attempt {} below base", attempt);
            assert!(
                d.as_millis() as u64 <= base.saturating_add(base / 4).min(60_000),
                "attempt {} above base+jitter",
                attempt
            );
            assert!(d <= Duration::from_millis(60_000), "attempt {} above cap", attempt);
        }
    }

    #[test]
    fn retry_after_seconds_header_is_honoured_and_capped() {
        let p = policy();
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_static("7"));
        assert_eq!(p.retry_after(&headers), Some(Duration::from_secs(7)));
        // Absent -> None.
        assert_eq!(p.retry_after(&HeaderMap::new()), None);
        // Unparseable (HTTP-date form) -> None, caller falls back to backoff.
        let mut bad = HeaderMap::new();
        bad.insert("retry-after", HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"));
        assert_eq!(p.retry_after(&bad), None);
        // Huge value -> capped at 60s.
        let mut huge = HeaderMap::new();
        huge.insert("retry-after", HeaderValue::from_static("3600"));
        assert_eq!(p.retry_after(&huge), Some(Duration::from_secs(60)));
    }

    #[test]
    fn delay_for_response_prefers_retry_after_over_backoff() {
        let p = policy();
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_static("3"));
        assert_eq!(
            p.delay_for_response(2, &headers),
            Duration::from_secs(3),
            "Retry-After wins over the 8s backoff for attempt 2"
        );
        // No header -> the backoff for attempt 2. backoff_delay jitters,
        // so compare bounds, not an exact value (two calls jitter
        // differently): base 8000ms + 0..=25% jitter.
        let d = p.delay_for_response(2, &HeaderMap::new());
        assert!(d >= Duration::from_millis(8_000), "below base: {:?}", d);
        assert!(d <= Duration::from_millis(10_000), "above base+jitter: {:?}", d);
    }

    #[test]
    fn retryable_statuses_are_429_503_and_5xx_only() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::GATEWAY_TIMEOUT));
        // Every other 4xx fails fast.
        for code in [400u16, 401, 403, 404, 405, 409, 422] {
            let s = StatusCode::from_u16(code).unwrap();
            assert!(!is_retryable_status(s), "{} must fail fast", code);
        }
    }

    #[tokio::test]
    async fn driver_retries_then_succeeds_and_stops_on_fatal() {
        // Succeeds on the 3rd try: 2 retries, then Ok.
        let p = RetryPolicy {
            max_attempts: 5,
            base_delay_ms: 1,
            max_delay_ms: 10,
        };
        let mut calls = 0;
        let out = retry_with_backoff(&p, |attempt| {
            calls += 1;
            let delay = p.backoff_delay(attempt);
            async move {
                if calls < 3 {
                    Err(RetryError::Retryable {
                        source: "boom".to_string(),
                        delay,
                    })
                } else {
                    Ok("done")
                }
            }
        })
        .await;
        assert_eq!(out, Ok("done"));
        assert_eq!(calls, 3);

        // Fatal on the first try: no retry at all.
        let mut fatal_calls = 0;
        let out: Result<&str, String> = retry_with_backoff(&p, |_attempt| {
            fatal_calls += 1;
            async move { Err(RetryError::Fatal("nope".to_string())) }
        })
        .await;
        assert_eq!(out, Err("nope".to_string()));
        assert_eq!(fatal_calls, 1);

        // Always retryable: exhausts the budget and returns the last error.
        let mut exhausted_calls = 0;
        let out: Result<&str, String> = retry_with_backoff(&p, |attempt| {
            exhausted_calls += 1;
            let delay = p.backoff_delay(attempt);
            async move {
                Err(RetryError::Retryable {
                    source: "still down".to_string(),
                    delay,
                })
            }
        })
        .await;
        assert_eq!(out, Err("still down".to_string()));
        assert_eq!(exhausted_calls, 5, "max_attempts total tries");
    }

    #[test]
    fn sync_driver_retries_then_succeeds_and_stops_on_fatal() {
        let p = RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 1,
            max_delay_ms: 10,
        };
        let mut calls = 0;
        let out = retry_with_backoff_sync(&p, |attempt| {
            calls += 1;
            let delay = p.backoff_delay(attempt);
            if calls < 3 {
                Err(RetryError::Retryable {
                    source: "boom".to_string(),
                    delay,
                })
            } else {
                Ok("done")
            }
        });
        assert_eq!(out, Ok("done"));
        assert_eq!(calls, 3);

        let mut fatal_calls = 0;
        let out: Result<&str, String> = retry_with_backoff_sync(&p, |_attempt| {
            fatal_calls += 1;
            Err(RetryError::Fatal("nope".to_string()))
        });
        assert_eq!(out, Err("nope".to_string()));
        assert_eq!(fatal_calls, 1);
    }
}
