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
use std::sync::OnceLock;
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
/// Called once to seed the shared client (see [`shared_client`]).
fn build_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .expect("Failed to build reqwest HTTP client")
}

/// Process-wide shared `reqwest::Client`, built once from [`build_client`].
/// `Client::clone()` is cheap (Arc-backed connection pool), so every health
/// check, model check, and chat turn reuses the same pool — keep-alive works
/// and per-turn TCP handshakes are gone.
static SHARED_CLIENT: OnceLock<Client> = OnceLock::new();

/// Return the shared HTTP client, building it once on first use.
pub fn shared_client() -> Client {
    SHARED_CLIENT.get_or_init(build_client).clone()
}

/// Install the rustls ring crypto provider for tests.
///
/// The binary does this in `main()`; the lib test harness does not, and
/// reqwest is built with `rustls-tls-manual-roots-no-provider`, so any test
/// that builds a `reqwest::Client` must call this first.
#[cfg(test)]
pub(crate) fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

// ── health/model probe cache (task 178) ───────────────────────────────────────
// Every chat turn used to cost 2–3 sequential `/api/tags` round trips (health
// check, model-existence check, and a re-fetch on failure) before the real
// work started. One `/api/tags` response carries everything needed for both
// checks, so we probe once and trust the result for PROBE_TTL. The entry is
// invalidated when a real request fails, forcing re-validation on next turn.

/// How long a `/api/tags` probe result is trusted before re-checking.
const PROBE_TTL: Duration = Duration::from_secs(60);

/// Cached outcome of one `/api/tags` probe.
#[derive(Debug, Clone)]
enum OllamaProbe {
    /// Reachable; carries the installed model list.
    Healthy(Vec<String>),
    /// Connection refused / timed out / non-2xx.
    Unreachable,
    /// Probe inconclusive (DNS, parse error…): caller may proceed anyway.
    Unknown(String),
}

static PROBE_CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, (OllamaProbe, std::time::Instant)>>> =
    OnceLock::new();

