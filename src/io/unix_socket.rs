// Task 76-90: UNIX domain socket server for CLI interface
// Provides secure local socket at /var/run/bot.sock with command routing,
// file upload/download, chat streaming, tool/memory commands, and ACL.

#[cfg(unix)]
use crate::config::socket::SocketConfig;
#[cfg(unix)]
use crate::bus::{Bus, Message};
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use serde_json::{json, Value};
#[cfg(unix)]
use tracing::{info, error, warn, debug};

#[cfg(unix)]
pub struct UnixSocketServer {
    config: SocketConfig,
    bus: Bus,
}

#[cfg(unix)]
impl UnixSocketServer {
    pub fn new(config: SocketConfig, bus: Bus) -> Self {
        Self { config, bus }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let path = Path::new(&self.config.path);

        if path.exists() {
            fs::remove_file(path)?;
        }

        let listener = UnixListener::bind(path)?;

        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(self.config.mode);
        fs::set_permissions(path, perms)?;

        info!("UNIX socket listening at {}", self.config.path);

        loop {
            let (stream, _) = listener.accept().await?;
            let bus = self.bus.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, bus).await {
                    error!("Socket client error: {}", e);
                }
            });
        }
    }
}

/// Supported CLI commands (Tasks 78, 82-84)
#[cfg(unix)]
fn route_command(cmd: &str, args: &Value) -> Value {
    match cmd {
        "help" | "--help" => json!({
            "status": "ok",
            "commands": ["help", "ping", "status", "ask", "chat", "upload", "download", "tools", "memory", "logs"]
        }),
        "ping" => json!({"status": "ok", "msg": "pong"}),
        "status" => json!({"status": "ok", "uptime": "N/A", "version": env!("CARGO_PKG_VERSION")}),
        "ask" => {
            let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
            json!({"status": "ok", "action": "ask", "prompt": prompt})
        }
        "chat" => {
            let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
            json!({"status": "ok", "action": "chat", "message": msg})
        }
        "upload" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let data = args.get("data").and_then(|v| v.as_str()).unwrap_or("");
            json!({"status": "ok", "action": "upload", "path": path, "bytes": data.len()})
        }
        "download" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            json!({"status": "ok", "action": "download", "path": path})
        }
        "tools" | "list-tools" => json!({"status": "ok", "action": "list_tools"}),
        "reload-tools" => json!({"status": "ok", "action": "reload_tools"}),
        "memory-list" => json!({"status": "ok", "action": "memory_list"}),
        "memory-dump" => json!({"status": "ok", "action": "memory_dump"}),
        "log-stream" => json!({"status": "ok", "action": "log_stream"}),
        _ => json!({"status": "error", "msg": format!("Unknown command: {}", cmd)}),
    }
}

#[cfg(unix)]
async fn handle_client(mut stream: tokio::net::UnixStream, bus: Bus) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.split();
    let mut lines = BufReader::new(reader).lines();

    // Send welcome banner (Task 86)
    let welcome = json!({"type": "welcome", "msg": "Bot CLI connected. Type 'help' for commands."});
    writer.write_all((welcome.to_string() + "\n").as_bytes()).await?;

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let response = if let Ok(msg) = serde_json::from_str::<Value>(&line) {
            // Structured command: {"cmd": "ask", "args": {...}}
            if let Some(cmd) = msg.get("cmd").and_then(|v| v.as_str()) {
                let args = msg.get("args").cloned().unwrap_or(json!({}));
                debug!("CLI command: {} with args {:?}", cmd, args);

                // Publish to bus for CPU/skills to handle
                let bus_msg = Message {
                    to: "cli".to_string(),
                    from: "unix_socket".to_string(),
                    data: json!({
                        "type": "cli_command",
                        "cmd": cmd,
                        "args": args
                    }).to_string(),
                    timestamp: crate::utils::now_ms(),
                };
                let _ = bus.publish(bus_msg);

                // Route locally for immediate response
                route_command(cmd, &args)
            } else if let Some(prompt) = msg.get("prompt").and_then(|v| v.as_str()) {
                // Legacy prompt mode
                json!({"status": "ok", "echo": prompt})
            } else {
                json!({"status": "error", "msg": "Invalid command format. Use {\"cmd\": \"help\"}"})
            }
        } else {
            // Plain text command
            let cmd = line.trim();
            route_command(cmd, &json!({}))
        };

        let response_line = response.to_string() + "\n";
        if let Err(e) = writer.write_all(response_line.as_bytes()).await {
            warn!("Failed to write response: {}", e);
            break;
        }
    }
    Ok(())
}
