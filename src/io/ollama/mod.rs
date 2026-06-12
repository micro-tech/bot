//! Ollama Handler — bus-integrated, with health check, timeout, and retry.
//!
//! # Responsibilities
//! - Verify Ollama is reachable before sending a request  (`check_ollama_health`)
//! - Call `/api/chat` with a configured timeout and automatic retry
//! - Publish the LLM response **back onto the Bus** so other components can act on it
//! - Publish structured error messages to `"web_interface"` when things go wrong
//!
//! # Starlink note
//! The connection can drop at any moment.  Every outbound HTTP request goes
//! through a retry loop with exponential-ish back-off, and every `reqwest`
//! client is built with explicit connect + total-request timeouts so nothing
//! ever hangs silently.

#![allow(dead_code)]

use log::{debug, error};
use log::{info, warn};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use crate::bus::Bus;
use crate::bus::Message;
use crate::utils::log_to_file;
use crate::utils::now_ms;
use serde_json::json;

// OllamaLlm trait (step 1)
pub mod llm_trait;

// OllamaLlm implementation (step 2)
pub mod llm_impl;

// ── OllamaRouter ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OllamaBackend {
    pub url: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct OllamaRouter {
    backends: Vec<OllamaBackend>,
}

impl OllamaRouter {
    pub fn new() -> Self {
        Self { backends: vec![] }
    }

    pub fn add_backend(&mut self, url: String, model: String) {
        self.backends.push(OllamaBackend { url, model });
    }

    pub fn default(&self) -> Option<&OllamaBackend> {
        self.backends.first()
    }
}

// ── tunables ──────────────────────────────────────────────────────────────────

/// Seconds to wait for the TCP connection to be established.
const CONNECT_TIMEOUT_SECS: u64 = 5;

/// Maximum seconds to wait for the full HTTP response once connected.
const REQUEST_TIMEOUT_SECS: u64 = 90;

/// How many times to attempt a request before giving up.
const MAX_RETRIES: u32 = 3;

/// Milliseconds to wait between retry attempts.
const RETRY_DELAY_MS: u64 = 2_500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmTarget {
    OllamaLan,   // your server Ollama
    OllamaLocal, // your 3090 box
    Gemini,
    Grok,
}

// ── client factory ────────────────────────────────────────────────────────────

/// Build a `reqwest::Client` with connection and request timeouts pre-set.
/// Called for every handler invocation so timeouts are always in effect.
fn build_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .expect("Failed to build reqwest HTTP client")
}

// ── health check ──────────────────────────────────────────────────────────────

/// Ping `{base_url}/api/tags` to verify that Ollama is alive and responding.
///
/// Returns `true` when the endpoint replies with a 2xx status code.
/// This is a cheap, fast check — use it before every real request.
///
/// # Example
/// ```no_run
/// if check_ollama_health("http://localhost:11434").await {
///     println!("Ollama is up!");
/// }
/// ```
pub async fn check_ollama_health(base_url: &str) -> bool {
    let client = build_client();
    let url = format!("{}/api/tags", base_url);

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            info!(
                "Ollama health-check ✅  ({}/api/tags → {})",
                base_url,
                resp.status()
            );
            true
        }
        Ok(resp) => {
            warn!(
                "Ollama health-check ⚠️  unexpected HTTP status: {}",
                resp.status()
            );
            false
        }
        Err(e) if e.is_timeout() => {
            error!("Ollama health-check ❌  timed out: {}", e);
            false
        }
        Err(e) if e.is_connect() => {
            error!("Ollama health-check ❌  connection refused/failed: {}", e);
            false
        }
        Err(e) => {
            error!("Ollama health-check ❌  {}", e);
            false
        }
    }
}

// ── model discovery ───────────────────────────────────────────────────────────

/// Query `/api/tags` and return the list of locally-pulled model names.
///
/// Useful for startup diagnostics and for generating a helpful error message
/// when the configured model is not found.
pub async fn fetch_available_models(base_url: &str) -> Result<Vec<String>, String> {
    let client = build_client();
    let url = format!("{}/api/tags", base_url);

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to reach Ollama /api/tags: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Ollama /api/tags returned HTTP {}", resp.status()));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse /api/tags JSON: {}", e))?;

    let names = json["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(names)
}

