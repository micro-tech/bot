//! Gemini LLM handler for Agent OS
//!
//! Provides an async, bus-integrated handler for Google Gemini API calls.
//! Built for reliability on unstable connections (e.g. Starlink) with
//! configurable timeouts and exponential back-off retries.

use crate::bus::{Bus, Message};
use log::{error, info, warn};
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

// ── tunables ──────────────────────────────────────────────────────────────────

const CONNECT_TIMEOUT_SECS: u64 = 10;
const REQUEST_TIMEOUT_SECS: u64 = 60;
const MAX_RETRIES: u32 = 3;
/// Initial delay between retries; doubles each round (exponential back-off).
const RETRY_DELAY_MS: u64 = 2_000;

// ── Gemini API endpoint ───────────────────────────────────────────────────────

const GEMINI_MODEL_DEFAULT: &str = "gemini-2.0-flash";
const GEMINI_URL_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

// ── helpers ───────────────────────────────────────────────────────────────────

fn build_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .expect("Failed to build Gemini reqwest client")
}

fn publish_error(bus: &Arc<Bus>, msg: &str) {
    let _ = bus.publish(Message {
        to: "web_interface".to_string(),
        from: "gemini".to_string(),
        data: json!({"type": "error", "msg": msg}).to_string(),
        timestamp: now_ms(),
    });
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── public bus-facing entry point ─────────────────────────────────────────────

/// Called by the Gemini bus-subscriber loop in `main.rs`.
///
/// `model` is resolved by `main.rs` in priority order:
///   1. `GEMINI_MODEL` env var (set in `.env`)
///   2. `[gemini] model` in `config.toml`
///   3. Built-in default (`gemini-2.0-flash`)
///
/// Reads `GEMINI_API_KEY` from the environment, extracts the `"prompt"` field
/// from `message.data` (falls back to the raw string), calls the Gemini REST
/// API with timeout + retry, and publishes the reply to `"web_interface"`.
pub async fn handle_gemini_bus_message(message: Message, bus: &Arc<Bus>, model: &str) {
    // ── 1. read API key ───────────────────────────────────────────────────────
    let api_key = match std::env::var("GEMINI_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            let err = "GEMINI_API_KEY environment variable is not set or empty";
            error!("{}", err);
            publish_error(bus, err);
            return;
        }
    };

    // ── 2. extract prompt ─────────────────────────────────────────────────────
    let prompt: String = serde_json::from_str::<Value>(&message.data)
        .ok()
        .and_then(|v| v["prompt"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| message.data.clone());

    let correlation_id: u64 = serde_json::from_str::<Value>(&message.data)
        .ok()
        .and_then(|v| v["correlation_id"].as_u64())
        .unwrap_or(0);

    info!(
        "Gemini <- from='{}' prompt='{}'",
        message.from,
        &prompt[..prompt.len().min(120)]
    );

    // ── 3. retry loop ─────────────────────────────────────────────────────────
    let client = build_client();
    let url = format!(
        "{}/{}:generateContent?key={}",
        GEMINI_URL_BASE, model, api_key
    );

    let mut last_err = String::new();
    let mut delay_ms = RETRY_DELAY_MS;

    for attempt in 1..=MAX_RETRIES {
        match call_gemini(&client, &url, &prompt).await {
            Ok(response) => {
                info!(
                    "Gemini replied on attempt {}/{} -- {} chars",
                    attempt,
                    MAX_RETRIES,
                    response.len()
                );

                // Forward to web UI
                let _ = bus.publish(Message {
                    to: "web_interface".to_string(),
                    from: "gemini".to_string(),
                    data: json!({
                        "type": "ollama_response",
                        "llm": "gemini",
                        "correlation_id": correlation_id,
                        "msg": response,
                    })
                    .to_string(),
                    timestamp: now_ms(),
                });

                // Also notify CPU if there is a correlation
                if correlation_id != 0 {
                    let _ = bus.publish(Message {
                        to: "cpu".to_string(),
                        from: "gemini".to_string(),
                        data: json!({
                            "type": "llm_response",
                            "correlation_id": correlation_id,
                            "msg": response,
                        })
                        .to_string(),
                        timestamp: now_ms(),
                    });
                }

                return;
            }
            Err(e) => {
                last_err = e.to_string();
                warn!(
                    "Gemini attempt {}/{} failed: {}",
                    attempt, MAX_RETRIES, last_err
                );

                if attempt < MAX_RETRIES {
                    let _ = bus.publish(Message {
                        to: "web_interface".to_string(),
                        from: "gemini".to_string(),
                        data: json!({
                            "type": "warning",
                            "msg": format!(
                                "Gemini request failed (attempt {}/{}), retrying... ({})",
                                attempt, MAX_RETRIES, last_err
                            ),
                        })
                        .to_string(),
                        timestamp: now_ms(),
                    });

                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    delay_ms *= 2; // exponential back-off
                }
            }
        }
    }

    // ── 4. all retries exhausted ──────────────────────────────────────────────
    let err = format!(
        "Gemini failed after {} attempt(s): {}",
        MAX_RETRIES, last_err
    );
    error!("{}", err);
    publish_error(bus, &err);
}

// ── HTTP call ─────────────────────────────────────────────────────────────────

async fn call_gemini(
    client: &Client,
    url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let body = json!({
        "contents": [{"parts": [{"text": prompt}]}]
    });

    let resp = client.post(url).json(&body).send().await.map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            if e.is_timeout() {
                format!("request timed out after {}s: {}", REQUEST_TIMEOUT_SECS, e).into()
            } else if e.is_connect() {
                format!("connection refused / DNS failure: {}", e).into()
            } else {
                format!("HTTP error: {}", e).into()
            }
        },
    )?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("Gemini returned HTTP {}: {}", status, body_text).into());
    }

    let parsed: Value = resp.json().await.map_err(|e| {
        let e: Box<dyn std::error::Error + Send + Sync> =
            format!("failed to parse Gemini JSON response: {}", e).into();
        e
    })?;

    let text = parsed["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    if text.is_empty() {
        return Err("Gemini returned an empty text field".into());
    }

    Ok(text)
}

