use crate::bus::{Bus, Message};
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket},
    },
    response::{Html, IntoResponse},
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::{SinkExt, StreamExt};
use log::info;
use rcgen::{ /* Certificate, CertificateParams, DistinguishedName */ };
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
// use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use toml;
use tower_http::cors::CorsLayer;

// ── AppState ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    bus: Arc<Bus>,
    msg_tx: broadcast::Sender<String>,
    config_str: String,
    backends_json: String, // JSON array of {id, label, kind}
}

// ── Config structs ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Config {
    bot: BotConfig,
    ollama: Vec<OllamaConfig>,
    web: WebConfig,
    heartbeat: HeartbeatConfig,
}

#[derive(Deserialize)]
struct BotConfig {
    name: String,
}

#[derive(Deserialize)]
struct OllamaConfig {
    name: String,
    url: String,
    model: String,
}

#[derive(Deserialize)]
struct WebConfig {
    port: u16,
    #[serde(default)]
    tls_enabled: bool,
    #[serde(default = "default_cert_path")]
    cert_path: String,
    #[serde(default = "default_key_path")]
    key_path: String,
}

fn default_cert_path() -> String {
    "cert.pem".to_string()
}
fn default_key_path() -> String {
    "key.pem".to_string()
}

#[derive(Deserialize)]
struct HeartbeatConfig {
    interval_seconds: u64,
}

// ── Server entry-point ────────────────────────────────────────────────────────