fn probe_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, (OllamaProbe, std::time::Instant)>> {
    PROBE_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Health + model list for `base_url`, served from a 60s TTL cache.
/// One `/api/tags` round trip populates both — no separate health check needed.
async fn probe_ollama(base_url: &str) -> OllamaProbe {
    if let Ok(cache) = probe_cache().lock() {
        if let Some((probe, at)) = cache.get(base_url) {
            if at.elapsed() < PROBE_TTL {
                return probe.clone();
            }
        }
    }
    let probe = probe_ollama_fresh(base_url).await;
    if let Ok(mut cache) = probe_cache().lock() {
        cache.insert(base_url.to_string(), (probe.clone(), std::time::Instant::now()));
    }
    probe
}

/// Single fresh `/api/tags` probe (no cache).
async fn probe_ollama_fresh(base_url: &str) -> OllamaProbe {
    let client = shared_client();
    let url = format!("{}/api/tags", base_url);
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() || e.is_connect() => return OllamaProbe::Unreachable,
        Err(e) => return OllamaProbe::Unknown(e.to_string()),
    };
    if !resp.status().is_success() {
        return OllamaProbe::Unreachable;
    }
    match resp.json::<Value>().await {
        Ok(json) => {
            let models = json["models"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["name"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            OllamaProbe::Healthy(models)
        }
        Err(e) => OllamaProbe::Unknown(format!("failed to parse /api/tags JSON: {e}")),
    }
}

/// True when an Ollama box at `base_url` looks usable right now.
/// Served from the 60s probe cache (no extra round trip in the common case).
/// `Unknown` counts as usable — the caller may proceed anyway; only a hard
/// `Unreachable` (refused/timeout/non-2xx) counts as down.
pub async fn ollama_reachable(base_url: &str) -> bool {
    match probe_ollama(base_url).await {
        OllamaProbe::Unreachable => false,
        _ => true,
    }
}

/// Drop the cached probe for `base_url` so the next turn re-validates.
/// Called when a real request fails — the cache must not mask an outage.
pub fn invalidate_ollama_probe(base_url: &str) {
    if let Ok(mut cache) = probe_cache().lock() {
        cache.remove(base_url);
    }
}

/// Does `model` match an entry of the installed-model list?
/// (Exact match, or base-name match: `"llama3.2"` matches `"llama3.2:latest"`.)
fn model_in_list(models: &[String], model: &str) -> bool {
    let model_base = model.split(':').next().unwrap_or(model);
    models
        .iter()
        .any(|a| a == model || a.split(':').next().unwrap_or(a) == model_base)
}

// ── health check ──────────────────────────────────────────────────────────────

/// Ping `{base_url}/api/tags` to verify that Ollama is alive and responding.
///
/// Returns `true` when the endpoint replies with a 2xx status code.
/// Served from a 60s TTL probe cache (see [`probe_ollama`]): consecutive calls
/// within the window cost no extra round trip. Use [`invalidate_ollama_probe`]
/// to force re-validation after a real request failure.
///
/// # Example
///
/// ```ignore
/// // (async context required)
/// if check_ollama_health("http://localhost:11434").await {
///     println!("Ollama is up!");
/// }
/// ```
pub async fn check_ollama_health(base_url: &str) -> bool {
    match probe_ollama(base_url).await {
        OllamaProbe::Healthy(_) => {
            info!("Ollama health-check ✅  ({}/api/tags reachable)", base_url);
            true
        }
        OllamaProbe::Unreachable => {
            error!("Ollama health-check ❌  unreachable: {}", base_url);
            false
        }
        OllamaProbe::Unknown(e) => {
            warn!("Ollama health-check ⚠️  inconclusive: {}", e);
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
    let client = shared_client();
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

    let client = shared_client();

    // ── 1+2. health check + model validation (single cached probe) ────────────
    // One `/api/tags` round trip (amortized over PROBE_TTL) covers both the
    // reachability check and the model-availability check that used to cost
    // 2–3 sequential round trips per turn.
    println!("[ollama_{}] probing {} ...", backend_name, base_url);
    match probe_ollama(base_url).await {
        OllamaProbe::Healthy(models) => {
            println!("[ollama_{}] Ollama reachable ✅", backend_name);
            if !model_in_list(&models, model) {
                let list = if models.is_empty() {
                    "  (no models found — run `ollama pull <model>` first)".to_string()
                } else {
                    models
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
            info!("Model '{}' is available in Ollama ✅", model);
            println!("[ollama_{}] model '{}' exists ✅", backend_name, model);
        }
        OllamaProbe::Unreachable => {
            let err = format!(
                "Ollama is not reachable at '{}'.\n\
                 Common causes:\n\
                 • Ollama not running on that machine\n\
                 • Firewall blocking the port\n\
                 • Ollama bound only to 127.0.0.1 (default). Fix: set OLLAMA_HOST=0.0.0.0 before starting Ollama, or use `ollama serve --host 0.0.0.0`\n\
                 • Wrong IP in config.toml for this backend",
                base_url
            );
            error!("{}", err);
            publish_error(bus, &err);
            return None;
        }
        OllamaProbe::Unknown(e) => {
            // If we can't even check, log a warning and try anyway.
            warn!(
                "Could not verify Ollama health/model availability ({}). Proceeding — may get a 404.",
                e
            );
            println!(
                "[ollama_{}] probe inconclusive ({}), proceeding anyway",
                backend_name, e
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

    // Task 238: correlation_id up front — token messages need it.
    let correlation_id: u64 = serde_json::from_str::<serde_json::Value>(&message.data)
        .ok()
        .and_then(|v| v["correlation_id"].as_u64())
        .unwrap_or(0);

    // Task 238: stream tokens to the UI as they arrive. The callback
    // publishes lightweight `llm_token` bus messages; the CPU forwarder
    // turns them into `chat_token` WS messages. The final `llm_response`
    // still carries the complete text (unchanged contract).
    let bus_clone = Arc::clone(bus);
    let backend_tag = format!("ollama_{}", backend_name);
    let mut on_token = |token: String| {
        let token_msg = crate::bus::Message {
            to: "cpu".to_string(),
            from: backend_tag.clone(),
            data: serde_json::json!({
                "type": "llm_token",
                "correlation_id": correlation_id,
                "token": token,
            })
            .to_string(),
            timestamp: now_ms(),
        };
        let _ = bus_clone.publish(token_msg);
    };

    for attempt in 1..=MAX_RETRIES {
        match call_ollama(&client, base_url, model, &prompt, bus, Some(&mut on_token)).await {
            // SUCCESS — Ollama returned a valid response
            Ok(response) => {
                info!(
                    "Ollama ✅  replied on attempt {}/{} — {} chars",
                    attempt,
                    MAX_RETRIES,
                    response.len()
                );

                // Task 223: record the completed turn for conversation history.
                crate::chat_history::record_turn(&prompt, &response);

                // correlation_id was extracted up front (task 238) for token messages.

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

                // Publish to CPU (single source of truth for responses)
                println!("[ollama_{}] ✅ SUCCESS from Ollama, publishing llm_response to 'cpu' ({} chars)", backend_name, response.len());
                if let Err(e) = bus.publish(response_msg.clone()) {
                    let error_msg = format!("Ollama {} failed to publish LLM response: {}", backend_name, e);
                    log_to_file(&error_msg);
                    error!("{}", error_msg);
                } else {
                    debug!("Ollama {} published LLM response to CPU", backend_name);
                    log_to_file(&format!("Ollama {} published LLM response to CPU", backend_name));
                }

                // REMOVED: direct publish to web_interface to avoid duplicate replies.
                // The CPU forwarder will handle turning llm_response into llm_output.

                return Some(response);
            }

            Err(e) => {
                last_err = e.to_string();
                warn!(
                    "Ollama attempt {}/{} failed: {}",
                    attempt, MAX_RETRIES, last_err
                );
                println!("[ollama_{}] attempt {}/{} FAILED: {}", backend_name, attempt, MAX_RETRIES, last_err);

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
    // The server may have gone away: drop the cached probe so the next turn
    // re-validates instead of trusting a stale "healthy".
    invalidate_ollama_probe(base_url);
    let err = format!(
        "Ollama failed after {} attempt(s): {}",
        MAX_RETRIES, last_err
    );
    error!("{}", err);
    // Task 223: keep the failed turn so "try again" still resolves against
    // the original request.
    crate::chat_history::record_turn(&prompt, "");
    publish_error(bus, &err);
    None
}

// ── internal HTTP helpers ─────────────────────────────────────────────────────

/// Wrapper that assembles the default tool-list and delegates to the loop.
/// `on_token` enables streaming (task 238); `None` = buffered (sub-agents).
async fn call_ollama(
    client: &Client,
    base_url: &str,
    model: &str,
    prompt: &str,
    bus: &Arc<Bus>,
    mut on_token: Option<&mut dyn FnMut(String)>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let tool_defs = crate::tools::tool_definitions();
    let cb: Option<&mut dyn FnMut(String)> = match &mut on_token {
        Some(f) => Some(&mut **f),
        None => None,
    };
    tools::call_ollama_tools(client, base_url, model, prompt, tool_defs, bus, cb).await
}

// ── Task 238: streaming NDJSON support ─────────────────────────────────────

/// Parse one NDJSON line from Ollama's streaming `/api/chat`.
/// Returns `(token_text, is_done)`. Empty lines and unparseable lines yield
/// `None` (skipped). The token is `message.content`; `done: true` marks the
/// final line (which also carries `prompt_eval_count`/`eval_count`).
pub fn parse_stream_line(line: &str) -> Option<(String, bool)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let token = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let done = v.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
    Some((token, done))
}

/// Consume a streaming `/api/chat` response, invoking `on_token` for each
/// content token as it arrives. Returns the reassembled single `message`
/// object (same shape as the non-streaming response's `message` field) so
/// the tool loop can inspect `tool_calls`.
///
/// The final `done` line's eval counts are merged into the returned object
/// under the top-level keys Ollama uses (`prompt_eval_count`, `eval_count`),
/// keeping task-236 metering working on the streaming path.
async fn read_streaming_response(
    resp: reqwest::Response,
    on_token: Option<&mut dyn FnMut(String)>,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut content = String::new();
    let mut tool_calls: Option<Value> = None;
    let mut eval_counts = serde_json::Map::new();
    let mut buf = String::new();
    let mut on_token = on_token;

    // Pull chunks with resp.chunk() (no `stream` feature needed) and split
    // NDJSON lines incrementally for time-to-first-token.
    let mut resp = resp;
    loop {
        let chunk = resp.chunk().await.map_err(|e| {
            let e: Box<dyn std::error::Error + Send + Sync> =
                format!("stream read error: {}", e).into();
            e
        })?;
        let Some(bytes) = chunk else { break };
        buf.push_str(&String::from_utf8_lossy(&bytes));
        // Process complete lines, keeping any partial tail buffered.
        while let Some(nl) = buf.find('\n') {
            let line: String = buf.drain(..=nl).collect();
            let Some((token, done)) = parse_stream_line(&line) else {
                continue;
            };
            if !token.is_empty() {
                content.push_str(&token);
                if let Some(cb) = on_token.as_deref_mut() {
                    cb(token);
                }
            }
            if done {
                // Capture eval counts + any tool_calls from the final line.
                if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                    for k in ["prompt_eval_count", "eval_count"] {
                        if let Some(n) = v.get(k) {
                            eval_counts.insert(k.to_string(), n.clone());
                        }
                    }
                    if let Some(tc) = v
                        .get("message")
                        .and_then(|m| m.get("tool_calls"))
                        .cloned()
                    {
                        tool_calls = Some(tc);
                    }
                }
            }
        }
    }
    // Trailing line without a newline terminator.
    if !buf.trim().is_empty() {
        if let Some((token, _)) = parse_stream_line(&buf) {
            if !token.is_empty() {
                content.push_str(&token);
                if let Some(cb) = on_token.as_deref_mut() {
                    cb(token);
                }
            }
        }
    }

    let mut message = serde_json::json!({ "role": "assistant", "content": content });
    if let Some(tc) = tool_calls {
        message["tool_calls"] = tc;
    }
    let mut parsed = serde_json::json!({ "message": message });
    for (k, v) in eval_counts {
        parsed[k] = v;
    }
    Ok(parsed)
}

// ── Tools-related code extracted to submod ─────────────────────────────

pub mod tools {
    use super::*; // Import top-level dependencies

    /// Max characters of a single tool result fed back into the model.
    /// Tool output is unbounded (log tails, greps, scans); the model only
    /// needs the head of it. Char-boundary safe via [`truncate`].
    const MAX_TOOL_RESULT_CHARS: usize = 8_000;

    /// Sliding-window cap on the message history sent to Ollama: the original
    /// user prompt (index 0) plus the most recent turns.
    const MAX_HISTORY_MESSAGES: usize = 21;

    /// Drop the oldest middle messages, always keeping the original user
    /// prompt (index 0) and the most recent `MAX_HISTORY_MESSAGES - 1`.
    ///
    /// A drain can land between an assistant message carrying `tool_calls`
    /// and its `tool`-role result, orphaning one side of the pair. After
    /// draining, drop any leading `tool` messages left without their call —
    /// a result with no call is confusing to the model.
    fn cap_history(messages: &mut Vec<Value>) {
        if messages.len() > MAX_HISTORY_MESSAGES {
            let keep = MAX_HISTORY_MESSAGES - 1;
            messages.drain(1..messages.len() - keep);
            while messages.get(1).and_then(|m| m.get("role")).and_then(|r| r.as_str())
                == Some("tool")
            {
                messages.remove(1);
            }
        }
    }

    /// Agentic tool-calling loop:
    /// - Send the prompt.
    /// - If the model returns `tool_calls`, execute them locally and send results back.
    /// - Repeat until the model returns plain text content.
    ///
    /// Task 238: when `on_token` is `Some`, the `/api/chat` call streams
    /// (NDJSON) and each content token is pushed to the callback as it
    /// arrives — the caller forwards them to the UI for time-to-first-token.
    /// `None` keeps the legacy buffered behavior (sub-agents need the
    /// complete text before proceeding).
    pub async fn call_ollama_tools(
        client: &Client,
        base_url: &str,
        model: &str,
        prompt: &str,
        tools: Value,
        bus: &Arc<Bus>,
        mut on_token: Option<&mut dyn FnMut(String)>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Task 215: commander system prompt — the model needs to know it
        // should delegate via spawn_subagent instead of chatting.
        let mut messages = Vec::new();
        if let Some(sys) = crate::tools::subagent_tool::commander_system_message() {
            messages.push(sys);
        }
        // Conversation history (task 223): recent turns as proper
        // user/assistant messages so follow-ups ("try again") resolve
        // against prior turns. The in-flight turn is recorded only after
        // its reply is produced, so it can never appear here twice.
        for turn in crate::chat_history::recent_turns() {
            messages.push(json!({"role": "user", "content": turn.user}));
            messages.push(json!({"role": "assistant", "content": turn.assistant}));
        }
        messages.push(json!({"role": "user", "content": prompt}));

        // Guard against infinite tool-call loops (e.g. a badly behaved model)
        let max_tool_rounds = 10usize;
        let mut tool_rounds = 0usize;

        // Task 238: streaming enabled once, outside the loop (avoids
        // borrow-across-iterations issues with the callback).
        let stream = on_token.is_some();

        loop {
            // Sliding-window cap: the history would otherwise grow without
            // bound across tool rounds, re-sending everything every turn.
            cap_history(&mut messages);

            let resp = client
                .post(format!("{}/api/chat", base_url))
                .json(&json!({
                    "model":    model,
                    "messages": messages,
                    "tools":    tools,
                    "stream":   stream,
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

            // Task 238: streaming NDJSON vs buffered single-JSON.
            // Reborrow the callback each round (short-lived, no cross-iteration borrow).
            let parsed: Value = if stream {
                let cb: Option<&mut dyn FnMut(String)> = match &mut on_token {
                    Some(f) => Some(&mut **f),
                    None => None,
                };
                read_streaming_response(resp, cb).await?
            } else {
                resp.json().await.map_err(|e| {
                    let e: Box<dyn std::error::Error + Send + Sync> =
                        format!("failed to parse Ollama JSON response: {}", e).into();
                    e
                })?
            };

            // Task 236: real token metering from Ollama eval counts.
            // Local backends are $0, but the counts feed usage stats.
            if let Some(usage) = crate::router::spend::parse_ollama_usage(&parsed) {
                let backend = if base_url.contains("127.0.0.1")
                    || base_url.contains("localhost")
                    || base_url.contains("::1")
                {
                    crate::router::LLMBackend::LocalOllama
                } else {
                    crate::router::LLMBackend::LanOllama
                };
                crate::router::spend::note_api_usage(&backend, usage);
            }

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
                            "result_preview": truncate(&tool_result, 200),
                        })
                        .to_string(),
                        timestamp: now_ms(),
                    });

                    messages.push(json!({
                        "role":         "tool",
                        "tool_call_id": tool["id"],
                        "content":      truncate(&tool_result, MAX_TOOL_RESULT_CHARS),
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

        #[test]
        fn test_cap_history_keeps_prompt_and_recent() {
            let mut msgs: Vec<Value> = (0..50)
                .map(|i| json!({"role": "user", "content": format!("msg {}", i)}))
                .collect();
            msgs[0] = json!({"role": "user", "content": "ORIGINAL PROMPT"});
            cap_history(&mut msgs);
            assert_eq!(msgs.len(), MAX_HISTORY_MESSAGES);
            assert_eq!(msgs[0]["content"], "ORIGINAL PROMPT");
            assert_eq!(msgs[msgs.len() - 1]["content"], "msg 49");
        }

        #[test]
        fn test_cap_history_short_history_untouched() {
            let mut msgs: Vec<Value> = (0..5).map(|i| json!({"content": i})).collect();
            cap_history(&mut msgs);
            assert_eq!(msgs.len(), 5);
        }

        #[test]
        fn test_cap_history_drops_orphaned_tool_messages() {
            // GIVEN a history where the drain would leave a "tool" result at
            // index 1 with its assistant tool_call drained away:
            let mut msgs: Vec<Value> = (0..50)
                .map(|i| json!({"role": "user", "content": format!("msg {}", i)}))
                .collect();
            msgs[0] = json!({"role": "user", "content": "ORIGINAL PROMPT"});
            // Index 30 becomes index 1 after the drain (keep = 20).
            msgs[30] = json!({"role": "tool", "content": "orphaned result"});
            // WHEN the history is capped,
            cap_history(&mut msgs);
            // THEN the orphan is gone, the prompt survives, and the window
            // holds at most MAX_HISTORY_MESSAGES.
            assert_eq!(msgs[0]["content"], "ORIGINAL PROMPT");
            assert_ne!(msgs[1]["role"], "tool", "orphaned tool result survived");
            assert!(msgs.len() <= MAX_HISTORY_MESSAGES);
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
    crate::utils::truncate_str(s, max)
}

// ── tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    // ── Task 238: NDJSON stream parser ────────────────────────────────────

    #[test]
    fn stream_line_yields_token_in_order() {
        // GIVEN three NDJSON lines (two tokens + done):
        let lines = [
            r#"{"model":"m","created_at":"t","message":{"role":"assistant","content":"Hello"},"done":false}"#,
            r#"{"model":"m","created_at":"t","message":{"role":"assistant","content":" world"},"done":false}"#,
            r#"{"model":"m","created_at":"t","message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":10,"eval_count":5}"#,
        ];
        // WHEN parsed in order,
        let mut tokens = Vec::new();
        let mut dones = Vec::new();
        for line in lines {
            let (token, done) = parse_stream_line(line).expect("line must parse");
            tokens.push(token);
            dones.push(done);
        }
        // THEN tokens arrive in order and only the last line is done:
        assert_eq!(tokens, vec!["Hello", " world", ""]);
        assert_eq!(dones, vec![false, false, true]);
        assert_eq!(tokens.concat(), "Hello world");
    }

    #[test]
    fn stream_line_skips_blanks_and_garbage() {
        assert!(parse_stream_line("").is_none());
        assert!(parse_stream_line("   \n").is_none());
        assert!(parse_stream_line("not json at all").is_none());
        // Valid JSON but no message.content -> empty token, not done.
        let (token, done) = parse_stream_line(r#"{"done":false}"#).unwrap();
        assert_eq!(token, "");
        assert!(!done);
    }

    #[test]
    fn stream_line_extracts_tool_calls_shape() {
        // Tool-call lines carry no content but the message shape survives
        // reassembly (read_streaming_response merges tool_calls at done).
        let line = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"read_log"}}]},"done":false}"#;
        let (token, done) = parse_stream_line(line).unwrap();
        assert_eq!(token, "");
        assert!(!done);
    }

    // ── model discovery ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_fetch_available_models_unreachable() {
        ensure_crypto_provider();
        // Should return an Err, not panic, when Ollama is not running.
        let result = fetch_available_models("http://127.0.0.1:19999").await;
        assert!(result.is_err(), "Expected Err when Ollama is unreachable");
    }

    #[tokio::test]
    async fn test_check_model_exists_unreachable() {
        ensure_crypto_provider();
        // Should return Err (not panic) when Ollama is not reachable.
        let result = check_model_exists("http://127.0.0.1:19999", "llama3.2").await;
        assert!(result.is_err(), "Expected Err when Ollama is unreachable");
    }

    // ── health check ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_health_check_unreachable_port() {
        ensure_crypto_provider();
        // Port 19999 should never be open in a normal test environment.
        let result = check_ollama_health("http://127.0.0.1:19999").await;
        assert!(!result, "Port 19999 should be unreachable");
    }

    #[tokio::test]
    async fn test_health_check_invalid_host() {
        ensure_crypto_provider();
        let result = check_ollama_health("http://this.host.does.not.exist:11434").await;
        assert!(!result, "Non-existent host should fail health check");
    }

    // ── probe cache (task 178) ──────────────────────────────────────────────

    #[test]
    fn test_model_in_list_matching() {
        let models = vec!["llama3.2:latest".to_string(), "qwen3:4b".to_string()];
        assert!(model_in_list(&models, "llama3.2:latest")); // exact
        assert!(model_in_list(&models, "llama3.2")); // base-name match
        assert!(model_in_list(&models, "qwen3:4b"));
        assert!(!model_in_list(&models, "mistral"));
        assert!(!model_in_list(&models, ""));
    }

    #[tokio::test]
    async fn test_probe_cache_unreachable_and_invalidate() {
        ensure_crypto_provider();
        // Dead port → Unreachable; invalidate() must not panic and the
        // re-probe must also report Unreachable.
        let p1 = probe_ollama("http://127.0.0.1:19999").await;
        assert!(matches!(p1, OllamaProbe::Unreachable));
        invalidate_ollama_probe("http://127.0.0.1:19999");
        let p2 = probe_ollama("http://127.0.0.1:19999").await;
        assert!(matches!(p2, OllamaProbe::Unreachable));
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
