//! Ollama Model Preload and Keep-Alive — Task #89
//!
//! # Purpose
//! Ensures the configured Ollama model stays resident in GPU/CPU memory by:
//!
//! 1. **Startup Preload** — sends a dummy `POST /api/generate` (empty prompt,
//!    `stream: false`, `num_predict: 0`) at bot startup so the model is loaded
//!    before the first user message arrives, eliminating the cold-start delay.
//!
//! 2. **Keep-Alive Heartbeat** — a background Tokio task fires a keep-alive
//!    request on a configurable interval (default: 4 min, below Ollama's
//!    5-min unload timeout) so the model is never evicted between user sessions.
//!
//! # Starlink resilience
//! All HTTP calls use `tokio::time::timeout` + exponential-backoff retry
//! (up to 3 attempts, delays 2 s → 4 s → 8 s, capped at 30 s).
//! Failures are logged as warnings — the bot **never panics** on keepalive errors.
//!
//! # Configuration (`.env`)
//! ```env
//! OLLAMA_PRELOAD=true           # set false to skip cold-start preload
//! OLLAMA_KEEP_ALIVE_SECS=240    # heartbeat interval in seconds; 0 = disabled
//! ```

use log::{error, info, warn};
use reqwest::Client;
use std::time::Duration;
use tokio::time::sleep;

// ── tunables ──────────────────────────────────────────────────────────────────

/// Seconds allowed for the TCP connect phase of a preload / keepalive call.
const CONNECT_TIMEOUT_SECS: u64 = 5;

/// Maximum total seconds allowed per HTTP request (preload or keepalive).
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// How many attempts before giving up on a single preload or keepalive ping.
const MAX_RETRIES: u32 = 3;

/// Default keepalive interval — 4 minutes, just below Ollama's default
/// 5-minute model-unload timeout.
pub const DEFAULT_KEEP_ALIVE_SECS: u64 = 240;

/// The `keep_alive` value sent to Ollama so it holds the model for at least
/// one hour regardless of its own eviction setting.
const OLLAMA_KEEP_ALIVE_FIELD: &str = "1h";

// ── HTTP client factory ───────────────────────────────────────────────────────

/// Build a `reqwest::Client` with explicit connect + total-request timeouts.
fn build_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .expect("keepalive: failed to build reqwest HTTP client")
}

// ── public API ────────────────────────────────────────────────────────────────

/// Send a dummy generate request to Ollama so the model is loaded into memory
/// before the first real user message arrives.
///
/// Uses an empty prompt with `stream: false` and `num_predict: 0` (zero tokens
/// generated) — the cheapest valid request that forces a model load.
///
/// Retries up to [`MAX_RETRIES`] times with exponential-backoff delays
/// (2 s → 4 s → 8 s) to tolerate transient Starlink drops.
///
/// # Errors
/// Returns `Err(String)` only if **all** retry attempts fail.
/// Callers should log a warning and continue — a failed preload is not fatal.
///
/// # Example
/// ```no_run
/// if let Err(e) = preload_model("http://localhost:11434", "qwen3.5:0.8b").await {
///     eprintln!("Preload warning: {}", e);
/// }
/// ```
pub async fn preload_model(base_url: &str, model: &str) -> Result<(), String> {
    let client = build_client();
    let url = format!("{}/api/generate", base_url);

    info!(
        "Ollama preload ▶  model='{}' @ {} — warming up…",
        model, base_url
    );

    let mut delay_ms: u64 = 2_000;

    for attempt in 1..=MAX_RETRIES {
        let send_fut = client
            .post(&url)
            .json(&serde_json::json!({
                "model":       model,
                "prompt":      "",
                "stream":      false,
                "num_predict": 0
            }))
            .send();

        let result =
            tokio::time::timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS), send_fut).await;

        match result {
            // ── timed out ─────────────────────────────────────────────────────
            Err(_elapsed) => {
                warn!(
                    "Ollama preload attempt {}/{} timed out after {}s",
                    attempt, MAX_RETRIES, REQUEST_TIMEOUT_SECS
                );
            }

            // ── got an HTTP response ──────────────────────────────────────────
            Ok(Ok(resp)) => {
                let status = resp.status();
                if status.is_success() {
                    info!(
                        "Ollama model preloaded successfully ✅  (attempt {}/{}, HTTP {})",
                        attempt, MAX_RETRIES, status
                    );
                    return Ok(());
                }
                // Non-2xx: log body snippet and retry (might be model loading slowly)
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    "Ollama preload attempt {}/{} — HTTP {}: {}",
                    attempt,
                    MAX_RETRIES,
                    status,
                    &body[..body.len().min(200)]
                );
            }

            // ── network / connection error ─────────────────────────────────────
            Ok(Err(e)) => {
                warn!(
                    "Ollama preload attempt {}/{} — network error: {}",
                    attempt, MAX_RETRIES, e
                );
            }
        }

        if attempt < MAX_RETRIES {
            info!("Ollama preload retrying in {}ms…", delay_ms);
            sleep(Duration::from_millis(delay_ms)).await;
            delay_ms = (delay_ms * 2).min(30_000); // exponential backoff, cap 30 s
        }
    }

    Err(format!(
        "Ollama preload failed after {} attempts — model '{}' may have a cold-start delay on first query",
        MAX_RETRIES, model
    ))
}

