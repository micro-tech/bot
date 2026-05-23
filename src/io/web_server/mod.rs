use crate::bus::{Bus, Message};
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket},
    },
    response::{Html, IntoResponse},
    routing::get,
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::{SinkExt, StreamExt};
use log::info;
use rcgen::{Certificate, CertificateParams, DistinguishedName};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
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

                        // Which LLM did the user select?
                        let llm = json_val["llm"].as_str().unwrap_or("").to_string();
                        let bus_dest = if llm == "gemini" {
                            "gemini".to_string()
                        } else if llm.is_empty() {
                            // No explicit selection — use the first ollama backend
                            let backends: serde_json::Value =
                                serde_json::from_str(&state.backends_json)
                                    .unwrap_or(serde_json::Value::Array(vec![]));
                            let first_id = backends[0]["id"].as_str().unwrap_or("server");
                            format!("ollama_{}", first_id)
                        } else {
                            format!("ollama_{}", llm)
                        };

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
    let content = fs::read_to_string("logs/chat_log.md")
        .unwrap_or_else(|_| "chat_log.md not found or empty".to_string());
    Html(format!("<pre style='white-space:pre-wrap;'>{}</pre>", html_escape(&content)))
}

async fn serve_error_log() -> impl IntoResponse {
    let content = fs::read_to_string("logs/error_log.md")
        .unwrap_or_else(|_| "error_log.md not found or empty".to_string());
    Html(format!("<pre style='white-space:pre-wrap;'>{}</pre>", html_escape(&content)))
}

async fn serve_bus_log() -> impl IntoResponse {
    let content = fs::read_to_string("logs/bus_log.md")
        .unwrap_or_else(|_| "bus_log.md not found or empty".to_string());
    Html(format!("<pre style='white-space:pre-wrap;'>{}</pre>", html_escape(&content)))
}

async fn serve_hartbeat_log() -> impl IntoResponse {
    let content = fs::read_to_string("logs/hartbeat_log.md")
        .unwrap_or_else(|_| "hartbeat_log.md not found or empty".to_string());
    Html(format!("<pre style='white-space:pre-wrap;'>{}</pre>", html_escape(&content)))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
}

