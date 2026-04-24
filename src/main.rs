//! Bot — Ollama model pre-warm / keep-alive, optional Discord gateway,
//! and a full HTTP control panel (WebSocket chat, config/manifest editor,
//! live logs).
//!
//! ## Startup sequence
//! 1. Load `.env` via `dotenv`.
//! 2. Initialise `env_logger`.
//! 3. **Ollama preload** — warm the model before the first user message.
//! 4. **Ollama keep-alive** — background task pings Ollama every N seconds.
//! 5. **HTTP control panel** — full chat UI on `STATUS_PORT` (default 8080).
//!    Access via `http://<host>:8080/`  — plain HTTP, no certificate needed.
//! 6. **Discord client** *(optional)* — only when `DISCORD_TOKEN` is set.
//!
//! ## Environment variables (`.env`)
//! | Variable                 | Default                         | Purpose                              |
//! |-------------------------|---------------------------------|--------------------------------------|
//! | `DISCORD_TOKEN`          | *(optional)*                    | Discord bot token — omit to disable  |
//! | `OLLAMA_URL`             | `http://192.168.1.149:11434`    | Ollama base URL                      |
//! | `OLLAMA_MODEL`           | `qwen3.5:0.8b`                  | Model to preload and keep warm       |
//! | `OLLAMA_PRELOAD`         | `true`                          | Set `false` to skip startup preload  |
//! | `OLLAMA_KEEP_ALIVE_SECS` | `240`                           | Heartbeat interval; `0` = disabled   |
//! | `STATUS_PORT`            | `8080`                          | Port for the HTTP control panel      |

use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use serde::Deserialize;
use serde_json::json;
use serenity::async_trait;
use serenity::model::channel::Message as DiscordMessage;
use serenity::model::gateway::Ready;
use serenity::prelude::*;
use tokio::sync::broadcast;
use tower::ServiceExt;

// ── tunables ──────────────────────────────────────────────────────────────────

/// Seconds to wait for a TCP connection to Ollama.
const KA_CONNECT_TIMEOUT_SECS: u64 = 5;

/// Maximum total seconds allowed for a single keepalive HTTP request.
const KA_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Retry attempts before giving up on a preload or keepalive ping.
const KA_MAX_RETRIES: u32 = 3;

/// Default interval between keepalive heartbeats (4 min < Ollama's 5-min timeout).
const KA_DEFAULT_INTERVAL_SECS: u64 = 240;

/// `keep_alive` value sent to Ollama — tells it to hold the model for 1 hour.
const KA_KEEP_ALIVE_FIELD: &str = "1h";

/// Default port for the HTTPS control panel.
const DEFAULT_STATUS_PORT: u16 = 8443;

// ── Config structs (config.toml) ──────────────────────────────────────────────

#[derive(Deserialize, Clone, Default)]
struct TomlConfig {
    #[serde(default)]
    ollama: Vec<OllamaEntry>,
}

#[derive(Deserialize, Clone)]
struct OllamaEntry {
    name: String,
    url: String,
    model: String,
}

/// Read `config.toml` from the current directory.
/// Returns the raw string and the parsed Ollama backends list.
fn load_config() -> (String, Vec<OllamaEntry>) {
    let raw = fs::read_to_string("config.toml").unwrap_or_default();
    let parsed: TomlConfig = toml::from_str(&raw).unwrap_or_default();
    (raw, parsed.ollama)
}

// ── Shared bot state ──────────────────────────────────────────────────────────

/// Thread-safe state shared between the HTTP server, keepalive task, and main.
#[derive(Clone)]
struct BotStatus {
    inner: Arc<Mutex<BotStatusInner>>,
}

struct BotStatusInner {
    started_at: Instant,
    ollama_url: String,
    ollama_model: String,
    discord_enabled: bool,
    last_ping_ok: Option<bool>,
    last_ping_at: Option<Instant>,
    /// Broadcast channel — any task can push a JSON string to all WS clients.
    broadcast_tx: broadcast::Sender<String>,
}

impl BotStatus {
    fn new(ollama_url: String, ollama_model: String, discord_enabled: bool) -> Self {
        let (broadcast_tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Mutex::new(BotStatusInner {
                started_at: Instant::now(),
                ollama_url,
                ollama_model,
                discord_enabled,
                last_ping_ok: None,
                last_ping_at: None,
                broadcast_tx,
            })),
        }
    }

    fn record_ping(&self, ok: bool) {
        let mut g = self.inner.lock().unwrap();
        g.last_ping_ok = Some(ok);
        g.last_ping_at = Some(Instant::now());
    }

    fn subscribe(&self) -> broadcast::Receiver<String> {
        self.inner.lock().unwrap().broadcast_tx.subscribe()
    }
}

// ── Ollama chat ───────────────────────────────────────────────────────────────

