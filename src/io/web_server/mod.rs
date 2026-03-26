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
use rcgen::generate_simple_self_signed;
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use toml;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    bus: Arc<Bus>,
    msg_tx: broadcast::Sender<String>,
    config_str: String,
}

#[derive(Deserialize)]
struct Config {
    bot: BotConfig,
    ollama: OllamaConfig,
    web: WebConfig,
    heartbeat: HeartbeatConfig,
}

#[derive(Deserialize)]
struct BotConfig {
    name: String,
}

#[derive(Deserialize)]
struct OllamaConfig {
    url: String,
    model: String,
}

#[derive(Deserialize)]
struct WebConfig {
    port: u16,
}

#[derive(Deserialize)]
struct HeartbeatConfig {
    interval_seconds: u64,
}

pub async fn start_web_server(
    bus: Arc<Bus>,
    port: u16,
    config_str: String,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting HTTPS Web Server on 0.0.0.0:{}", port);

    // Generate self-signed certs if not exist
    fs::create_dir_all("certs")?;
    let cert_path = "certs/cert.pem";
    let key_path = "certs/key.pem";
    if !Path::new(cert_path).exists() || !Path::new(key_path).exists() {
        info!("Generating self-signed certificates...");
        let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        let cert = generate_simple_self_signed(subject_alt_names)?;
        fs::write(cert_path, cert.cert.pem())?;
        fs::write(key_path, cert.signing_key.serialize_pem())?;
        info!("Certificates generated at certs/cert.pem and certs/key.pem");
    }

    // Create broadcast channel for web messages
    let (msg_tx, _) = broadcast::channel(100);
    let state = AppState {
        bus,
        msg_tx: msg_tx.clone(),
        config_str,
    };

    // Spawn bus message forwarder
    let state_forwarder = state.clone();
    tokio::spawn(async move {
        let rx = state_forwarder.bus.subscribe("web_interface");
        tokio::task::spawn_blocking(move || {
            while let Ok(msg) = rx.recv() {
                let json_str = serde_json::to_string(&msg)
                    .unwrap_or_else(|_| r#"{"error": "serialize failed"}"#.to_string());
                let _ = state_forwarder.msg_tx.send(json_str);
            }
        });
    });

    // Build app with TLS layer
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/ws", get(ws_handler))
        .with_state(state)
        .layer(CorsLayer::permissive());

    let rustls_config = RustlsConfig::from_pem_file(cert_path, key_path).await?;

    info!("Web server listening on https://localhost:8443 (accept self-signed cert warning)");

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    axum_server::bind_rustls(addr, rustls_config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

fn get_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    // Split WebSocket into sender + receiver (Axum 0.8)
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Subscribe to CPU→Web broadcast channel
    let mut recv = state.msg_tx.subscribe();

    // Send config immediately on connect
    let config_msg = json!({
        "type": "config",
        "data": state.config_str.clone()
    })
    .to_string();

    let _ = ws_sender.send(WsMessage::Text(config_msg.into())).await;

    // Task: forward broadcast messages to WebSocket
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

    // Main loop: WebSocket → Bus
    while let Some(msg) = ws_receiver.next().await {
        let msg = match msg {
            Ok(msg) => msg,
            Err(_) => break,
        };

        if let WsMessage::Text(text_bytes) = msg {
            let text = text_bytes.to_string();

            if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
                if let Some(msg_type) = json_val["type"].as_str() {
                    match msg_type {
                        "chat" => {
                            let chat_msg = json_val["msg"].as_str().unwrap_or("").to_string();

                            // Send to bus → CPU or Ollama
                            let bus_msg = Message {
                                to: "ollama".to_string(),
                                from: "web_user".to_string(),
                                data: chat_msg.clone(),
                                timestamp: get_timestamp(),
                            };
                            state.bus.publish(bus_msg);

                            // Echo user message back to UI
                            let echo_msg = json!({
                                "type": "user_msg",
                                "from": "You",
                                "data": chat_msg
                            })
                            .to_string();

                            let _ = state.msg_tx.send(echo_msg);
                        }

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
                                    state.bus.publish(bus_msg);
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
                                    state.bus.publish(bus_msg);
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
                                state.bus.publish(bus_msg);
                            }
                        }

                        _ => {}
                    }
                }
            }
        }
    }

    recv_task.abort();
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn serve_index() -> Html<String> {
    Html(MAIN_HTML.to_string())
}