/// Return `true` if `model` is present in Ollama's local model list.
///
/// Matching rules (in order):
/// 1. Exact name match:  `"qwen3.5:4b"` == `"qwen3.5:4b"`
/// 2. Base-name match:   `"llama3.2"` matches `"llama3.2:latest"`
///
/// Returns `Err` only if the `/api/tags` request itself fails.
pub async fn check_model_exists(base_url: &str, model: &str) -> Result<bool, String> {
    let available = fetch_available_models(base_url).await?;
    let model_base = model.split(':').next().unwrap_or(model);

    let found = available
        .iter()
        .any(|a| a == model || a.split(':').next().unwrap_or(a) == model_base);

    Ok(found)
}

// ── public entry-point ────────────────────────────────────────────────────────

/// Receive a bus `Message`, call Ollama, and publish the response back on the bus.
///
/// Flow:
/// 1. Health-check Ollama — if unreachable, publish an error and return early.
/// 2. Model validation — query `/api/tags`; if the configured model is not
///    installed, publish a clear error listing what IS available and return.
///    This prevents burning the full retry budget on a 404 that will never
///    succeed, and gives the user an actionable fix immediately.
/// 3. Call `/api/chat` with up to `MAX_RETRIES` attempts and a delay between each.
///    A 404 "model not found" mid-flight also aborts the retry loop early.
/// 4. On success:
///    - Publish the LLM text to `message.from`  (reply to original sender).
///    - Publish a structured `ollama_response` JSON to `"web_interface"`.
/// 5. On total failure: publish a structured `error` JSON to `"web_interface"`.
///
/// # Parameters
/// * `message`   — The incoming bus message to process.
/// * `bus`       — Shared bus handle used to publish replies.
/// * `base_url`  — Ollama base URL, e.g. `"http://localhost:11434"`.
/// * `model`     — Model name, e.g. `"llama3.2"`.
pub async fn handle_ollama_message(
    message: Message,
    bus: &Arc<Bus>,
    base_url: &str,
    model: &str,
    backend_name: &str,
) -> Option<String> {
    info!(
        "Ollama ← from='{}' data='{}'",
        message.from,
        truncate(&message.data, 120)
    );

    let client = build_client();

    // ── 1. health check ───────────────────────────────────────────────────────
    if !check_ollama_health(base_url).await {
        let err = format!(
            "Ollama is not reachable at '{}' — request dropped",
            base_url
        );
        error!("{}", err);
        publish_error(bus, &err);
        return None;
    }

    // ── 2. model validation ───────────────────────────────────────────────────
    // Do this BEFORE the retry loop — a missing model always returns 404 and
    // retrying it is pointless.  We give the user a precise, actionable error.
    match check_model_exists(base_url, model).await {
        Ok(true) => {
            info!("Model '{}' is available in Ollama ✅", model);
        }
        Ok(false) => {
            // Fetch the list again to include it in the error message.
            let available = fetch_available_models(base_url).await.unwrap_or_default();
            let list = if available.is_empty() {
                "  (no models found — run `ollama pull <model>` first)".to_string()
            } else {
                available
                    .iter()
                    .map(|m| format!("  • {}", m))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let err = format!(
                "Model '{}' is NOT installed in Ollama.\n\
                 Fix: update config.toml  →  [ollama] model = \"<name>\"\n\
                      or run: ollama pull {}\n\
                 Available models:\n{}",
                model, model, list
            );
            error!("{}", err);
            publish_error(bus, &err);
            return None;
        }
        Err(e) => {
            // If we can't even check, log a warning and try anyway.
            warn!(
                "Could not verify model availability ({}). Proceeding — may get a 404.",
                e
            );
        }
    }

    // ── 3. retry loop ─────────────────────────────────────────────────────────
    let mut last_err = String::new();

    // Extract the actual prompt text from the JSON payload (if any).
    let prompt: String = serde_json::from_str::<serde_json::Value>(&message.data)
        .ok()
        .and_then(|v| v["prompt"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| message.data.clone());

    for attempt in 1..=MAX_RETRIES {
        match call_ollama(&client, base_url, model, &prompt, bus).await {
            // SUCCESS — Ollama returned a valid response
            Ok(response) => {
                info!(
                    "Ollama ✅  replied on attempt {}/{} — {} chars",
                    attempt,
                    MAX_RETRIES,
                    response.len()
                );

                // Parse the ORIGINAL request to extract correlation_id
                let request_payload: serde_json::Value =
                    serde_json::from_str(&message.data).unwrap_or_default();

                let correlation_id = request_payload["correlation_id"].as_u64().unwrap_or(0);

                // Build the correct CPU-bound message
                let response_msg = Message {
                    to: "cpu".to_string(),
                    from: format!("ollama_{}", backend_name),
                    data: serde_json::json!({
                        "type": "llm_response",
                        "correlation_id": correlation_id,
                        "msg": response,
                    })
                    .to_string(),
                    timestamp: now_ms(),
                };

                // Publish to CPU
                if let Err(e) = bus.publish(response_msg.clone()) {
                    let error_msg = format!("Ollama {} failed to publish LLM response: {}", backend_name, e);
                    log_to_file(&error_msg);
                    error!("{}", error_msg);
                } else {
                    debug!("Ollama {} published LLM response to CPU", backend_name);
                    log_to_file(&format!("Ollama {} published LLM response to CPU", backend_name));
                }

                // Also forward to web UI
                let _ = bus.publish(Message {
                    to: "web_interface".to_string(),
                    from: format!("ollama_{}", backend_name),
                    data: json!({
                        "type": "ollama_response",
                        "llm": backend_name,
                        "reply_to": message.from,
                        "msg": response
                    })
                    .to_string(),
                    timestamp: now_ms(),
                });

                return Some(response);
            }

            Err(e) => {
                last_err = e.to_string();
                warn!(
                    "Ollama attempt {}/{} failed: {}",
                    attempt, MAX_RETRIES, last_err
                );

                // A 404 "model not found" will never succeed — abort immediately
                // rather than exhausting all retry attempts uselessly.
                if last_err.contains("404")
                    || (last_err.to_lowercase().contains("model")
                        && last_err.to_lowercase().contains("not found"))
                {
                    error!(
                        "Aborting retries — model '{}' not found in Ollama. \
                         Check config.toml [ollama] model or run `ollama pull {}`.",
                        model, model
                    );
                    break;
                }

                if attempt < MAX_RETRIES {
                    // Publish a transient warning so the UI knows we are retrying
                    let _ = bus.publish(Message {
                        to: "web_interface".to_string(),
                        from: "ollama".to_string(),
                        data: json!({
                            "type": "warning",
                            "msg": format!(
                                "Ollama request failed (attempt {}/{}), retrying… ({})",
                                attempt, MAX_RETRIES, last_err
                            )
                        })
                        .to_string(),
                        timestamp: now_ms(),
                    });

                    tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                }
            }
        }
    }

    // ── 4. all retries exhausted ──────────────────────────────────────────────
    let err = format!(
        "Ollama failed after {} attempt(s): {}",
        MAX_RETRIES, last_err
    );
    error!("{}", err);
    publish_error(bus, &err);
    None
}

// ── internal HTTP helpers ─────────────────────────────────────────────────────

/// Wrapper that assembles the default tool-list and delegates to the loop.
async fn call_ollama(
    client: &Client,
    base_url: &str,
    model: &str,
    prompt: &str,
    bus: &Arc<Bus>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let tool_defs = crate::tools::tool_definitions();
    tools::call_ollama_tools(client, base_url, model, prompt, tool_defs, bus).await
}

// ── Tools-related code extracted to submod ─────────────────────────────

pub mod tools {
    use super::*; // Import top-level dependencies

    /// Agentic tool-calling loop:
    /// - Send the prompt.
    /// - If the model returns `tool_calls`, execute them locally and send results back.
    /// - Repeat until the model returns plain text content.
    pub async fn call_ollama_tools(
        client: &Client,
        base_url: &str,
        model: &str,
        prompt: &str,
        tools: Value,
        bus: &Arc<Bus>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut messages = vec![json!({"role": "user", "content": prompt})];

        // Guard against infinite tool-call loops (e.g. a badly behaved model)
        let max_tool_rounds = 10usize;
        let mut tool_rounds = 0usize;

        loop {
            let resp = client
                .post(format!("{}/api/chat", base_url))
                .json(&json!({
                    "model":    model,
                    "messages": messages,
                    "tools":    tools,
                    "stream":   false   // MUST be false — streaming returns NDJSON,
                                        // not a single JSON object, and breaks parsing.
                }))
                .send()
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    if e.is_timeout() {
                        format!("request timed out after {}s: {}", REQUEST_TIMEOUT_SECS, e).into()
                    } else if e.is_connect() {
                        format!("connection error: {}", e).into()
                    } else {
                        format!("HTTP error: {}", e).into()
                    }
                })?;

            // Surface non-2xx as an error so the retry loop can handle them.
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Ollama returned HTTP {}: {}", status, body).into());
            }

            let parsed: Value = resp.json().await.map_err(|e| {
                let e: Box<dyn std::error::Error + Send + Sync> =
                    format!("failed to parse Ollama JSON response: {}", e).into();
                e
            })?;

            let msg = &parsed["message"];
            messages.push(msg.clone());

            if let Some(tool_calls) = msg["tool_calls"].as_array() {
                tool_rounds += 1;
                if tool_rounds > max_tool_rounds {
                    return Err("Ollama tool-call loop exceeded safety limit".into());
                }

                for tool in tool_calls {
                    let name = tool["function"]["name"].as_str().unwrap_or("");
                    let args = tool["function"]["arguments"].clone();
                    let tool_result = crate::tools::execute(name, &args);

                    // Basic error detection for tool failures
                    if tool_result.starts_with("Error") || tool_result.starts_with("Unknown tool") {
                        warn!("Tool '{}' execution reported error: {}", name, tool_result);
                    }

                    info!("Tool called: '{}' → {} chars", name, tool_result.len());

                    // Notify web UI that a tool was executed
                    let _ = bus.publish(crate::bus::Message {
                        to: "web_interface".to_string(),
                        from: "tool_executor".to_string(),
                        data: serde_json::json!({
                            "type": "tool_call",
                            "tool": name,
                            "args": args,
                            "result_preview": &tool_result[..tool_result.len().min(200)],
                        })
                        .to_string(),
                        timestamp: now_ms(),
                    });

                    messages.push(json!({
                        "role":         "tool",
                        "tool_call_id": tool["id"],
                        "content":      tool_result
                    }));
                }
                // Loop — send tool results back to the model.
            } else {
                // No tool calls → model produced its final answer.
                let content = msg["content"].as_str().unwrap_or("").to_string();
                if content.is_empty() {
                    return Err("Ollama returned an empty content field".into());
                }
                return Ok(content);
            }
        }
    }

    // ── tool registry ─────────────────────────────────────────────────────────────

    /// Returns the JSON array of tools exposed to the model.
    pub fn default_tools() -> Value {
        crate::tools::tool_definitions()
    }

    /// Dispatch a tool call by name, returning the result as a plain string.
    pub fn execute_tool(name: &str, args: Value) -> String {
        crate::tools::execute(name, &args)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn test_execute_tool_read_log_missing() {
            let result = execute_tool("read_log", json!({"log_file": "nonexistent.md"}));
            assert!(result.contains("Error reading"));
        }

        fn test_execute_tool_send_email_placeholder() {
            let result = execute_tool(
                "send_email",
                json!({"to": "test@example.com", "subject": "Hi", "body": "Test"}),
            );
            assert!(result.contains("placeholder"));
        }

        fn test_execute_tool_unknown() {
            let result = execute_tool("unknown_tool", json!({}));
            assert!(result.contains("Unknown tool"));
        }

        fn test_truncate_short_string() {
            assert_eq!(truncate("short", 10), "short");
        }

        fn test_truncate_long_string() {
            let input = "very long string that should be truncated";
            let truncated = truncate(input, 10);
            assert_eq!(truncated, "very long ");
            assert!(truncated.len() <= 10);
        }

        fn test_truncate_unicode_boundary() {
            let input = "a💣b"; // 5 bytes
            let truncated = truncate(input, 4);
            assert_eq!(truncated, "a"); // Safe boundary
        }
    }
}