/// Spawn a long-lived background Tokio task that sends periodic keep-alive
/// requests to Ollama so the model is never evicted from memory between user
/// sessions.
///
/// Each heartbeat POSTs to `/api/generate` with `keep_alive: "1h"` and an
/// empty prompt.  Ollama resets its eviction timer on receipt.
///
/// Failures are logged as warnings — the task **never panics** and never
/// crashes the bot.  The task runs until the process exits.
///
/// Pass `interval_secs = 0` to disable keepalive without spawning a task.
///
/// # Parameters
/// * `base_url`      — Ollama base URL, e.g. `"http://localhost:11434"`
/// * `model`         — Model name, e.g. `"qwen3.5:0.8b"`
/// * `interval_secs` — Seconds between heartbeats.  `0` disables.
pub fn spawn_keepalive_task(base_url: String, model: String, interval_secs: u64) {
    if interval_secs == 0 {
        info!("Ollama keep-alive heartbeat disabled (OLLAMA_KEEP_ALIVE_SECS=0)");
        return;
    }

    info!(
        "Ollama keep-alive ❤  spawning heartbeat task — model='{}', interval={}s",
        model, interval_secs
    );

    tokio::spawn(async move {
        let client = build_client();
        let url = format!("{}/api/generate", base_url);
        let interval = Duration::from_secs(interval_secs);

        loop {
            sleep(interval).await;
            send_keepalive_ping(&client, &url, &model).await;
        }
    });
}

// ── internal helpers ──────────────────────────────────────────────────────────

/// Fire one keep-alive ping.  Retries up to [`MAX_RETRIES`] times with
/// exponential backoff.  Logs warnings on failure; never panics.
async fn send_keepalive_ping(client: &Client, url: &str, model: &str) {
    let mut delay_ms: u64 = 2_000;

    for attempt in 1..=MAX_RETRIES {
        let send_fut = client
            .post(url)
            .json(&serde_json::json!({
                "model":      model,
                "prompt":     "",
                "stream":     false,
                "keep_alive": OLLAMA_KEEP_ALIVE_FIELD
            }))
            .send();

        let result =
            tokio::time::timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS), send_fut).await;

        match result {
            Err(_) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} timed out",
                    attempt, MAX_RETRIES
                );
            }
            Ok(Ok(resp)) if resp.status().is_success() => {
                info!(
                    "Ollama keep-alive ❤  ping OK (attempt {}/{}, HTTP {})",
                    attempt,
                    MAX_RETRIES,
                    resp.status()
                );
                return; // success — nothing more to do for this interval
            }
            Ok(Ok(resp)) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} — HTTP {}",
                    attempt,
                    MAX_RETRIES,
                    resp.status()
                );
            }
            Ok(Err(e)) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} — network error: {}",
                    attempt, MAX_RETRIES, e
                );
            }
        }

        if attempt < MAX_RETRIES {
            sleep(Duration::from_millis(delay_ms)).await;
            delay_ms = (delay_ms * 2).min(30_000);
        }
    }

    error!(
        "Ollama keep-alive ❌  all {} attempts failed — model '{}' may be unloaded",
        MAX_RETRIES, model
    );
}

