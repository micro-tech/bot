//! Gemini LLM handler for Helix
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

const GEMINI_MODEL_DEFAULT: &str = "gemini-3.6-flash";
const GEMINI_URL_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

// ── helpers ───────────────────────────────────────────────────────────────────

fn build_client() -> Client {
    // reqwest 0.13 with `rustls-no-provider` bundles Mozilla's root CA store
    // automatically, so no manual cert loading is needed.  The global ring
    // crypto provider is installed once at startup in main.rs.
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
        // Task 221: agentic tool loop (function calling) instead of the old
        // single-shot call — this is what lets the commander delegate.
        match call_gemini_tools(&client, &url, &prompt, bus).await {
            Ok(response) => {
                info!(
                    "Gemini replied on attempt {}/{} -- {} chars",
                    attempt,
                    MAX_RETRIES,
                    response.len()
                );

                // Task 223: record the completed turn for conversation history.
                crate::chat_history::record_turn(&prompt, &response);

                // Forward ONLY to CPU as llm_response (single source of truth)
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

                // REMOVED: direct publish to web_interface to avoid duplicate replies.
                // The CPU forwarder in main.rs will convert llm_response → llm_output.

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
    // Task 223: keep the failed turn so "try again" still resolves against
    // the original request.
    crate::chat_history::record_turn(&prompt, "");
    publish_error(bus, &err);
}

// ── HTTP call ─────────────────────────────────────────────────────────────────

async fn call_gemini(
    client: &Client,
    url: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Task 215: commander system prompt via Gemini's system_instruction.
    let mut body = json!({
        "contents": [{"parts": [{"text": prompt}]}]
    });
    if crate::tools::subagent_tool::commander_enabled() {
        body["system_instruction"] = json!({
            "parts": [{"text": crate::tools::subagent_tool::COMMANDER_SYSTEM_PROMPT}]
        });
    }

    let resp = client.post(url).json(&body).send().await.map_err(
        map_reqwest_err,
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

// ── Tool-calling loop (task 221) ─────────────────────────────────────────────

/// Max tool rounds per chat turn (same guardrail style as the Ollama loop).
const MAX_GEMINI_TOOL_ROUNDS: usize = 10;
/// Max chars of a tool result fed back to the model / shown in the UI preview.
const MAX_GEMINI_TOOL_RESULT_CHARS: usize = 8_000;

/// Convert the shared tool definitions (Ollama/OpenAI shape) into Gemini
/// `functionDeclarations`. Task 221.1.
fn gemini_function_declarations() -> Value {
    let defs = crate::tools::tool_definitions();
    let decls: Vec<Value> = defs
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|d| {
            let f = d.get("function")?;
            let name = f.get("name")?.as_str()?;
            if name.is_empty() {
                return None;
            }
            let mut decl = json!({
                "name": name,
                "description": f.get("description")?.as_str().unwrap_or(""),
            });
            if let Some(params) = f.get("parameters") {
                decl["parameters"] = params.clone();
            }
            Some(decl)
        })
        .collect();
    json!(decls)
}

/// Extract (name, args) pairs from a Gemini response's `parts` array.
fn gemini_function_calls(parts: &Value) -> Vec<(String, Value)> {
    parts
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            let fc = p.get("functionCall")?;
            let name = fc.get("name")?.as_str()?.to_string();
            let args = fc.get("args").cloned().unwrap_or(json!({}));
            Some((name, if args.is_object() { args } else { json!({}) }))
        })
        .collect()
}

/// Concatenate the text parts of a Gemini response.
fn gemini_text_parts(parts: &Value) -> String {
    parts
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.get("text")?.as_str().map(|s| s.to_string()))
        .collect::<Vec<_>>()
        .join("")
}

