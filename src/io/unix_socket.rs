// Task 76: UNIX domain socket server for CLI interface
// Provides a secure local socket at /var/run/bot.sock (or configurable path)

use crate::config::socket::SocketConfig;
use crate::bus::Bus;
use tokio::net::UnixListener;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use std::path::Path;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use serde_json::Value;
use tracing::{info, error};

pub struct UnixSocketServer {
    config: SocketConfig,
    bus: Bus,
}

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

async fn handle_client(stream: UnixStream, bus: Bus) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        // Expect JSON-lines protocol
        if let Ok(msg) = serde_json::from_str::<Value>(&line) {
            // Forward into internal bus
            bus.publish("unix_socket", msg).await;
            
            // Simple ack
            writer.write_all(b"{\"status\":\"ok\"}\n").await?;
        } else {
            writer.write_all(b"{\"error\":\"invalid json\"}\n").await?;
        }
    }
    Ok(())
}