// ── config helpers ────────────────────────────────────────────────────────────

/// Read `OLLAMA_PRELOAD` from the environment.
///
/// Returns `true` unless the variable is explicitly set to `"false"` (case-
/// insensitive).  Absent or unparseable values default to `true`.
pub fn read_preload_flag() -> bool {
    std::env::var("OLLAMA_PRELOAD")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        != "false"
}

/// Read `OLLAMA_KEEP_ALIVE_SECS` from the environment.
///
/// Returns [`DEFAULT_KEEP_ALIVE_SECS`] when the variable is absent or cannot
/// be parsed as a `u64`.
pub fn read_keep_alive_secs() -> u64 {
    std::env::var("OLLAMA_KEEP_ALIVE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_KEEP_ALIVE_SECS)
}

// ── unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── config helper tests ───────────────────────────────────────────────────

    #[test]
    fn test_read_preload_flag_defaults_true_when_absent() {
        std::env::remove_var("OLLAMA_PRELOAD");
        assert!(
            read_preload_flag(),
            "Should default to true when OLLAMA_PRELOAD is not set"
        );
    }

    #[test]
    fn test_read_preload_flag_false() {
        std::env::set_var("OLLAMA_PRELOAD", "false");
        assert!(!read_preload_flag());
        std::env::remove_var("OLLAMA_PRELOAD");
    }

    #[test]
    fn test_read_preload_flag_false_uppercase() {
        std::env::set_var("OLLAMA_PRELOAD", "FALSE");
        assert!(!read_preload_flag());
        std::env::remove_var("OLLAMA_PRELOAD");
    }

    #[test]
    fn test_read_preload_flag_true_explicit() {
        std::env::set_var("OLLAMA_PRELOAD", "true");
        assert!(read_preload_flag());
        std::env::remove_var("OLLAMA_PRELOAD");
    }

    #[test]
    fn test_read_preload_flag_garbage_defaults_true() {
        std::env::set_var("OLLAMA_PRELOAD", "yes_please");
        assert!(read_preload_flag(), "Non-'false' values should be treated as true");
        std::env::remove_var("OLLAMA_PRELOAD");
    }

    #[test]
    fn test_read_keep_alive_secs_default_when_absent() {
        std::env::remove_var("OLLAMA_KEEP_ALIVE_SECS");
        assert_eq!(read_keep_alive_secs(), DEFAULT_KEEP_ALIVE_SECS);
    }

    #[test]
    fn test_read_keep_alive_secs_custom_value() {
        std::env::set_var("OLLAMA_KEEP_ALIVE_SECS", "120");
        assert_eq!(read_keep_alive_secs(), 120);
        std::env::remove_var("OLLAMA_KEEP_ALIVE_SECS");
    }

    #[test]
    fn test_read_keep_alive_secs_invalid_falls_back_to_default() {
        std::env::set_var("OLLAMA_KEEP_ALIVE_SECS", "not_a_number");
        assert_eq!(read_keep_alive_secs(), DEFAULT_KEEP_ALIVE_SECS);
        std::env::remove_var("OLLAMA_KEEP_ALIVE_SECS");
    }

    #[test]
    fn test_read_keep_alive_secs_zero_disables() {
        std::env::set_var("OLLAMA_KEEP_ALIVE_SECS", "0");
        assert_eq!(read_keep_alive_secs(), 0);
        std::env::remove_var("OLLAMA_KEEP_ALIVE_SECS");
    }

    // ── network resilience tests (unreachable host → verify no panic / no hang) ─

    #[tokio::test]
    async fn test_preload_model_unreachable_returns_err() {
        // Port 19999 is never open in a normal test environment.
        // Verifies: returns Err, does not panic, does not hang indefinitely.
        let result = preload_model("http://127.0.0.1:19999", "test_model").await;
        assert!(
            result.is_err(),
            "Expected Err when Ollama is unreachable, got Ok"
        );
    }

    #[tokio::test]
    async fn test_spawn_keepalive_task_zero_interval_is_noop() {
        // interval = 0 → returns immediately without spawning — must not panic.
        spawn_keepalive_task(
            "http://127.0.0.1:19999".to_string(),
            "test_model".to_string(),
            0,
        );
    }
}