pub async fn start_web_server(
    bus: Arc<Bus>,
    port: u16,
    config_str: String,
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse config (with defaults)
    let parsed_cfg: Config = toml::from_str(&config_str).unwrap_or_else(|_| Config {
        bot: BotConfig {
            name: "Bot".to_string(),
        },
        ollama: vec![],
        web: WebConfig {
            port: 8443,
            tls_enabled: true,
            cert_path: default_cert_path(),
            key_path: default_key_path(),
        },
        heartbeat: HeartbeatConfig {
            interval_seconds: 300,
        },
    });

    let tls_enabled = parsed_cfg.web.tls_enabled;
    let cert_path = &parsed_cfg.web.cert_path;
    let key_path = &parsed_cfg.web.key_path;

    // Ensure certificates exist (generate self-signed if missing)
    if tls_enabled {
        ensure_certificates(cert_path, key_path)?;
    }

    // Build backend list...
    let mut backend_list = Vec::new();
    for b in &parsed_cfg.ollama {
        backend_list.push(serde_json::json!({
            "id":    b.name,
            "label": format!("Ollama {}", b.name),
            "kind":  "ollama"
        }));
    }
    backend_list.push(serde_json::json!({
        "id":    "gemini",
        "label": "Gemini",
        "kind":  "gemini"
    }));
    let backends_json = serde_json::to_string(&backend_list).unwrap_or_else(|_| "[]".to_string());

    let state = AppState {
        bus,
        msg_tx: broadcast::channel(100).0,
        config_str,
        backends_json,
    };

    // Build Axum router
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/ws", get(ws_handler))
        .route("/logs/chat", get(serve_chat_log))
        .route("/logs/chat/clear", post(clear_chat_log))
        .route("/logs/error", get(serve_error_log))
        .route("/logs/bus", get(serve_bus_log))
        .route("/logs/hartbeat", get(serve_hartbeat_log))
        .with_state(state)
        .layer(CorsLayer::permissive());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    if tls_enabled {
        info!("Starting HTTPS Web Server on 0.0.0.0:{}", port);
        info!("Using certificate: {} and key: {}", cert_path, key_path);

        let rustls_config = RustlsConfig::from_pem_file(cert_path, key_path).await?;
        axum_server::bind_rustls(addr, rustls_config)
            .serve(app.into_make_service())
            .await?;
    } else {
        info!("Starting HTTP Web Server on 0.0.0.0:{}", port);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn get_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

async fn handle_ws(socket: WebSocket, state: AppState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Subscribe to the CPU→Web broadcast channel
    let mut recv = state.msg_tx.subscribe();

    // 1. Send current config
    let config_msg = json!({
        "type": "config",
        "data": state.config_str.clone()
    })
    .to_string();
    let _ = ws_sender.send(WsMessage::Text(config_msg.into())).await;

    // 2. Send system manifest
    let manifest_path = "system_manifest.md";
    if !Path::new(manifest_path).exists() {
        fs::write(
            manifest_path,
            "# System Manifest\n\nWelcome to the bot system.\n\nEdit this file to configure behaviour.\n",
        )
        .ok();
    }
    let manifest = fs::read_to_string(manifest_path).unwrap_or_default();
    let manifest_msg = json!({
        "type": "manifest",
        "data": manifest
    })
    .to_string();
    let _ = ws_sender.send(WsMessage::Text(manifest_msg.into())).await;

    // 3. Send backends list so the UI can build LLM selector buttons
    let backends_msg = serde_json::json!({
        "type": "backends",
        "backends": serde_json::from_str::<serde_json::Value>(&state.backends_json)
            .unwrap_or(serde_json::Value::Array(vec![]))
    })
    .to_string();
    let _ = ws_sender.send(WsMessage::Text(backends_msg.into())).await;

    // 4. Send a test log entry to confirm the WS pipe is working
    let test_log_msg = json!({
        "type": "log",
        "level": "info",
        "msg": "WebSocket connection established"
    })
    .to_string();
    let _ = state.msg_tx.send(test_log_msg);

    // 5. Task: forward broadcast messages → WebSocket
    let recv_task = tokio::spawn(async move {
        loop {
            let message_str = match recv.recv().await {
                Ok(msg) => msg,
                Err(_) => break,
            };
            if ws_sender
                .send(WsMessage::Text(message_str.into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // 6. Bus → WebSocket forwarder (single source of truth)
    let bus_clone = state.bus.clone();
    let msg_tx_clone = state.msg_tx.clone();
    let bus_forward_task = tokio::spawn(async move {
        let rx = bus_clone.subscribe("web_interface");
        while let Ok(msg) = rx.recv() {
            let json_msg = json!({
                "to": msg.to,
                "from": msg.from,
                "data": msg.data,
                "timestamp": msg.timestamp
            })
            .to_string();

            let _ = msg_tx_clone.send(json_msg);
        }
    });

    // 6. Main loop: WebSocket messages → Bus
    while let Some(msg) = ws_receiver.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };

        if let WsMessage::Text(text_bytes) = msg {
            let text = text_bytes.to_string();

            if let Ok(json_val) = serde_json::from_str::<Value>(&text)
                && let Some(msg_type) = json_val["type"].as_str()
            {
                match msg_type {
                    // ── Chat: LLM-aware routing ──────────────────────────
                    "chat" => {
                        let chat_msg = json_val["msg"].as_str().unwrap_or("").to_string();
                        if chat_msg.is_empty() {
                            continue;
                        }

                        let llm = json_val["llm"].as_str().unwrap_or("").to_string();
                        info!("Chat request received | llm='{}' | msg='{}'", llm, chat_msg);

                        let bus_dest = if llm == "gemini" {
                            "gemini".to_string()
                        } else if llm.is_empty() {
                            let backends: serde_json::Value =
                                serde_json::from_str(&state.backends_json)
                                    .unwrap_or(serde_json::Value::Array(vec![]));
                            let first_id = backends[0]["id"].as_str().unwrap_or("server");
                            format!("ollama_{}", first_id)
                        } else {
                            format!("ollama_{}", llm)
                        };

                        info!("Routing chat to bus destination: {}", bus_dest);

                        let correlation_id = get_timestamp();
                        let bus_msg = Message {
                            to: bus_dest,
                            from: "web_interface".to_string(),
                            data: json!({
                                "type": "chat_request",
                                "prompt": chat_msg,
                                "correlation_id": correlation_id,
                            })
                            .to_string(),
                            timestamp: correlation_id,
                        };
                        let _ = state.bus.publish(bus_msg);

                        // Echo user message back to UI
                        let echo_msg = json!({
                            "type": "user_msg",
                            "from": "You",
                            "data": chat_msg
                        })
                        .to_string();
                        let _ = state.msg_tx.send(echo_msg);
                    }

                    // ── Config save ──────────────────────────────────────
                    "config_save" => {
                        let new_config_str = json_val["data"].as_str().unwrap_or("");

                        if let Ok(_parsed) = toml::from_str::<Config>(new_config_str) {
                            if fs::write("config.toml", new_config_str).is_ok() {
                                let success_msg = json!({
                                        "type": "config_status",
                                        "status": "success",
                                        "msg": "Config saved successfully. Restart the bot to apply changes."
                                    })
                                    .to_string();
                                let bus_msg = Message {
                                    to: "web_interface".to_string(),
                                    from: "config".to_string(),
                                    data: success_msg,
                                    timestamp: get_timestamp(),
                                };
                                let _ = state.bus.publish(bus_msg);

                                // Echo updated config back
                                let updated_config_msg = json!({
                                    "type": "config",
                                    "data": new_config_str
                                })
                                .to_string();
                                let _ = state.msg_tx.send(updated_config_msg);
                            } else {
                                let error_msg = json!({
                                    "type": "config_status",
                                    "status": "error",
                                    "msg": "Failed to write config file."
                                })
                                .to_string();
                                let bus_msg = Message {
                                    to: "web_interface".to_string(),
                                    from: "config".to_string(),
                                    data: error_msg,
                                    timestamp: get_timestamp(),
                                };
                                let _ = state.bus.publish(bus_msg);
                            }
                        } else {
                            let error_msg = json!({
                                "type": "config_status",
                                "status": "error",
                                "msg": "Invalid TOML syntax."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "config".to_string(),
                                data: error_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);
                        }
                    }

                    // ── Manifest save ────────────────────────────────────
                    "manifest_save" => {
                        let new_manifest = json_val["data"].as_str().unwrap_or("");

                        if fs::write("system_manifest.md", new_manifest).is_ok() {
                            let success_msg = json!({
                                "type": "manifest_status",
                                "status": "success",
                                "msg": "Manifest saved successfully."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "manifest".to_string(),
                                data: success_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);

                            // Echo updated manifest back
                            let updated_manifest_msg = json!({
                                "type": "manifest",
                                "data": new_manifest
                            })
                            .to_string();
                            let _ = state.msg_tx.send(updated_manifest_msg);
                        } else {
                            let error_msg = json!({
                                "type": "manifest_status",
                                "status": "error",
                                "msg": "Failed to write manifest file."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "manifest".to_string(),
                                data: error_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);
                        }
                    }

                    "slash_cmd" => {
                        let cmd = json_val["cmd"].as_str().unwrap_or("").trim().to_string();
                        let result = handle_slash_command(&cmd);
                        let msg_out = serde_json::json!({
                            "type": "user_msg",
                            "from": "You",
                            "data": cmd.clone()
                        })
                        .to_string();
                        let _ = state.msg_tx.send(msg_out);
                        let reply = serde_json::json!({
                            "type": "ollama_response",
                            "llm": "system",
                            "msg": result
                        })
                        .to_string();
                        let bus_msg = crate::bus::Message {
                            to: "web_interface".to_string(),
                            from: "system".to_string(),
                            data: reply,
                            timestamp: get_timestamp(),
                        };
                        let _ = state.bus.publish(bus_msg);
                    }

                    _ => {}
                }
            }
        }
    }

    recv_task.abort();
    bus_forward_task.abort();
}

// ── Axum route helpers ────────────────────────────────────────────────────────

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn serve_index() -> Html<String> {
    Html(MAIN_HTML.to_string())
}

// ── Log file handlers ────────────────────────────────────────────────────────

async fn serve_chat_log() -> impl IntoResponse {
    let path = "logs/chat_log.md";
    if !std::path::Path::new(path).exists() {
        let _ = std::fs::write(path, "[INIT] Chat log created\n");
    }
    let content = fs::read_to_string(path)
        .unwrap_or_else(|_| "chat_log.md not found or empty".to_string());
    Html(format!(
        "<pre style='white-space:pre-wrap;'>{}</pre>",
        html_escape(&content)
    ))
}

async fn clear_chat_log() -> impl IntoResponse {
    let init_line = format!(
        "[INIT] Chat log cleared via web UI at {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    let _ = fs::write("logs/chat_log.md", init_line);
    Html("<span style='color:#69f0ae'>Chat log cleared.</span>")
}

async fn serve_error_log() -> impl IntoResponse {
    let path = "logs/error_log.md";
    if !std::path::Path::new(path).exists() {
        let _ = std::fs::write(path, "[INIT] Error log created\n");
    }
    let content = fs::read_to_string(path)
        .unwrap_or_else(|_| "error_log.md not found or empty".to_string());
    Html(format!(
        "<pre style='white-space:pre-wrap;'>{}</pre>",
        html_escape(&content)
    ))
}

async fn serve_bus_log() -> impl IntoResponse {
    let content = fs::read_to_string("logs/bus_log.md")
        .unwrap_or_else(|_| "bus_log.md not found or empty".to_string());
    Html(format!(
        "<pre style='white-space:pre-wrap;'>{}</pre>",
        html_escape(&content)
    ))
}

async fn serve_hartbeat_log() -> impl IntoResponse {
    let content = fs::read_to_string("logs/hartbeat_log.md")
        .unwrap_or_else(|_| "hartbeat_log.md not found or empty".to_string());
    Html(format!(
        "<pre style='white-space:pre-wrap;'>{}</pre>",
        html_escape(&content)
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const MAIN_HTML: &str = include_str!("static/index.html");


/// Returns true if a file exists AND contains valid PEM data (not a placeholder).
fn is_valid_pem(path: &str, expected_header: &str) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.contains(expected_header))
        .unwrap_or(false)
}

/// Generate self-signed certificate + key if they are missing or contain
/// placeholder content (not real PEM data).
fn ensure_certificates(cert_path: &str, key_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cert_ok = is_valid_pem(cert_path, "-----BEGIN CERTIFICATE-----");
    let key_ok = is_valid_pem(key_path, "-----BEGIN");

    if cert_ok && key_ok {
        return Ok(());
    }

    log::info!("TLS cert/key missing or invalid — generating self-signed certificate");

    let key_pair = rcgen::KeyPair::generate()?;
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()])?;
    let cert = params.self_signed(&key_pair)?;

    std::fs::write(cert_path, cert.pem())?;
    std::fs::write(key_path, key_pair.serialize_pem())?;

    log::info!(
        "Self-signed certificate written to {} / {}",
        cert_path,
        key_path
    );

    Ok(())
}

/// Execute a slash command typed directly in the chat box.
/// Returns the result string to display in the chat.
fn handle_slash_command(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
    let verb = parts[0].trim_start_matches('/').to_lowercase();
    let args = serde_json::json!({});

    match verb.as_str() {
        "status" => crate::tools::execute("system_status", &args),
        "tools" => crate::tools::execute("list_tools", &args),
        "notes" => crate::tools::execute("list_notes", &args),
        "beliefs" => crate::tools::execute("get_beliefs", &args),
        "note" => {
            // /note <title>
            let title = parts.get(1).copied().unwrap_or("untitled");
            crate::tools::execute("read_note", &serde_json::json!({"title": title}))
        }
        "set" => {
            // /set key=value
            let kv = parts.get(1).copied().unwrap_or("");
            if let Some((k, v)) = kv.split_once('=') {
                crate::tools::execute(
                    "set_belief",
                    &serde_json::json!({"key": k.trim(), "value": v.trim()}),
                )
            } else {
                "Usage: /set key=value".to_string()
            }
        }
        "log" => {
            // /log [filename] — default to chat_log.md
            let file = parts
                .get(1)
                .map(|f| {
                    if f.contains('/') {
                        f.to_string()
                    } else {
                        format!("logs/{}", f)
                    }
                })
                .unwrap_or_else(|| "logs/chat_log.md".to_string());
            crate::tools::execute("read_log", &serde_json::json!({"log_file": file}))
        }
        "bayes" => {
            // /bayes [show|status|update <evidence>|reset]
            let sub = parts.get(1).copied().unwrap_or("show").to_lowercase();
            match sub.as_str() {
                "show" | "status" => crate::tools::execute("bayes_show", &args),
                "reset" => crate::tools::execute("bayes_reset", &args),
                "update" => {
                    let evidence = parts.get(2).copied().unwrap_or("");
                    if evidence.is_empty() {
                        "Usage: /bayes update <evidence>\nExample: /bayes update positive_signal"
                            .to_string()
                    } else {
                        crate::tools::execute(
                            "bayes_update",
                            &serde_json::json!({"evidence": evidence}),
                        )
                    }
                }
                _ => format!(
                    "Unknown bayes sub-command '{}'.\nAvailable: /bayes show  /bayes status  /bayes update <evidence>  /bayes reset",
                    sub
                ),
            }
        }
        "help" => format!(
            "Slash commands:\n\
             /status              — system health\n\
             /tools               — list all tools\n\
             /notes               — list saved notes\n\
             /note <title>        — read a note\n\
             /beliefs             — show agent beliefs\n\
             /set k=v             — store a belief\n\
             /log [file]          — tail a log file\n\
             /bayes show          — show Bayesian belief state\n\
             /bayes update <ev>   — apply Bayesian evidence update\n\
             /bayes reset         — reset to default priors\n\
             /help                — this message"
        ),
        other => format!("Unknown command '/{}'  — type /help for a list", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    #[tokio::test]
    async fn test_server_start() {
        let bus = Arc::new(Bus::new());
        // Full start expects certs/port, but verifies no panic on startup path
        tokio::spawn(async move {
            let _ = start_web_server(bus, 8443, "".to_string()).await;
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}