// ── HTML / JS front-end ───────────────────────────────────────────────────────

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
        .message { margin-bottom: 12px; line-height: 1.5; }
        .message .sender { font-weight: bold; }
        .you .sender { color: #00d4ff; }
        .bot .sender { color: #69f0ae; }
        .error-msg .sender { color: #ff5252; }
        .warning-msg .sender { color: #ffab40; }
        .tool-call .sender { color: #ffd54f; }
        .tool-call code { background: #0d2137; padding: 1px 5px; border-radius: 3px; font-size:0.8em; }
        .tool-call em { color: #90a4ae; font-size: 0.85em; }

        .log-info  { color: #81d4fa; font-size: 0.85em; font-family: monospace; }
        .log-error { color: #ff5252; font-size: 0.85em; font-family: monospace; }
        .log-warn  { color: #ffab40; font-size: 0.85em; font-family: monospace; }
        .log-debug { color: #aaa;    font-size: 0.85em; font-family: monospace; }

        #status-bar { font-size: 0.8em; color: #aaa; margin-top: 8px; }
        #status-bar .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: #ff5252; margin-right: 5px; }
        #status-bar .dot.connected { background: #69f0ae; }

        textarea { width: 100%; background: #0d1b2a; color: #e0e0e0; border: 1px solid #0f3460; border-radius: 4px; padding: 8px; }
        button.save-btn { margin-top: 8px; padding: 8px 20px; background: #0f3460; color: #00d4ff; border: 1px solid #00d4ff; cursor: pointer; border-radius: 4px; }
        button.save-btn:hover { background: #1a5276; }
        .status-msg { margin-top: 8px; font-size: 0.9em; }
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
            <!-- Buttons injected dynamically by JS when 'backends' message arrives -->
            <div id="llm-buttons"></div>
        </div>
        <div id="chat-row">
            <input type="text" id="chat-input" placeholder="Type your message… (Enter to send)"
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
        <h2>System Logs</h2>
        <div style="margin-bottom: 12px; display: flex; gap: 8px; flex-wrap: wrap;">
            <button class="save-btn" onclick="loadLog('/logs/chat')">Chat Log</button>
            <button class="save-btn" onclick="loadLog('/logs/error')">Error Log</button>
            <button class="save-btn" onclick="loadLog('/logs/bus')">Bus Log</button>
            <button class="save-btn" onclick="loadLog('/logs/hartbeat')">Hartbeat Log</button>
            <button class="save-btn" style="background:#5c2d2d; border-color:#ff5252"
                    onclick="document.getElementById('log-output').innerHTML=''">Clear</button>
        </div>
        <div id="log-output" style="height:500px; overflow-y:auto; background:#0d1b2a; border:1px solid #0f3460; padding:12px; border-radius:4px; font-family:monospace; white-space:pre-wrap;"></div>
    </div>

    <script>
        let ws = null;
        let selectedLlm = '';   // '' means "use server default (first backend)"
        const WS_URL = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.hostname + ':8443/ws';


        // ── WebSocket ──────────────────────────────────────────────────────────
        function connectWS() {
            ws = new WebSocket(WS_URL);
            ws.onopen = () => {
                setStatus(true);
                console.log('WebSocket connected');
            };
            ws.onclose = () => {
                setStatus(false);
                console.log('WebSocket disconnected, reconnecting in 3s…');
                setTimeout(connectWS, 3000);
            };
            ws.onerror = (err) => { console.error('WS error:', err); setStatus(false); };
            ws.onmessage = handleMessage;
        }

        function setStatus(connected) {
            document.getElementById('ws-dot').className = 'dot' + (connected ? ' connected' : '');
            document.getElementById('ws-status').textContent = connected ? 'Connected' : 'Disconnected';
        }

        // ── Load static log files ─────────────────────────────────────────────
        async function loadLog(url) {
            const output = document.getElementById('log-output');
            output.innerHTML = '<em>Loading...</em>';
            try {
                const res = await fetch(url);
                const text = await res.text();
                output.innerHTML = text;
                output.scrollTop = 0;
            } catch (e) {
                output.innerHTML = `<span style="color:#ff5252">Failed to load log: ${e}</span>`;
            }
        }

        // ── Message dispatcher ─────────────────────────────────────────────────
        function handleMessage(event) {
            let data;
            try { data = JSON.parse(event.data); }
            catch(e) { console.error('WS parse error:', e, event.data); return; }

            // Parse inner data.data (bus messages wrap payload in .data as JSON string)
            let inner;
            try {
                inner = (typeof data.data === 'string') ? JSON.parse(data.data) : data.data;
            } catch(e) {
                inner = data.data;
            }

            // ── Direct-type messages (server-side echoes, not bus-wrapped) ──
            if (data.type === 'backends') {
                buildLlmButtons(data.backends || []);
                return;
            }
            if (data.type === 'user_msg') {
                appendChat('you', 'You', toStr(inner || data.data));
                return;
            }
            if (data.type === 'manifest') {
                document.getElementById('manifest-textarea').value = data.data || '';
                return;
            }
            if (data.type === 'config') {
                document.getElementById('config-textarea').value = data.data || '';
                return;
            }
            if (data.type === 'log') {
                appendLog(data.level || 'info', data.msg || toStr(data));
                return;
            }

            // ── Bus-wrapped messages (have .to / .from / .data) ──
            if (data.to === 'web_interface') {
                const itype = (inner && typeof inner === 'object') ? inner.type : null;
                switch (itype) {
                    case 'log':
                        appendLog(inner.level || 'info', inner.msg || toStr(inner));
                        return;

                    case 'tool_call': {
                        const toolName = inner.tool || '?';
                        const preview  = inner.result_preview || '';
                        const argsStr  = inner.args ? JSON.stringify(inner.args) : '';
                        appendToolCall(toolName, argsStr, preview);
                        return;
                    }

                    case 'ollama_response': {
                        const llmLabel = inner.llm
                            ? labelFor(inner.llm)
                            : (data.from || 'Bot');
                        appendChat('bot', llmLabel, inner.msg || toStr(inner));
                        return;
                    }

                    case 'llm_output': {
                        const llmLabel = data.from || 'Bot';
                        appendChat('bot', llmLabel, inner.msg || toStr(inner));
                        return;
                    }

                    case 'error':
                        appendChat('error-msg', '\u26A0 Error', inner.msg || toStr(inner));
                        return;

                    case 'warning':
                        appendChat('warning-msg', '\u26A0', inner.msg || toStr(inner));
                        return;

                    case 'config_status': {
                        const el = document.getElementById('config-status');
                        el.textContent = inner.msg || '';
                        el.style.color = inner.status === 'success' ? '#69f0ae' : '#ff5252';
                        return;
                    }

                    case 'manifest_status': {
                        const el = document.getElementById('manifest-status');
                        el.textContent = inner.msg || '';
                        el.style.color = inner.status === 'success' ? '#69f0ae' : '#ff5252';
                        return;
                    }

                    default:
                        console.debug('Unhandled bus message:', data);
                        return;
                }
            }

            console.debug('Unhandled WS message:', data);
        }

        // ── LLM selector ───────────────────────────────────────────────────────
        const llmLabels = {};

        function buildLlmButtons(backends) {
            const container = document.getElementById('llm-buttons');
            container.innerHTML = '';
            backends.forEach((b, i) => {
                llmLabels[b.id] = b.label;
                const btn = document.createElement('button');
                btn.className = 'llm-btn' + (i === 0 ? ' active' : '');
                btn.textContent = b.label;
                btn.dataset.id = b.id;
                if (i === 0) selectedLlm = b.id;
                btn.onclick = () => {
                    document.querySelectorAll('.llm-btn').forEach(b => b.classList.remove('active'));
                    btn.classList.add('active');
                    selectedLlm = b.id;
                };
                container.appendChild(btn);
            });
        }

        function labelFor(id) {
            return llmLabels[id] || id;
        }

        // ── Chat helpers ────────────────────────────────────────────────────────
        function sendChat() {
            const input = document.getElementById('chat-input');
            const msg = input.value.trim();
            if (!msg || !ws || ws.readyState !== WebSocket.OPEN) return;
            // Slash commands → direct skill execution
            if (msg.startsWith('/')) {
                ws.send(JSON.stringify({ type: 'slash_cmd', cmd: msg }));
                input.value = '';
                return;
            }
            ws.send(JSON.stringify({ type: 'chat', msg: msg, llm: selectedLlm }));
            input.value = '';
        }

        function appendChat(cssClass, from, msg) {
            const div = document.createElement('div');
            div.className = 'message ' + cssClass;
            div.innerHTML =
                '<span class="sender">' + escapeHtml(toStr(from)) + ':</span> ' +
                escapeHtml(toStr(msg));
            const container = document.getElementById('chat-messages');
            container.appendChild(div);
            div.scrollIntoView({ behavior: 'smooth' });
        }

        function appendLog(level, msg) {
            const div = document.createElement('div');
            div.className = 'log-' + (level || 'info');
            const ts = new Date().toLocaleTimeString();
            div.textContent = '[' + ts + '][' + (level || 'info').toUpperCase() + '] ' + msg;
            const container = document.getElementById('log-output');
            container.appendChild(div);
            div.scrollIntoView({ behavior: 'smooth' });
        }

        function appendToolCall(tool, args, preview) {
            const div = document.createElement('div');
            div.className = 'message tool-call';
            div.innerHTML =
                `<span class="sender">⚙ Tool [${escapeHtml(tool)}]</span> ` +
                `<code>${escapeHtml(args)}</code>` +
                (preview ? ` → <em>${escapeHtml(preview)}</em>` : '');
            const container = document.getElementById('chat-messages');
            container.appendChild(div);
            div.scrollIntoView({ behavior: 'smooth' });
        }

        // ── Tab switching ───────────────────────────────────────────────────────
        function showTab(event, name) {
            document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
            document.querySelectorAll('.tab-button').forEach(b => b.classList.remove('active'));
            document.getElementById(name + '-tab').classList.add('active');
            event.target.classList.add('active');
        }

        // ── Config / Manifest save ──────────────────────────────────────────────
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

        // ── Utilities ──────────────────────────────────────────────────────────
        function toStr(v) {
            if (v === null || v === undefined) return '';
            if (typeof v === 'string') return v;
            return JSON.stringify(v);
        }

        function escapeHtml(text) {
            const map = { '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#039;' };
            return String(text).replace(/[&<>"']/g, m => map[m]);
        }

        connectWS();
    </script>
</body>
</html>
"#;

/// Generate self-signed certificate + key if they don't exist.
/// Uses rcgen to create a 2048-bit RSA cert valid for 10 years.
fn ensure_certificates(cert_path: &str, key_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if std::path::Path::new(cert_path).exists() && std::path::Path::new(key_path).exists() {
        return Ok(());
    }

    let key_pair = rcgen::KeyPair::generate()?;
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()])?;
    let cert = params.self_signed(&key_pair)?;

    std::fs::write(cert_path, cert.pem())?;
    std::fs::write(key_path, key_pair.serialize_pem())?;

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
