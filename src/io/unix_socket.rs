// Task 76: UNIX domain socket server for CLI interface
// Provides a secure local socket at /var/run/bot.sock (or configurable path)

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
use serde_json::Value;
#[cfg(unix)]
use tracing::{info, error};

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

#[cfg(unix)]
async fn handle_client(mut stream: tokio::net::UnixStream, bus: Bus) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        if let Ok(msg) = serde_json::from_str::<Value>(&line) {
            let bus_msg = Message {
                to: "unix_socket".to_string(),
                from: "unix_socket".to_string(),
                data: msg.to_string(),
                timestamp: crate::utils::now_ms(),
            };
            let _ = bus.publish(bus_msg);

            writer.write_all(b"{\"status\":\"ok\"}\n").await?;
        } else {
            writer.write_all(b"{\"error\":\"invalid json\"}\n").await?;
        }
    }
    Ok(())
}