// ── legacy sync helper (kept for tests / backward-compat) ────────────────────

/// Synchronous wrapper — kept for existing unit tests.
/// For production use, prefer `handle_gemini_bus_message`.
pub fn call_gemini_sync(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    let api_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        return Err("GEMINI_API_KEY not set".into());
    }
    let rt = tokio::runtime::Runtime::new()?;
    let client = build_client();
    // sync wrapper always reads GEMINI_MODEL env var, falls back to built-in default
    let resolved_model = std::env::var("GEMINI_MODEL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| GEMINI_MODEL_DEFAULT.to_string());
    let url = format!(
        "{}/{}:generateContent?key={}",
        GEMINI_URL_BASE, resolved_model, api_key
    );
    rt.block_on(call_gemini(&client, &url, prompt))
        .map_err(|e| e.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_gemini_sync_no_key() {
        // Without a real API key this should return an Err, not panic.
        // SAFETY: single-threaded test; no other threads read this env var concurrently.
        unsafe { std::env::remove_var("GEMINI_API_KEY") };
        let result = call_gemini_sync("hello");
        assert!(
            result.is_err(),
            "Expected Err when GEMINI_API_KEY is not set"
        );
    }

    #[tokio::test]
    async fn test_handle_gemini_no_key() {
        use crate::bus::Bus;
        unsafe { std::env::remove_var("GEMINI_API_KEY") };
        let bus = Arc::new(Bus::new());
        let _rx = bus.subscribe("web_interface");
        let msg = Message {
            to: "gemini".to_string(),
            from: "test".to_string(),
            data: r#"{"type":"chat_request","prompt":"hello","correlation_id":1}"#.to_string(),
            timestamp: 0,
        };
        // Should not panic — just publish an error to web_interface.
        handle_gemini_bus_message(msg, &bus, "gemini-2.0-flash").await;
    }
}