/// Send a prompt to Ollama and return the response text.
/// Uses a 120-second timeout to tolerate Starlink drop/reconnect cycles.
/// Retries up to `KA_MAX_RETRIES` times with exponential back-off.
async fn ollama_chat(base_url: &str, model: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(KA_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client build error: {}", e))?;

    let url = format!("{}/api/generate", base_url);
    let payload = json!({
        "model":  model,
        "prompt": prompt,
        "stream": false,
    });

    let mut delay_ms: u64 = 2_000;

    for attempt in 1..=KA_MAX_RETRIES {
        let result = tokio::time::timeout(
            Duration::from_secs(120),
            client.post(&url).json(&payload).send(),
        )
        .await;

        match result {
            Err(_) => {
                warn!(
                    "Ollama chat attempt {}/{} timed out",
                    attempt, KA_MAX_RETRIES
                );
            }
            Ok(Err(e)) => {
                warn!(
                    "Ollama chat attempt {}/{} network error: {}",
                    attempt, KA_MAX_RETRIES, e
                );
            }
            Ok(Ok(resp)) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    warn!(
                        "Ollama chat attempt {}/{} HTTP {}: {}",
                        attempt,
                        KA_MAX_RETRIES,
                        status,
                        &body[..body.len().min(200)]
                    );
                } else {
                    let body: serde_json::Value = resp
                        .json()
                        .await
                        .map_err(|e| format!("Parse error: {}", e))?;
                    return Ok(body["response"]
                        .as_str()
                        .unwrap_or("(empty response)")
                        .to_string());
                }
            }
        }

        if attempt < KA_MAX_RETRIES {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            delay_ms = (delay_ms * 2).min(30_000);
        }
    }

    Err(format!(
        "Ollama chat failed after {} attempts — is the model server running?",
        KA_MAX_RETRIES
    ))
}

// ── HTTP control panel ────────────────────────────────────────────────────────

/// Generate a self-signed TLS certificate into `certs/` if not already present.
/// Uses rcgen with localhost + 127.0.0.1 SANs.  The browser will warn once
/// (self-signed) — click "Advanced → Accept the risk" to proceed.
fn ensure_certs() -> Result<(), Box<dyn std::error::Error>> {
    use rcgen::{generate_simple_self_signed, CertifiedKey};

    fs::create_dir_all("certs")?;
    let cert_path = Path::new("certs/cert.pem");
    let key_path = Path::new("certs/key.pem");

    if !cert_path.exists() || !key_path.exists() {
        info!("Generating self-signed TLS certificate in certs/ …");
        let sans = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        let CertifiedKey { cert, key_pair } = generate_simple_self_signed(sans)?;
        fs::write(cert_path, cert.pem())?;
        fs::write(key_path, key_pair.serialize_pem())?;
        info!("TLS certificate written to certs/cert.pem + certs/key.pem");
    }

    Ok(())
}

/// Load TLS cert + key from `certs/` and build a `tokio_rustls::TlsAcceptor`.
fn build_tls_acceptor() -> Result<tokio_rustls::TlsAcceptor, Box<dyn std::error::Error>> {
    use rustls_pemfile::{certs, private_key};
    use std::sync::Arc;
    use tokio_rustls::rustls::ServerConfig;

    let cert_data = fs::read("certs/cert.pem")?;
    let key_data = fs::read("certs/key.pem")?;

    let cert_chain: Vec<_> = certs(&mut &cert_data[..]).collect::<Result<_, _>>()?;
    let key = private_key(&mut &key_data[..])?.ok_or("no private key found in certs/key.pem")?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;

    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

/// Spawn the axum HTTPS server in the background.
/// Routes: `/` → full control panel, `/ws` → WebSocket, `/health` → plain OK.
fn spawn_status_server(status: BotStatus) {
    let port = env::var("STATUS_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_STATUS_PORT);

    info!("Control panel starting on https://0.0.0.0:{}/", port);

    tokio::spawn(async move {
        // ── TLS setup ─────────────────────────────────────────────────────────
        if let Err(e) = ensure_certs() {
            error!("TLS cert setup failed: {} — control panel not started", e);
            return;
        }

        let tls_acceptor = match build_tls_acceptor() {
            Ok(a) => a,
            Err(e) => {
                error!("TLS acceptor error: {} — control panel not started", e);
                return;
            }
        };

        // ── Axum router ───────────────────────────────────────────────────────
        let app = Router::new()
            .route("/", get(serve_index))
            .route("/ws", get(ws_handler))
            .route("/health", get(health_check))
            .with_state(status);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Control panel bind :{} failed: {}", port, e);
                return;
            }
        };

        info!(
            "Control panel ready — https://192.168.1.168:{}/ \
             (accept the self-signed cert warning on first visit)",
            port
        );

        // ── TLS accept loop ───────────────────────────────────────────────────
        loop {
            let (tcp, _) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    error!("TCP accept: {}", e);
                    continue;
                }
            };

            let acceptor = tls_acceptor.clone();
            let svc = app.clone();

            tokio::spawn(async move {
                let tls_stream = match acceptor.accept(tcp).await {
                    Ok(s) => s,
                    Err(e) => {
                        // port-scanner noise — keep at info level
                        info!("TLS handshake rejected: {}", e);
                        return;
                    }
                };

                let io = hyper_util::rt::TokioIo::new(tls_stream);
                let hyper_svc = hyper::service::service_fn(move |req| svc.clone().oneshot(req));

                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, hyper_svc)
                    .with_upgrades()
                    .await
                {
                    let msg = e.to_string();
                    if !msg.contains("connection closed") && !msg.contains("reset by peer") {
                        error!("HTTP/1.1 connection: {}", e);
                    }
                }
            });
        }
    });
}

