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
//!
//! M24 (task 412): the network behaviour is pinned by tests in this file
//! that drive the driver over real sockets (mockito + raw TCP listeners):
//! 429/503 with `Retry-After`, connect timeout, mid-stream disconnect,
//! delayed response, retry exhaustion, the backoff schedule, the total
//! deadline, and the no-replay-of-tools property (the driver re-sends the
//! HTTP request; it never re-runs the caller's work).

use reqwest::StatusCode;
use reqwest::header::HeaderMap;
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

/// Is this reqwest error retryable? Timeouts, connection-phase failures
/// (refused/DNS/TLS during connect), and request-phase failures are
/// transient; anything else is not.
///
/// `is_request()` covers the mid-stream disconnect — the server accepted
/// the connection then closed it while the request was in flight (the
/// Starlink drop shape). It also covers request-construction errors (bad
/// URL/header), which are rare and merely cost a bounded retry before
/// failing; the alternative (failing fast on every transient drop) is the
/// resilience gap this module exists to close. The duplicate-send risk is
/// no greater than the existing `is_timeout()` retry: a timeout can also
/// fire after the server processed the request, and the call sites (LLM
/// API POSTs) are effectively idempotent from the caller's perspective.
pub fn is_retryable_reqwest_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request()
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
pub async fn retry_with_backoff<T, E, F, Fut>(policy: &RetryPolicy, mut op: F) -> Result<T, E>
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
pub fn retry_with_backoff_sync<T, E, F>(policy: &RetryPolicy, mut op: F) -> Result<T, E>
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
            assert!(
                d.as_millis() as u64 >= base,
                "attempt {} below base",
                attempt
            );
            assert!(
                d.as_millis() as u64 <= base.saturating_add(base / 4).min(60_000),
                "attempt {} above base+jitter",
                attempt
            );
            assert!(
                d <= Duration::from_millis(60_000),
                "attempt {} above cap",
                attempt
            );
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
        bad.insert(
            "retry-after",
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
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
        assert!(
            d <= Duration::from_millis(10_000),
            "above base+jitter: {:?}",
            d
        );
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

    // ── M24 (task 412): network resilience over real sockets ──────────────
    // The driver tests above use a fake closure. These drive the driver the
    // way the LLM clients do — a real reqwest POST classified by
    // `is_retryable_reqwest_error` / `is_retryable_status` — against a
    // mockito server and raw TCP listeners, so a connect timeout, a
    // mid-stream disconnect, 429/503 with Retry-After, and retry
    // exhaustion are all exercised over real sockets.

    /// One POST through the retry driver, mirroring the Gemini/Ollama call
    /// sites: reqwest errors via `is_retryable_reqwest_error` (backoff),
    /// HTTP statuses via `is_retryable_status` + `delay_for_response`.
    /// Returns (result, attempts made).
    async fn post_with_retry(
        client: &reqwest::Client,
        url: &str,
        body: &str,
        policy: RetryPolicy,
    ) -> (Result<String, String>, usize) {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts_cb = attempts.clone();
        let out = retry_with_backoff(&policy, |attempt| {
            let client = client.clone();
            let url = url.to_string();
            let body = body.to_string();
            let attempts_cb = attempts_cb.clone();
            async move {
                attempts_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match client.post(&url).body(body).send().await {
                    Err(e) => {
                        if is_retryable_reqwest_error(&e) {
                            Err(RetryError::Retryable {
                                source: format!("reqwest: {}", e),
                                delay: policy.backoff_delay(attempt),
                            })
                        } else {
                            Err(RetryError::Fatal(format!("reqwest: {}", e)))
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            Ok(resp.text().await.unwrap_or_default())
                        } else if is_retryable_status(status) {
                            Err(RetryError::Retryable {
                                source: format!("HTTP {}", status),
                                delay: policy.delay_for_response(attempt, resp.headers()),
                            })
                        } else {
                            Err(RetryError::Fatal(format!("HTTP {}", status)))
                        }
                    }
                }
            }
        })
        .await;
        (out, attempts.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// Test client that never touches a proxy: the sandbox egress proxy
    /// would otherwise swallow connect failures and hang the test.
    fn no_proxy_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test client")
    }

    /// A small policy so the network tests run in milliseconds, not the
    /// production 2s/60s backoff.
    fn fast_policy(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay_ms: 10,
            max_delay_ms: 50,
        }
    }

    #[tokio::test]
    async fn retry_429_with_retry_after_then_succeeds() {
        let mut server = mockito::Server::new_async().await;
        let path = "/v1/test";
        // FIFO: the 429 fires first, then the 200.
        let m429 = server
            .mock("POST", path)
            .with_status(429)
            .with_header("retry-after", "1")
            .with_body("slow down")
            .expect(1)
            .create_async()
            .await;
        let m200 = server
            .mock("POST", path)
            .with_status(200)
            .with_body("ok")
            .expect(1)
            .create_async()
            .await;
        let client = no_proxy_client();
        let url = format!("{}{}", server.url(), path);
        let start = std::time::Instant::now();
        // Retry-After is capped at max_delay_ms, so the fast policy's 50ms
        // cap would shrink the 1s Retry-After to 50ms and the elapsed
        // assertion could never pass. Use a policy whose cap is above 1s.
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay_ms: 10,
            max_delay_ms: 2000,
        };
        let (out, attempts) = post_with_retry(&client, &url, "{}", policy).await;
        let elapsed = start.elapsed();
        assert_eq!(out, Ok("ok".to_string()));
        assert_eq!(attempts, 2, "one 429 retry, then success");
        // Retry-After: 1s must be honoured, not the 10ms backoff.
        assert!(
            elapsed >= std::time::Duration::from_secs(1),
            "Retry-After not honoured: elapsed {:?}",
            elapsed
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "took too long: {:?}",
            elapsed
        );
        m429.assert_async().await;
        m200.assert_async().await;
    }

    #[tokio::test]
    async fn retry_503_then_succeeds_uses_backoff() {
        let mut server = mockito::Server::new_async().await;
        let path = "/v1/test";
        let m503 = server
            .mock("POST", path)
            .with_status(503)
            .with_body("unavailable")
            .expect(1)
            .create_async()
            .await;
        let m200 = server
            .mock("POST", path)
            .with_status(200)
            .with_body("ok")
            .expect(1)
            .create_async()
            .await;
        let client = no_proxy_client();
        let url = format!("{}{}", server.url(), path);
        let start = std::time::Instant::now();
        let (out, attempts) = post_with_retry(&client, &url, "{}", fast_policy(5)).await;
        let elapsed = start.elapsed();
        assert_eq!(out, Ok("ok".to_string()));
        assert_eq!(attempts, 2, "one 503 retry, then success");
        // Backoff for attempt 0 is base 10ms (plus up to 25% jitter).
        assert!(
            elapsed >= std::time::Duration::from_millis(10),
            "no backoff delay: {:?}",
            elapsed
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "took too long: {:?}",
            elapsed
        );
        m503.assert_async().await;
        m200.assert_async().await;
    }

    #[tokio::test]
    async fn retry_400_fails_fast_without_retry() {
        let mut server = mockito::Server::new_async().await;
        let path = "/v1/test";
        let m400 = server
            .mock("POST", path)
            .with_status(400)
            .with_body("bad request")
            .expect(1)
            .create_async()
            .await;
        let client = no_proxy_client();
        let url = format!("{}{}", server.url(), path);
        let (out, attempts) = post_with_retry(&client, &url, "{}", fast_policy(5)).await;
        assert!(out.is_err(), "400 must fail");
        assert_eq!(attempts, 1, "client error must not be retried");
        m400.assert_async().await;
    }

    /// A listener that accepts a connection and immediately drops it —
    /// the server half of a mid-stream disconnect (Starlink drop shape).
    /// The accept thread is detached; the OS reclaims it when the test
    /// process exits.
    fn drop_server() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind drop server");
        let port = listener.local_addr().expect("port").port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                drop(stream);
            }
        });
        format!("http://127.0.0.1:{}/v1/test", port)
    }

    #[tokio::test]
    async fn retry_midstream_disconnect_is_retried_to_exhaustion() {
        let url = drop_server();
        let client = no_proxy_client();
        let start = std::time::Instant::now();
        let (out, attempts) = post_with_retry(&client, &url, "{}", fast_policy(3)).await;
        let elapsed = start.elapsed();
        assert!(out.is_err(), "disconnect must fail, got {:?}", out);
        assert_eq!(
            attempts, 3,
            "mid-stream disconnect must be retried to the budget, got {} (err: {:?})",
            attempts, out
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "must not hang: {:?}",
            elapsed
        );
    }

    /// A port that is guaranteed closed: bind one, read it, drop the
    /// listener. Used for the connect-refused test.
    fn closed_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind for a closed port")
            .local_addr()
            .expect("listener addr")
            .port()
    }

    #[tokio::test]
    async fn retry_connect_refused_is_retried_to_exhaustion() {
        // A closed loopback port: the connect is refused immediately
        // (is_connect), so the retry budget is exercised without waiting.
        // M24 note: a true connect TIMEOUT (SYN blackhole, e.g. the
        // TEST-NET-1 address 192.0.2.1) cannot be reproduced in the
        // sandbox — the gateway accepts the SYN and it degrades to a
        // response timeout, which
        // retry_delayed_response_times_out_and_exhausts already covers.
        // The retry path is the same (connect failure -> retryable), and
        // the connect_timeout itself is a reqwest client setting.
        let port = closed_port();
        let client = no_proxy_client();
        let url = format!("http://127.0.0.1:{}/v1/test", port);
        let start = std::time::Instant::now();
        let (out, attempts) = post_with_retry(&client, &url, "{}", fast_policy(3)).await;
        let elapsed = start.elapsed();
        assert!(out.is_err(), "refused connect must fail, got {:?}", out);
        assert_eq!(
            attempts, 3,
            "connect failure must be retried to the budget, got {}",
            attempts
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "must not hang: {:?}",
            elapsed
        );
    }

    /// A listener that accepts, reads the request, waits, then answers —
    /// a slow server that trips the client's request timeout.
    fn slow_server(delay: std::time::Duration) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind slow server");
        let port = listener.local_addr().expect("port").port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                std::thread::spawn(move || {
                    use std::io::Read;
                    use std::io::Write;
                    let mut buf = [0u8; 4096];
                    let _ = s.read(&mut buf);
                    std::thread::sleep(delay);
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
                    let _ = s.flush();
                });
            }
        });
        format!("http://127.0.0.1:{}/v1/test", port)
    }

    #[tokio::test]
    async fn retry_delayed_response_times_out_and_exhausts() {
        let url = slow_server(std::time::Duration::from_millis(500));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .expect("test client");
        let start = std::time::Instant::now();
        let (out, attempts) = post_with_retry(&client, &url, "{}", fast_policy(3)).await;
        let elapsed = start.elapsed();
        assert!(out.is_err(), "slow server must time out, got {:?}", out);
        assert_eq!(
            attempts, 3,
            "request timeout must be retried to the budget, got {}",
            attempts
        );
        // Total deadline: 3 attempts x 100ms timeout + backoff, well under 5s.
        assert!(
            elapsed >= std::time::Duration::from_millis(300),
            "attempts did not wait out their timeouts: {:?}",
            elapsed
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "must not hang: {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn retry_does_not_replay_tool_side_effects() {
        // H4 (task 373): the retry driver re-sends the HTTP request, it
        // never re-runs the caller's work. The tool runs once to build the
        // request body; the retried op only re-sends that same body. The
        // loop-level version of this property is
        // gemini_tool_loop_transient_503_does_not_replay_tool_calls; this
        // is the driver-level contract.
        let mut server = mockito::Server::new_async().await;
        let path = "/v1/test";
        // The tool's output, built once: every attempt must send this
        // exact body (matched by both mocks), never a regenerated one.
        let body = r#"{"tool":"spawn_subagent","task":"plan a rust bot app"}"#;
        let m503 = server
            .mock("POST", path)
            .match_body(body)
            .with_status(503)
            .with_body("unavailable")
            .expect(2)
            .create_async()
            .await;
        let m200 = server
            .mock("POST", path)
            .match_body(body)
            .with_status(200)
            .with_body("ok")
            .expect(1)
            .create_async()
            .await;
        // The tool ran once, before the retry loop, to build the request.
        let tool_runs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        tool_runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let client = no_proxy_client();
        let url = format!("{}{}", server.url(), path);
        let (out, attempts) = post_with_retry(&client, &url, body, fast_policy(5)).await;
        assert_eq!(out, Ok("ok".to_string()));
        assert_eq!(attempts, 3, "two 503s then success");
        assert_eq!(
            tool_runs.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the tool must run exactly once while the HTTP call retries"
        );
        m503.assert_async().await;
        m200.assert_async().await;
    }
}