// ── tiny helpers ──────────────────────────────────────────────────────────────

/// Publish a structured error JSON to `"web_interface"`.
fn publish_error(bus: &Arc<Bus>, msg: &str) {
    let _ = bus.publish(Message {
        to: "web_interface".to_string(),
        from: "ollama".to_string(),
        data: json!({ "type": "error", "msg": msg }).to_string(),
        timestamp: now_ms(),
    });
}

/// Return at most `max` bytes of `s` (no panic on non-ASCII boundaries thanks
/// to `char_indices`).
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    // Walk back from `max` to a valid char boundary.
    let mut boundary = max;
    while !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &s[..boundary]
}

// ── tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    // ── model discovery ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_fetch_available_models_unreachable() {
        // Should return an Err, not panic, when Ollama is not running.
        let result = fetch_available_models("http://127.0.0.1:19999").await;
        assert!(result.is_err(), "Expected Err when Ollama is unreachable");
    }

    #[tokio::test]
    async fn test_check_model_exists_unreachable() {
        // Should return Err (not panic) when Ollama is not reachable.
        let result = check_model_exists("http://127.0.0.1:19999", "llama3.2").await;
        assert!(result.is_err(), "Expected Err when Ollama is unreachable");
    }

    // ── health check ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_health_check_unreachable_port() {
        // Port 19999 should never be open in a normal test environment.
        let result = check_ollama_health("http://127.0.0.1:19999").await;
        assert!(!result, "Port 19999 should be unreachable");
    }

    #[tokio::test]
    async fn test_health_check_invalid_host() {
        let result = check_ollama_health("http://this.host.does.not.exist:11434").await;
        assert!(!result, "Non-existent host should fail health check");
    }

    // ── tool execution ────────────────────────────────────────────────────────

    #[test]
    fn test_execute_tool_read_log_missing() {
        let args = json!({"log_file": "logs/file_that_does_not_exist_xyz.md"});
        let result = tools::execute_tool("read_log", args);
        assert!(
            result.starts_with("Error reading"),
            "Expected an error message, got: {}",
            result
        );
    }

    #[test]
    fn test_execute_tool_send_email_placeholder() {
        let args = json!({
            "to": "test@example.com",
            "subject": "Test",
            "body": "Hello"
        });
        let result = tools::execute_tool("send_email", args);
        assert!(
            result.contains("SMTP") || result.contains("email_outbox") || result.contains("queued"),
            "Expected SMTP/outbox response, got: {}",
            result
        );
    }

    #[test]
    fn test_execute_tool_unknown() {
        let result = tools::execute_tool("totally_unknown_tool", json!({}));
        assert!(
            result.contains("Unknown tool"),
            "Expected 'Unknown tool' in response, got: {}",
            result
        );
    }

    // ── truncate helper ───────────────────────────────────────────────────────

    #[test]
    fn test_truncate_short_string() {
        assert_eq!(truncate("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_long_string() {
        let s = "a".repeat(200);
        let result = truncate(&s, 120);
        assert_eq!(result.len(), 120);
    }

    #[test]
    fn test_truncate_unicode_boundary() {
        // "日本語" is 9 bytes (3 bytes per char).  Truncating at 5 bytes must
        // not panic and must stay at a valid char boundary.
        let s = "日本語test";
        let result = truncate(s, 5);
        assert!(s.is_char_boundary(result.len()));
    }

    // ── bus plumbing ──────────────────────────────────────────────────────────

    #[test]
    fn test_publish_error_does_not_panic() {
        let bus = Arc::new(Bus::new());
        // Subscribe so the router has somewhere to send the message.
        let _rx = bus.subscribe("web_interface");
        publish_error(&bus, "test error — should not panic");
    }

    // ── integration smoke-test (skipped in CI unless Ollama is running) ───────
    //
    // Run with:
    //   cargo test -- --ignored test_handle_ollama_live
    //
    #[tokio::test]
    #[ignore]
    async fn test_handle_ollama_live() {
        let bus = Arc::new(Bus::new());
        let _rx = bus.subscribe("test_sender");
        let _web_rx = bus.subscribe("web_interface");

        let message = Message {
            to: "ollama".to_string(),
            from: "test_sender".to_string(),
            data: "Say hello in exactly three words.".to_string(),
            timestamp: now_ms(),
        };

        let result =
            handle_ollama_message(message, &bus, "http://localhost:11434", "llama3.2", "test")
                .await;

        assert!(result.is_some(), "Expected a response from live Ollama");
        let text = result.unwrap();
        assert!(!text.is_empty(), "Response should not be empty");
        println!("Live Ollama response: {}", text);
    }
}

// ── OllamaLlm struct moved from src/llm/ollama.rs ───────────────────────────

pub mod llm {

    use super::OllamaRouter;
    #[allow(unused_imports)]
    use crate::utils::now_ms;

    #[allow(unused_imports)]
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use reqwest::Client;
    use serde_json::Value;
    use std::sync::Arc;

    // Imports needed for the OllamaLlm implementation
    #[allow(unused_imports)]
    use crate::cpu::interfaces::LlmInterface;
    #[allow(unused_imports)]
    use crate::hy_evo::genome::WorkflowGenome;
    #[allow(unused_imports)]
    use crate::hy_evo::reflection::ReflectionLlm;
    #[allow(unused_imports)]
    use crate::hy_evo::scoring::ExecutionMetrics;

    /// Ollama-backed LLM client.
    /// Implements `ReflectionLlm` so it can drive HyEvo reflection and evolution.
    #[derive(Clone)]
    pub struct OllamaLlm {
        client: Arc<Client>,
        router: Arc<OllamaRouter>,
    }

    impl OllamaLlm {
        pub fn new(router: Arc<OllamaRouter>) -> Self {
            Self {
                client: Arc::new(Client::new()),
                router,
            }
        }

        /// Low-level call to the Ollama `/api/generate` endpoint.
        async fn call_ollama(&self, prompt: &str) -> anyhow::Result<String> {
            let backend = self
                .router
                .default()
                .expect("No Ollama backends configured");

            let url = format!("{}/api/generate", backend.url);
            let model = backend.model.clone();

            let response = self
                .client
                .post(url)
                .json(&serde_json::json!({
                    "model": model,
                    "prompt": prompt,
                    "stream": false
                }))
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Ollama request failed: {}", e))?;

            let json: Value = response
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to parse Ollama response: {}", e))?;

            let result = json["response"].as_str().unwrap_or("").to_string();
            Ok(result)
        }
    }

    #[async_trait]
    impl ReflectionLlm for OllamaLlm {
        /// Reflect on a completed workflow execution.
        /// Sends a structured prompt to Ollama describing the workflow and its metrics.
        async fn reflect(
            &self,
            workflow: &WorkflowGenome,
            metrics: &ExecutionMetrics,
        ) -> anyhow::Result<String> {
            let prompt = format!(
                "You are an AI agent runtime optimizer.\n\
             Reflect on the following workflow execution and suggest improvements.\n\n\
             Workflow ID: {}\n\
             Nodes: {}\n\n\
             Execution Metrics:\n\
             - Latency: {}ms\n\
             - LLM calls: {}\n\
             - Skill calls: {}\n\
             - Memory ops: {}\n\
             - Bus ops: {}\n\
             - Errors: {}\n\
             - Success: {}\n\n\
             Provide a concise reflection and list specific improvements as bullet points starting with '- '.",
                workflow.id,
                workflow.nodes.len(),
                metrics.latency_ms,
                metrics.llm_calls,
                metrics.skill_calls,
                metrics.memory_ops,
                metrics.bus_ops,
                metrics.errors,
                metrics.success,
            );

            self.call_ollama(&prompt).await
        }

        /// Suggest workflow evolution steps given LLM feedback and a genome description.
        async fn evolve_code(&self, feedback: &str, code: &str) -> anyhow::Result<String> {
            let prompt = format!(
                "You are an AI agent workflow optimizer.\n\
             Given the following performance feedback and current workflow description,\n\
             suggest concrete improvements as bullet points starting with '- '.\n\n\
             Feedback:\n{}\n\n\
             Current Workflow:\n{}",
                feedback, code
            );

            self.call_ollama(&prompt).await
        }
    }

    #[async_trait]
    impl LlmInterface for OllamaLlm {
        async fn call(
            &self,
            _model: &str,
            prompt: &str,
            _params: &Value,
        ) -> crate::hy_evo::node::NodeResult {
            match self.call_ollama(prompt).await {
                Ok(response) => crate::hy_evo::node::NodeResult::Text(response),
                Err(e) => crate::hy_evo::node::NodeResult::Error(format!("LLM call failed: {}", e)),
            }
        }

        async fn summarize(&self, text: &str) -> crate::hy_evo::node::NodeResult {
            let prompt = format!("Summarize the following text concisely:\n\n{}", text);
            match self.call_ollama(&prompt).await {
                Ok(summary) => crate::hy_evo::node::NodeResult::Text(summary),
                Err(e) => {
                    crate::hy_evo::node::NodeResult::Error(format!("Summarize failed: {}", e))
                }
            }
        }
    }
}