async fn health_check() -> &'static str {
    "OK"
}

async fn serve_index() -> Html<&'static str> {
    Html(MAIN_HTML)
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

async fn ws_handler(ws: WebSocketUpgrade, State(status): State<BotStatus>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, status))
}

async fn handle_ws(socket: WebSocket, status: BotStatus) {
    let (ws_tx, mut ws_rx) = socket.split();

    // ── mpsc bridge: multiple tasks → single WebSocket sender ─────────────────
    let (mpsc_tx, mut mpsc_rx) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();

    let fwd_task = tokio::spawn(async move {
        let mut ws_tx = ws_tx;
        while let Some(msg) = mpsc_rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    // ── broadcast → mpsc bridge (push server events to this client) ───────────
    let mut bcast_rx = status.subscribe();
    let bcast_mpsc = mpsc_tx.clone();
    let bcast_task = tokio::spawn(async move {
        loop {
            match bcast_rx.recv().await {
                Ok(s) => {
                    if bcast_mpsc.send(WsMessage::Text(s.into())).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    // ── send initial state to the newly connected client ──────────────────────
    let (config_str, ollama_entries) = load_config();

    ws_send(
        &mpsc_tx,
        json!({"type": "config", "data": config_str}).to_string(),
    );

    let manifest = fs::read_to_string("system_manifest.md").unwrap_or_default();
    ws_send(
        &mpsc_tx,
        json!({"type": "manifest", "data": manifest}).to_string(),
    );

    // Build LLM backends list for the selector buttons
    let mut backends: Vec<serde_json::Value> = ollama_entries
        .iter()
        .map(|e| {
            json!({
                "id":    e.name,
                "label": format!("Ollama — {}", e.name),
                "kind":  "ollama",
            })
        })
        .collect();

    if backends.is_empty() {
        // Fall back to env-var defaults when config.toml has no [[ollama]] entries
        let (_, model) = {
            let g = status.inner.lock().unwrap();
            (g.ollama_url.clone(), g.ollama_model.clone())
        };
        backends.push(json!({
            "id":    "default",
            "label": format!("Ollama ({})", model),
            "kind":  "ollama",
        }));
    }

    ws_send(
        &mpsc_tx,
        json!({"type": "backends", "backends": backends}).to_string(),
    );
    ws_send(
        &mpsc_tx,
        json!({"type": "log", "level": "info", "msg": "WebSocket connected"}).to_string(),
    );

    // ── main receive loop ─────────────────────────────────────────────────────
    while let Some(Ok(msg)) = ws_rx.next().await {
        let text = match msg {
            WsMessage::Text(t) => t.to_string(),
            WsMessage::Close(_) => break,
            _ => continue,
        };

        let val: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match val["type"].as_str().unwrap_or("") {
            // ── Chat: forward prompt to Ollama ────────────────────────────────
            "chat" => {
                let prompt = val["msg"].as_str().unwrap_or("").to_string();
                let llm_id = val["llm"].as_str().unwrap_or("").to_string();
                if prompt.is_empty() {
                    continue;
                }

                // Echo user message back immediately
                ws_send(
                    &mpsc_tx,
                    json!({"type": "user_msg", "from": "You", "data": prompt.clone()}).to_string(),
                );

                let (url, model) = resolve_backend(&ollama_entries, &llm_id, &status);
                let tx_c = mpsc_tx.clone();
                let llm_label = llm_id.clone();

                tokio::spawn(async move {
                    let reply = match ollama_chat(&url, &model, &prompt).await {
                        Ok(r) => json!({
                            "type": "ollama_response",
                            "llm":  llm_label,
                            "msg":  r,
                        }),
                        Err(e) => json!({
                            "type": "ollama_response",
                            "llm":  "error",
                            "msg":  format!("❌ {}", e),
                        }),
                    };
                    let _ = tx_c.send(WsMessage::Text(reply.to_string().into()));
                });
            }

            // ── Config save ───────────────────────────────────────────────────
            "config_save" => {
                let data = val["data"].as_str().unwrap_or("").to_string();
                let reply = if toml::from_str::<TomlConfig>(&data).is_ok() {
                    match fs::write("config.toml", &data) {
                        Ok(()) => json!({
                            "type":   "config_status",
                            "status": "success",
                            "msg":    "✅ Config saved. Restart the bot to apply changes.",
                        }),
                        Err(e) => json!({
                            "type":   "config_status",
                            "status": "error",
                            "msg":    format!("❌ Write error: {}", e),
                        }),
                    }
                } else {
                    json!({
                        "type":   "config_status",
                        "status": "error",
                        "msg":    "❌ Invalid TOML syntax — not saved.",
                    })
                };
                ws_send(&mpsc_tx, reply.to_string());
            }

            // ── Manifest save ─────────────────────────────────────────────────
            "manifest_save" => {
                let data = val["data"].as_str().unwrap_or("").to_string();
                let reply = match fs::write("system_manifest.md", &data) {
                    Ok(()) => json!({
                        "type":   "manifest_status",
                        "status": "success",
                        "msg":    "✅ Manifest saved.",
                    }),
                    Err(e) => json!({
                        "type":   "manifest_status",
                        "status": "error",
                        "msg":    format!("❌ Write error: {}", e),
                    }),
                };
                ws_send(&mpsc_tx, reply.to_string());
            }

            // ── Slash commands ────────────────────────────────────────────────
            "slash_cmd" => {
                let cmd = val["cmd"].as_str().unwrap_or("").trim().to_string();
                ws_send(
                    &mpsc_tx,
                    json!({"type": "user_msg", "from": "You", "data": cmd.clone()}).to_string(),
                );
                let result = handle_slash_command(&cmd, &status);
                ws_send(
                    &mpsc_tx,
                    json!({"type": "ollama_response", "llm": "system", "msg": result}).to_string(),
                );
            }

            _ => {}
        }
    }

    fwd_task.abort();
    bcast_task.abort();
}

/// Push a JSON string through the mpsc channel to the WebSocket sender task.
fn ws_send(tx: &tokio::sync::mpsc::UnboundedSender<WsMessage>, msg: String) {
    let _ = tx.send(WsMessage::Text(msg.into()));
}

/// Resolve which Ollama URL + model to use for the selected `llm_id`.
/// Falls back to the first config entry, then to env-var defaults.
fn resolve_backend(entries: &[OllamaEntry], llm_id: &str, status: &BotStatus) -> (String, String) {
    if let Some(e) = entries.iter().find(|e| e.name == llm_id) {
        return (e.url.clone(), e.model.clone());
    }
    if let Some(e) = entries.first() {
        return (e.url.clone(), e.model.clone());
    }
    let g = status.inner.lock().unwrap();
    (g.ollama_url.clone(), g.ollama_model.clone())
}

/// Handle a `/command` typed in the chat box without needing any external tools.
fn handle_slash_command(cmd: &str, status: &BotStatus) -> String {
    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
    let verb = parts[0].trim_start_matches('/').to_lowercase();

    match verb.as_str() {
        "status" => {
            let g = status.inner.lock().unwrap();
            let secs = g.started_at.elapsed().as_secs();
            let ping = match g.last_ping_ok {
                Some(true) => "✅ OK",
                Some(false) => "❌ FAILED",
                None => "⏳ pending",
            };
            let discord = if g.discord_enabled {
                "enabled"
            } else {
                "disabled (no token)"
            };
            format!(
                "Bot Status\n\
                 ──────────────────\n\
                 Uptime:   {}h {}m {}s\n\
                 Ollama:   {}\n\
                 Model:    {}\n\
                 Ping:     {}\n\
                 Discord:  {}",
                secs / 3600,
                (secs % 3600) / 60,
                secs % 60,
                g.ollama_url,
                g.ollama_model,
                ping,
                discord,
            )
        }

        "ping" => {
            let g = status.inner.lock().unwrap();
            let age = g.last_ping_at.map(|t| {
                let s = t.elapsed().as_secs();
                format!("{}s ago", s)
            });
            format!(
                "Ollama @ {}\nLast keepalive: {} ({})",
                g.ollama_url,
                match g.last_ping_ok {
                    Some(true) => "✅ OK",
                    Some(false) => "❌ FAILED",
                    None => "⏳ not yet",
                },
                age.as_deref().unwrap_or("never")
            )
        }

        "log" => {
            let file = parts
                .get(1)
                .map(|f| {
                    if f.contains('/') || f.contains('\\') {
                        f.to_string()
                    } else {
                        format!("logs/{}", f)
                    }
                })
                .unwrap_or_else(|| "logs/chat_log.md".to_string());

            match fs::read_to_string(&file) {
                Ok(content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    let start = lines.len().saturating_sub(20);
                    format!(
                        "Last {} lines of {}:\n{}",
                        lines.len() - start,
                        file,
                        lines[start..].join("\n")
                    )
                }
                Err(e) => format!("Error reading {}: {}", file, e),
            }
        }

        "help" => "\
Slash commands:\n\
  /status      — bot health and uptime\n\
  /ping        — Ollama keepalive status\n\
  /log [file]  — tail a log file (default: logs/chat_log.md)\n\
  /help        — this message"
            .to_string(),

        other => format!("Unknown command '/{}' — type /help for a list", other),
    }
}

// ── Full control-panel HTML ───────────────────────────────────────────────────

const MAIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Bot Control Panel</title>
    <style>
        * { box-sizing: border-box; }
        body { font-family: Arial, sans-serif; margin: 20px; background: #1a1a2e; color: #e0e0e0; }
        h1 { color: #00d4ff; margin-bottom: 15px; }
        #tabs { display: flex; margin-bottom: 20px; gap: 5px; }
        #tabs button { padding: 10px 20px; background: #16213e; color: #e0e0e0; border: 1px solid #0f3460; cursor: pointer; border-radius: 4px; }
        #tabs button.active { background: #0f3460; color: #00d4ff; border-color: #00d4ff; }
        .tab-content { display: none; background: #16213e; padding: 20px; border-radius: 8px; border: 1px solid #0f3460; }
        .tab-content.active { display: block; }

        /* LLM Selector */
        #llm-selector { display: flex; gap: 8px; margin-bottom: 12px; flex-wrap: wrap; align-items: center; }
        #llm-selector span { color: #aaa; font-size: 0.9em; margin-right: 4px; }
        .llm-btn { padding: 6px 16px; border: 1px solid #0f3460; background: #0d1b2a; color: #ccc; cursor: pointer; border-radius: 20px; font-size: 0.85em; transition: all 0.2s; }
        .llm-btn.active { background: #0f3460; color: #00d4ff; border-color: #00d4ff; font-weight: bold; }
        .llm-btn:hover { border-color: #00d4ff; color: #00d4ff; }

        /* Chat */
        #chat-row { display: flex; gap: 8px; }
        #chat-input { flex: 1; padding: 10px; background: #0d1b2a; color: #e0e0e0; border: 1px solid #0f3460; border-radius: 4px; }
        #chat-send { padding: 10px 20px; background: #00897b; color: white; border: none; cursor: pointer; border-radius: 4px; }
        #chat-send:hover { background: #00acc1; }

        #chat-messages, #log-output {
            height: 400px; overflow-y: scroll; border: 1px solid #0f3460;
            padding: 12px; margin-top: 10px; background: #0d1b2a; border-radius: 4px;
        }
        .message { margin-bottom: 12px; line-height: 1.5; white-space: pre-wrap; }
        .message .sender { font-weight: bold; }
        .you .sender { color: #00d4ff; }
        .bot .sender { color: #69f0ae; }
        .error-msg .sender { color: #ff5252; }
        .warning-msg .sender { color: #ffab40; }
        .system .sender { color: #ce93d8; }

        .log-info  { color: #81d4fa; font-size: 0.85em; font-family: monospace; }
        .log-error { color: #ff5252; font-size: 0.85em; font-family: monospace; }
        .log-warn  { color: #ffab40; font-size: 0.85em; font-family: monospace; }
        .log-debug { color: #aaa;    font-size: 0.85em; font-family: monospace; }

        #status-bar { font-size: 0.8em; color: #aaa; margin-top: 8px; }
        #status-bar .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: #ff5252; margin-right: 5px; }
        #status-bar .dot.connected { background: #69f0ae; }

        textarea { width: 100%; background: #0d1b2a; color: #e0e0e0; border: 1px solid #0f3460; border-radius: 4px; padding: 8px; font-family: monospace; font-size: 0.9em; }
        button.save-btn { margin-top: 8px; padding: 8px 20px; background: #0f3460; color: #00d4ff; border: 1px solid #00d4ff; cursor: pointer; border-radius: 4px; }
        button.save-btn:hover { background: #1a5276; }
        .status-msg { margin-top: 8px; font-size: 0.9em; }
        .thinking { color: #aaa; font-style: italic; }
    </style>
</head>
<body>
    <h1>&#x1F916; Bot Control Panel</h1>

    <div id="tabs">
        <button class="tab-button active" onclick="showTab(event,'chat')">Chat</button>
        <button class="tab-button" onclick="showTab(event,'config')">Config</button>
        <button class="tab-button" onclick="showTab(event,'manifest')">Manifest</button>
        <button class="tab-button" onclick="showTab(event,'logs')">Logs</button>
    </div>

    <!-- CHAT TAB -->
    <div id="chat-tab" class="tab-content active">
        <div id="llm-selector">
            <span>LLM:</span>
            <div id="llm-buttons"></div>
        </div>
        <div id="chat-row">
            <input type="text" id="chat-input"
                   placeholder="Type a message or /help for commands… (Enter to send)"
                   onkeypress="if(event.key==='Enter') sendChat()">
            <button id="chat-send" onclick="sendChat()">Send &#x27A4;</button>
        </div>
        <div id="chat-messages"></div>
        <div id="status-bar">
            <span class="dot" id="ws-dot"></span>
            <span id="ws-status">Connecting…</span>
        </div>
    </div>

    <!-- CONFIG TAB -->
    <div id="config-tab" class="tab-content">
        <h2>Configuration (config.toml)</h2>
        <textarea id="config-textarea" rows="22"></textarea>
        <button class="save-btn" onclick="saveConfig()">&#x1F4BE; Save Config</button>
        <div id="config-status" class="status-msg"></div>
    </div>

    <!-- MANIFEST TAB -->
    <div id="manifest-tab" class="tab-content">
        <h2>System Manifest</h2>
        <textarea id="manifest-textarea" rows="22"></textarea>
        <button class="save-btn" onclick="saveManifest()">&#x1F4BE; Save Manifest</button>
        <div id="manifest-status" class="status-msg"></div>
    </div>

    <!-- LOGS TAB -->
    <div id="logs-tab" class="tab-content">
        <h2>Live Logs</h2>
        <button class="save-btn" style="margin-bottom:8px"
                onclick="document.getElementById('log-output').innerHTML=''">&#x1F9F9; Clear</button>
        <div id="log-output"></div>
    </div>

    <script>
        let ws = null;
        let selectedLlm = '';
        // Use wss:// (secure WebSocket) on the same host:port as this page
        const WS_URL = 'wss://' + location.host + '/ws';

        // ── WebSocket ─────────────────────────────────────────────────────────
        function connectWS() {
            ws = new WebSocket(WS_URL);
            ws.onopen  = () => { setStatus(true);  console.log('WS connected'); };
            ws.onclose = () => {
                setStatus(false);
                console.log('WS disconnected — reconnecting in 3 s…');
                setTimeout(connectWS, 3000);
            };
            ws.onerror   = () => setStatus(false);
            ws.onmessage = handleMessage;
        }

        function setStatus(ok) {
            document.getElementById('ws-dot').className = 'dot' + (ok ? ' connected' : '');
            document.getElementById('ws-status').textContent = ok ? 'Connected' : 'Disconnected';
        }

        // ── Incoming message dispatcher ───────────────────────────────────────
        function handleMessage(event) {
            let data;
            try { data = JSON.parse(event.data); } catch(e) { return; }

            switch (data.type) {
                case 'backends':
                    buildLlmButtons(data.backends || []);
                    return;

                case 'user_msg':
                    appendChat('you', 'You', toStr(data.data));
                    return;

                case 'ollama_response':
                    if (data.llm === 'error') {
                        appendChat('error-msg', '⚠ Error', toStr(data.msg));
                    } else if (data.llm === 'system') {
                        appendChat('system', '⚙ System', toStr(data.msg));
                    } else {
                        appendChat('bot', labelFor(data.llm) || 'Bot', toStr(data.msg));
                    }
                    removethinking();
                    return;

                case 'manifest':
                    document.getElementById('manifest-textarea').value = data.data || '';
                    return;

                case 'config':
                    document.getElementById('config-textarea').value = data.data || '';
                    return;

                case 'config_status': {
                    const el = document.getElementById('config-status');
                    el.textContent  = data.msg || '';
                    el.style.color  = data.status === 'success' ? '#69f0ae' : '#ff5252';
                    return;
                }

                case 'manifest_status': {
                    const el = document.getElementById('manifest-status');
                    el.textContent  = data.msg || '';
                    el.style.color  = data.status === 'success' ? '#69f0ae' : '#ff5252';
                    return;
                }

                case 'log':
                    appendLog(data.level || 'info', data.msg || toStr(data));
                    return;

                default:
                    // Bus-wrapped messages {to, from, data}
                    if (data.to === 'web_interface') {
                        let inner;
                        try { inner = (typeof data.data === 'string') ? JSON.parse(data.data) : data.data; }
                        catch(e) { inner = data.data; }
                        if (inner && inner.type) {
                            handleMessage({ data: JSON.stringify(inner) });
                        }
                    }
            }
        }

        // ── LLM selector buttons ──────────────────────────────────────────────
        const llmLabels = {};

        function buildLlmButtons(backends) {
            const container = document.getElementById('llm-buttons');
            container.innerHTML = '';
            backends.forEach((b, i) => {
                llmLabels[b.id] = b.label;
                const btn = document.createElement('button');
                btn.className   = 'llm-btn' + (i === 0 ? ' active' : '');
                btn.textContent = b.label;
                btn.dataset.id  = b.id;
                if (i === 0) selectedLlm = b.id;
                btn.onclick = () => {
                    document.querySelectorAll('.llm-btn').forEach(x => x.classList.remove('active'));
                    btn.classList.add('active');
                    selectedLlm = b.id;
                };
                container.appendChild(btn);
            });
        }

        function labelFor(id) { return llmLabels[id] || id; }

        // ── Chat helpers ──────────────────────────────────────────────────────
        function sendChat() {
            const input = document.getElementById('chat-input');
            const msg   = input.value.trim();
            if (!msg || !ws || ws.readyState !== WebSocket.OPEN) return;
            input.value = '';

            if (msg.startsWith('/')) {
                ws.send(JSON.stringify({ type: 'slash_cmd', cmd: msg }));
                return;
            }

            ws.send(JSON.stringify({ type: 'chat', msg: msg, llm: selectedLlm }));
            showThinking();
        }

        function showThinking() {
            const div = document.createElement('div');
            div.className = 'message thinking';
            div.id = 'thinking-indicator';
            div.textContent = '⏳ Thinking…';
            const c = document.getElementById('chat-messages');
            c.appendChild(div);
            div.scrollIntoView({ behavior: 'smooth' });
        }

        function removethinking() {
            const el = document.getElementById('thinking-indicator');
            if (el) el.remove();
        }

        function appendChat(cssClass, from, msg) {
            removethinking();
            const div = document.createElement('div');
            div.className = 'message ' + cssClass;
            div.innerHTML =
                '<span class="sender">' + escapeHtml(toStr(from)) + ':</span> ' +
                escapeHtml(toStr(msg));
            const c = document.getElementById('chat-messages');
            c.appendChild(div);
            div.scrollIntoView({ behavior: 'smooth' });
        }

        function appendLog(level, msg) {
            const div = document.createElement('div');
            div.className = 'log-' + (level || 'info');
            const ts = new Date().toLocaleTimeString();
            div.textContent = '[' + ts + '][' + (level || 'info').toUpperCase() + '] ' + msg;
            const c = document.getElementById('log-output');
            c.appendChild(div);
            div.scrollIntoView({ behavior: 'smooth' });
        }

        // ── Tab switching ─────────────────────────────────────────────────────
        function showTab(event, name) {
            document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
            document.querySelectorAll('.tab-button').forEach(b => b.classList.remove('active'));
            document.getElementById(name + '-tab').classList.add('active');
            event.target.classList.add('active');
        }

        // ── Config / Manifest save ────────────────────────────────────────────
        function saveConfig() {
            const toml = document.getElementById('config-textarea').value;
            if (ws && ws.readyState === WebSocket.OPEN)
                ws.send(JSON.stringify({ type: 'config_save', data: toml }));
        }

        function saveManifest() {
            const md = document.getElementById('manifest-textarea').value;
            if (ws && ws.readyState === WebSocket.OPEN)
                ws.send(JSON.stringify({ type: 'manifest_save', data: md }));
        }

        // ── Utilities ─────────────────────────────────────────────────────────
        function toStr(v) {
            if (v === null || v === undefined) return '';
            if (typeof v === 'string') return v;
            return JSON.stringify(v);
        }

        function escapeHtml(text) {
            const m = { '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#039;' };
            return String(text).replace(/[&<>"']/g, c => m[c]);
        }

        connectWS();
    </script>
</body>
</html>
"#;

// ── Discord event handler ─────────────────────────────────────────────────────

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!("Discord bot logged in as '{}'", ready.user.name);
    }

    async fn message(&self, ctx: Context, msg: DiscordMessage) {
        if msg.author.bot {
            return;
        }

        let content = msg.content.to_lowercase();
        let reply = match content.as_str() {
            "hello" => Some("Hello!"),
            "ping" => Some("pong"),
            "help" => Some("Available commands: hello, ping, help"),
            _ => None,
        };

        if let Some(text) = reply {
            if let Err(e) = msg.reply(&ctx.http, text).await {
                error!("Failed to reply to '{}': {:?}", content, e);
            } else {
                info!("Replied '{}' to {}", text, msg.author.name);
            }
        }
    }
}

// ── Ollama preload ────────────────────────────────────────────────────────────

/// Send a dummy `POST /api/generate` (empty prompt, `num_predict: 0`) to Ollama
/// so the configured model is loaded into GPU/CPU memory before the first real
/// user message arrives, eliminating cold-start delay.
///
/// Retries up to [`KA_MAX_RETRIES`] times with exponential back-off
/// (2 s → 4 s → 8 s, capped at 30 s) to tolerate transient Starlink drops.
async fn preload_model(base_url: &str, model: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(KA_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(KA_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let url = format!("{}/api/generate", base_url);
    info!(
        "Ollama preload ▶  model='{}' @ {} — warming up…",
        model, base_url
    );

    let mut delay_ms: u64 = 2_000;

    for attempt in 1..=KA_MAX_RETRIES {
        let payload = serde_json::json!({
            "model":       model,
            "prompt":      "",
            "stream":      false,
            "num_predict": 0
        });

        let result = tokio::time::timeout(
            Duration::from_secs(KA_REQUEST_TIMEOUT_SECS),
            client.post(&url).json(&payload).send(),
        )
        .await;

        match result {
            Err(_elapsed) => {
                warn!(
                    "Ollama preload attempt {}/{} timed out after {}s",
                    attempt, KA_MAX_RETRIES, KA_REQUEST_TIMEOUT_SECS
                );
            }
            Ok(Ok(resp)) => {
                let status = resp.status();
                if status.is_success() {
                    info!(
                        "Ollama model preloaded successfully ✅  (attempt {}/{}, HTTP {})",
                        attempt, KA_MAX_RETRIES, status
                    );
                    return Ok(());
                }
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    "Ollama preload attempt {}/{} — HTTP {}: {}",
                    attempt,
                    KA_MAX_RETRIES,
                    status,
                    &body[..body.len().min(200)]
                );
            }
            Ok(Err(e)) => {
                warn!(
                    "Ollama preload attempt {}/{} — network error: {}",
                    attempt, KA_MAX_RETRIES, e
                );
            }
        }

        if attempt < KA_MAX_RETRIES {
            info!("Ollama preload retrying in {}ms…", delay_ms);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            delay_ms = (delay_ms * 2).min(30_000);
        }
    }

    Err(format!(
        "Ollama preload failed after {} attempts — \
         model '{}' may have a cold-start delay on the first query",
        KA_MAX_RETRIES, model
    ))
}

// ── Ollama keep-alive ─────────────────────────────────────────────────────────

/// Spawn a long-lived background task that pings Ollama periodically so the
/// model stays resident in GPU/CPU memory between sessions.
///
/// Pass `interval_secs = 0` to disable without spawning a task.
fn spawn_keepalive_task(base_url: String, model: String, interval_secs: u64, status: BotStatus) {
    if interval_secs == 0 {
        info!("Ollama keep-alive heartbeat disabled (OLLAMA_KEEP_ALIVE_SECS=0)");
        return;
    }

    info!(
        "Ollama keep-alive ❤  task spawned — model='{}', interval={}s",
        model, interval_secs
    );

    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(KA_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(KA_REQUEST_TIMEOUT_SECS))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                error!("Ollama keep-alive: failed to build HTTP client — {}", e);
                return;
            }
        };

        let url = format!("{}/api/generate", base_url);
        let interval = Duration::from_secs(interval_secs);

        loop {
            tokio::time::sleep(interval).await;
            let ok = ping_once(&client, &url, &model).await;
            status.record_ping(ok);
        }
    });
}

/// Fire one keepalive ping with up to [`KA_MAX_RETRIES`] attempts and
/// exponential back-off.  Returns `true` on success.  Never panics.
async fn ping_once(client: &reqwest::Client, url: &str, model: &str) -> bool {
    let payload = serde_json::json!({
        "model":      model,
        "prompt":     "",
        "stream":     false,
        "keep_alive": KA_KEEP_ALIVE_FIELD
    });

    let mut delay_ms: u64 = 2_000;

    for attempt in 1..=KA_MAX_RETRIES {
        let result = tokio::time::timeout(
            Duration::from_secs(KA_REQUEST_TIMEOUT_SECS),
            client.post(url).json(&payload).send(),
        )
        .await;

        match result {
            Err(_) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} timed out",
                    attempt, KA_MAX_RETRIES
                );
            }
            Ok(Ok(resp)) if resp.status().is_success() => {
                info!(
                    "Ollama keep-alive ❤  ping OK (attempt {}/{}, HTTP {})",
                    attempt,
                    KA_MAX_RETRIES,
                    resp.status()
                );
                return true;
            }
            Ok(Ok(resp)) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} — HTTP {}",
                    attempt,
                    KA_MAX_RETRIES,
                    resp.status()
                );
            }
            Ok(Err(e)) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} — network error: {}",
                    attempt, KA_MAX_RETRIES, e
                );
            }
        }

        if attempt < KA_MAX_RETRIES {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            delay_ms = (delay_ms * 2).min(30_000);
        }
    }

    error!(
        "Ollama keep-alive ❌  all {} attempts failed — model may be unloaded from memory",
        KA_MAX_RETRIES
    );
    false
}

// ── Config helpers ────────────────────────────────────────────────────────────

/// Read `OLLAMA_PRELOAD` env var.  Defaults to `true`.
fn read_preload_flag() -> bool {
    env::var("OLLAMA_PRELOAD")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        != "false"
}

/// Read `OLLAMA_KEEP_ALIVE_SECS` env var.  Falls back to the default constant.
fn read_keep_alive_secs() -> u64 {
    env::var("OLLAMA_KEEP_ALIVE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(KA_DEFAULT_INTERVAL_SECS)
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // ── 1. load .env and initialise logging ───────────────────────────────────
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Bot starting up…");

    // ── 2. read Ollama config ─────────────────────────────────────────────────
    let ollama_url =
        env::var("OLLAMA_URL").unwrap_or_else(|_| "http://192.168.1.149:11434".to_string());
    let ollama_model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen3.5:0.8b".to_string());

    info!(
        "Ollama config — url='{}', model='{}'",
        ollama_url, ollama_model
    );

    // ── 3. Ollama model preload ───────────────────────────────────────────────
    if read_preload_flag() {
        match preload_model(&ollama_url, &ollama_model).await {
            Ok(()) => info!("Ollama model preloaded successfully"),
            Err(e) => warn!("Ollama preload skipped — {}", e),
        }
    } else {
        info!("Ollama preload disabled via OLLAMA_PRELOAD=false");
    }

    // ── 4. Ollama keep-alive heartbeat ────────────────────────────────────────
    let keep_alive_secs = read_keep_alive_secs();
    let discord_enabled = env::var("DISCORD_TOKEN").is_ok();
    let status = BotStatus::new(ollama_url.clone(), ollama_model.clone(), discord_enabled);
    spawn_keepalive_task(ollama_url, ollama_model, keep_alive_secs, status.clone());

    // ── 5. HTTP control panel ─────────────────────────────────────────────────
    spawn_status_server(status);

    // ── 6. Discord client (optional) ─────────────────────────────────────────
    match env::var("DISCORD_TOKEN") {
        Ok(token) => {
            info!("DISCORD_TOKEN found — starting Discord client…");

            let intents = GatewayIntents::GUILD_MESSAGES
                | GatewayIntents::MESSAGE_CONTENT
                | GatewayIntents::GUILDS;

            let mut client = Client::builder(&token, intents)
                .event_handler(Handler)
                .await
                .expect("Failed to create Discord client");

            info!("Discord client created — connecting…");

            if let Err(e) = client.start().await {
                error!("Discord client error: {:?}", e);
            }
        }
        Err(_) => {
            info!(
                "DISCORD_TOKEN not set — Discord disabled. \
                 Running in Ollama-only mode. \
                 Add DISCORD_TOKEN to .env to enable Discord."
            );
            // Park main — keep the HTTP control panel and keepalive tasks alive.
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        }
    }
}