/// Agentic tool-calling loop for the Gemini backend (task 221).
///
/// Mirrors the Ollama backend's loop: declare the tools, execute any
/// `functionCall` parts locally, send `functionResponse` parts back, repeat
/// until the model returns plain text. Every invocation publishes
/// `{type:"tool_call"}` on the bus — the same contract the web UI's
/// delegation cards render, so the commander pattern works on Gemini too.
async fn call_gemini_tools(
    client: &Client,
    url: &str,
    prompt: &str,
    bus: &Arc<Bus>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Task 223: conversation history as proper user/model turns, so
    // follow-ups ("try again") resolve against prior turns. History is
    // always clean pairs (failed turns carry a marker), which satisfies
    // Gemini's user/model role alternation. The in-flight turn is recorded
    // only after its reply, so it never appears here twice.
    let mut contents: Vec<Value> = Vec::new();
    for turn in crate::chat_history::recent_turns() {
        contents.push(json!({"role": "user", "parts": [{"text": turn.user}]}));
        contents.push(json!({"role": "model", "parts": [{"text": turn.assistant}]}));
    }
    contents.push(json!({"role": "user", "parts": [{"text": prompt}]}));
    let mut rounds = 0usize;

    loop {
        let mut body = json!({
            "contents": contents,
            "tools": [{"functionDeclarations": gemini_function_declarations()}],
        });
        // Task 215: commander system prompt via Gemini's system_instruction.
        if crate::tools::subagent_tool::commander_enabled() {
            body["system_instruction"] = json!({
                "parts": [{"text": crate::tools::subagent_tool::COMMANDER_SYSTEM_PROMPT}]
            });
        }

        let resp = client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_err)?;

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
        // Each generateContent round is billable — feed the spend cap (task 217).
        crate::router::note_api_call(&crate::router::LLMBackend::Gemini);

        let parts = &parsed["candidates"][0]["content"]["parts"];
        let calls = gemini_function_calls(parts);
        if calls.is_empty() {
            let text = gemini_text_parts(parts);
            if text.trim().is_empty() {
                return Err("Gemini returned no text and no function calls".into());
            }
            return Ok(text);
        }

        rounds += 1;
        if rounds > MAX_GEMINI_TOOL_ROUNDS {
            return Err("Gemini tool-call loop exceeded safety limit".into());
        }

        // Keep the model's functionCall content in the history, then append
        // our functionResponse parts (role "user" per the Gemini API).
        contents.push(json!({"role": "model", "parts": parts.clone()}));
        let mut response_parts = Vec::with_capacity(calls.len());
        for (name, args) in &calls {
            let result = crate::tools::execute(name, args);
            if result.starts_with("Error") || result.starts_with("Unknown tool") {
                warn!("Tool '{}' execution reported error: {}", name, result);
            }
            info!("Gemini tool called: '{}' → {} chars", name, result.len());

            // Same publish contract as the Ollama path — the UI renders it.
            let _ = bus.publish(Message {
                to: "web_interface".to_string(),
                from: "tool_executor".to_string(),
                data: json!({
                    "type": "tool_call",
                    "tool": name,
                    "args": args,
                    "result_preview": crate::utils::truncate_str(&result, 200),
                })
                .to_string(),
                timestamp: now_ms(),
            });

            response_parts.push(json!({
                "functionResponse": {
                    "name": name,
                    "response": {
                        "result": crate::utils::truncate_str(&result, MAX_GEMINI_TOOL_RESULT_CHARS),
                    },
                }
            }));
        }
        contents.push(json!({"role": "user", "parts": response_parts}));
    }
}

/// Shared reqwest error diagnosis (connection-phase errors need the TLS/DNS/
/// refused distinction; everything else is a plain network error).
fn map_reqwest_err(e: reqwest::Error) -> Box<dyn std::error::Error + Send + Sync> {
    if e.is_timeout() {
        format!("request timed out after {}s: {}", REQUEST_TIMEOUT_SECS, e).into()
    } else if e.is_connect() {
        // reqwest's is_connect() fires for TCP refused, DNS failures AND TLS
        // certificate errors — all occurring in the connection phase.
        // Inspect the error text so the user sees a precise diagnosis
        // instead of a generic "DNS failure" when it is actually a TLS error.
        let detail = e.to_string().to_lowercase();
        if detail.contains("certificate")
            || detail.contains("unknownissuer")
            || detail.contains("invalidcertificate")
            || detail.contains("handshake")
            || detail.contains("tls")
            || detail.contains("ssl")
        {
            format!("TLS/certificate error: {}", e).into()
        } else if detail.contains("dns")
            || detail.contains("resolve")
            || detail.contains("lookup")
        {
            format!("DNS resolution failed: {}", e).into()
        } else {
            format!("connection refused / network error: {}", e).into()
        }
    } else {
        format!("network error: {}", e).into()
    }
}

// ── crate-internal direct call (task 214: spawn_subagent) ───────────────────────