const MAIN_HTML: &str = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Bot Control Panel</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; background: #f0f0f0; }
        #tabs { display: flex; margin-bottom: 20px; }
        #tabs button { padding: 10px 20px; margin-right: 5px; background: #007bff; color: white; border: none; cursor: pointer; }
        #tabs button.active { background: #0056b3; }
        .tab-content { display: none; background: white; padding: 20px; border-radius: 5px; box-shadow: 0 2px 5px rgba(0,0,0,0.1); }
        .tab-content.active { display: block; }
        #chat-input { width: 70%; padding: 10px; }
        #chat-send { padding: 10px 20px; background: #28a745; color: white; border: none; cursor: pointer; }
        #chat-messages, #log-output { height: 400px; overflow-y: scroll; border: 1px solid #ddd; padding: 10px; margin-top: 10px; background: #fafafa; }
        .log-info { color: blue; }
        .log-error { color: red; }
        .log-warn { color: orange; }
        .message { margin-bottom: 10px; }
        .message strong { color: #007bff; }
    </style>
</head>
<body>
    <h1>Bot Control Panel (like OpenClaw)</h1>
    <div id="tabs">
        <button class="tab-button active" onclick="showTab(event, 'chat')">Chat</button>
        <button class="tab-button" onclick="showTab(event, 'config')">Config</button>
        <button class="tab-button" onclick="showTab(event, 'logs')">Logs</button>
    </div>

    <div id="chat-tab" class="tab-content active">
        <input type="text" id="chat-input" placeholder="Type your message here..." onkeypress="if(event.key==='Enter') sendChat()">
        <button id="chat-send" onclick="sendChat()">Send</button>
        <div id="chat-messages"></div>
    </div>

    <div id="config-tab" class="tab-content">
        <h2>Configuration</h2>
        <textarea id="config-textarea" rows="20" cols="80"></textarea><br>
        <button onclick="saveConfig()">Save Config</button>
        <div id="config-status"></div>
    </div>

    <div id="logs-tab" class="tab-content">
        <h2>Live Logs</h2>
        <div id="log-output"></div>
    </div>

    <script>
        let ws = null;
        const WS_URL = 'wss://localhost:8443/ws';

        function connectWS() {
            ws = new WebSocket(WS_URL);
            ws.onopen = () => console.log('WebSocket connected');
            ws.onclose = () => {
                console.log('WebSocket disconnected, reconnecting...');
                setTimeout(connectWS, 3000);
            };
            ws.onerror = (err) => console.error('WS error:', err);
            ws.onmessage = (event) => {
                try {
                    const data = JSON.parse(event.data);
                    let inner_data;
                    try {
                        inner_data = JSON.parse(data.data);
                    } catch (e) {
                        inner_data = data.data;
                    }
                    let handled = false;
                    if (data.type === 'user_msg') {
                        appendChat(data.from || 'You', inner_data);
                        handled = true;
                    }
                    if (!handled && inner_data && inner_data.type === 'log') {
                        appendLog(inner_data.level || 'info', inner_data.msg || inner_data.data);
                        handled = true;
                    }
                    if (!handled && data.type === 'config') {
                        document.getElementById('config-textarea').value = data.data;
                        handled = true;
                    }
                    if (!handled && data.type === 'config_status') {
                        const statusDiv = document.getElementById('config-status');
                        statusDiv.textContent = data.msg;
                        statusDiv.style.color = data.status === 'success' ? 'green' : 'red';
                        handled = true;
                    }
                    if (!handled && data.to === 'web_interface') {
                        appendChat(data.from || 'Bot', inner_data);
                    }
                } catch (e) {
                    console.error('Parse error:', e, event.data);
                }
            };
        }

        function sendChat() {
            const input = document.getElementById('chat-input');
            const msg = input.value.trim();
            if (msg && ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({type: 'chat', msg: msg}));
                input.value = '';
            }
        }

        function appendChat(from, msg) {
            const div = document.createElement('div');
            div.className = 'message';
            div.innerHTML = `<strong>${from}:</strong> ${escapeHtml(msg)}`;
            document.getElementById('chat-messages').appendChild(div);
            div.scrollIntoView();
        }

        function appendLog(level, msg) {
            const div = document.createElement('div');
            div.className = `log-${level}`;
            div.textContent = `[${level.toUpperCase()}] ${msg}`;
            document.getElementById('log-output').appendChild(div);
            div.scrollIntoView();
        }

        function showTab(event, tabName) {
            document.querySelectorAll('.tab-content').forEach(tab => tab.classList.remove('active'));
            document.querySelectorAll('.tab-button').forEach(btn => btn.classList.remove('active'));
            document.getElementById(tabName + '-tab').classList.add('active');
            event.target.classList.add('active');
        }

        function saveConfig() {
            const toml = document.getElementById('config-textarea').value;
            if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({type: 'config_save', data: toml}));
            }
        }

        function escapeHtml(text) {
            const map = {'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;'};
            return text.replace(/[&<>"']/g, m => map[m]);
        }

        connectWS();
    </script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    #[tokio::test]
    async fn test_server_start() {
        let bus = Arc::new(Bus::new());
        // Full start expects certs/port, but verifies no panic
        tokio::spawn(async move {
            let _ = start_web_server(bus, 8443, "".to_string()).await;
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}