/// Direct async Gemini prompt call for the sub-agent runner.
///
/// Reads `GEMINI_API_KEY` / `GEMINI_MODEL` from the environment, same as the
/// bus path. Single-shot (no tool loop) — the sub-agent gets one strong answer.
pub(crate) async fn call_gemini_direct(prompt: &str) -> Result<String, String> {
    let api_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        return Err("GEMINI_API_KEY is not set — cannot run a sub-agent on Gemini.                     Set it in the environment or .env, or pick a local backend."
            .to_string());
    }
    let model = std::env::var("GEMINI_MODEL")
        .ok()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| GEMINI_MODEL_DEFAULT.to_string());
    let url = format!("{}/{}:generateContent?key={}", GEMINI_URL_BASE, model, api_key);
    let client = build_client();
    // Count the spend BEFORE the call so a hung request still records intent;
    // the hook is cheap and idempotent per call.
    let out = call_gemini(&client, &url, prompt).await;
    match out {
        Ok(text) => {
            crate::router::note_api_call(&crate::router::LLMBackend::Gemini);
            Ok(text)
        }
        Err(e) => Err(e.to_string()),
    }
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

    // ── Task 221: tool-calling loop ──────────────────────────────────────────

    #[test]
    fn gemini_declarations_include_spawn_subagent() {
        // The commander cannot delegate on Gemini unless spawn_subagent is
        // declared. Shape must be Gemini's functionDeclarations form, not
        // the Ollama/OpenAI wrapper.
        let decls = gemini_function_declarations();
        let arr = decls.as_array().expect("array");
        assert!(!arr.is_empty(), "no tools declared at all");
        let spawn = arr
            .iter()
            .find(|d| d["name"] == "spawn_subagent")
            .expect("spawn_subagent must be declared for Gemini");
        assert!(
            !spawn["description"].as_str().unwrap_or("").is_empty(),
            "spawn_subagent needs a description"
        );
        assert!(
            spawn["parameters"].is_object(),
            "spawn_subagent needs a parameters schema"
        );
        assert!(
            spawn.get("function").is_none(),
            "must be Gemini shape, not the Ollama wrapper"
        );
    }

    #[test]
    fn gemini_response_parsing_helpers() {
        let parts = serde_json::json!([
            {"text": "Working on it."},
            {"functionCall": {"name": "spawn_subagent", "args": {"task": "x", "backend": "auto"}}},
            {"functionCall": {"name": "get_beliefs", "args": {}}},
        ]);
        let calls = gemini_function_calls(&parts);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "spawn_subagent");
        assert_eq!(calls[0].1["task"], "x");
        assert_eq!(calls[1].0, "get_beliefs");
        assert_eq!(gemini_text_parts(&parts), "Working on it.");
        // Missing args default to {}.
        let no_args = serde_json::json!([{"functionCall": {"name": "list_tools"}}]);
        assert_eq!(gemini_function_calls(&no_args)[0].1, json!({}));
    }

    // NOTE: multi_thread flavor — tool_definitions() bridges into the MCP
    // runtime via block_in_place, which panics on a current-thread runtime.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gemini_tool_loop_emits_spawn_subagent_call() {
        // Task 221.4: headless drill — the Gemini path must emit a
        // spawn_subagent tool call and publish the tool_call bus message the
        // UI delegation cards render. No real API key, no real sub-agent.
        crate::io::ollama::ensure_crypto_provider();
        let _tl = crate::tools::subagent_tool::test_env_lock();
        // SAFETY: test-scoped env manipulation under the shared test lock.
        unsafe {
            std::env::set_var("HELIX_SUBAGENT_TEST_STUB", "1");
            std::env::set_var(
                "HELIX_SPEND_FILE",
                std::env::temp_dir().join("helix-test-spend-221.json"),
            );
        }

        let mut server = mockito::Server::new_async().await;
        let path = "/v1beta/models/test-model:generateContent";
        // One mock, sequenced responses via a hit counter: turn 1 returns a
        // functionCall, turn 2 returns final text. (mockito matches FIFO, so
        // two same-path mocks can't be ordered reliably.)
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits_cb = hits.clone();
        let mock = server
            .mock("POST", path)
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_chunked_body(move |w: &mut dyn std::io::Write| {
                let n = hits_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let body = if n == 0 {
                    r#"{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"spawn_subagent","args":{"task":"plan a rust bot app","backend":"auto"}}}]}}]}"#
                } else {
                    r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"Delegated. The sub-agent is planning the Rust bot."}]}}]}"#
                };
                w.write_all(body.as_bytes())
            })
            .expect(2)
            .create_async()
            .await;

        let bus = Arc::new(Bus::new());
        let rx = bus.subscribe("web_interface");
        let client = build_client();
        let url = format!("{}{}?key=fake-test-key", server.url(), path);

        let out = call_gemini_tools(&client, &url, "run a sub agent to plan a rust bot", &bus)
            .await
            .expect("tool loop should complete");
        assert!(
            out.contains("Delegated"),
            "expected the model's final text, got: {}",
            out
        );

        mock.assert_async().await;

        // The tool_call bus message is the UI contract (delegation cards).
        let mut saw_spawn = false;
        while let Ok(msg) = rx.try_recv() {
            if let Ok(v) = serde_json::from_str::<Value>(&msg.data) {
                if v["type"] == "tool_call" && v["tool"] == "spawn_subagent" {
                    saw_spawn = true;
                    assert!(
                        v["args"]["task"].as_str().unwrap_or("").contains("rust bot"),
                        "args not forwarded: {}",
                        v["args"]
                    );
                    assert!(v["result_preview"].as_str().unwrap_or("").contains("subagent-test-stub"));
                }
            }
        }
        assert!(saw_spawn, "expected a spawn_subagent tool_call on the bus");

        unsafe {
            std::env::remove_var("HELIX_SUBAGENT_TEST_STUB");
            std::env::remove_var("HELIX_SPEND_FILE");
        }
    }
}
