use log::{LevelFilter, error, info, set_boxed_logger, set_max_level, warn};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::{Duration as TokioDuration, interval};
use toml;

// Include modules
mod bayesian;
mod bus;
mod cpu;
mod hooks;
mod hy_evo;
mod io;
mod memory;
mod skills;
mod utils;

use crate::bus::{Bus, Message};
use crate::io::ollama::{check_ollama_health, fetch_available_models, handle_ollama_message};
use crate::io::web_server::start_web_server;
use utils::log_to_file;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Heartbeat {
    timestamp: u64,
    system_status: String,
    recent_events: Vec<String>,
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

#[derive(Clone)]
struct LogMsg {
    level: String,
    msg: String,
}

struct WebLogger {
    tx: mpsc::UnboundedSender<LogMsg>,
}

impl log::Log for WebLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let msg = LogMsg {
                level: record.level().to_string().to_lowercase(),
                msg: format!("{}", record.args()),
            };
            let _ = self.tx.send(msg);
        }
    }

    fn flush(&self) {}
}

fn get_current_timestamp() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(_) => {
            log_to_file(&"Failed to get system time".to_string());
            error!("Failed to get system time");
            0
        }
    }
}

async fn send_heartbeat(bus: Arc<Bus>) -> Result<(), String> {
    let heartbeat = Heartbeat {
        timestamp: get_current_timestamp(),
        system_status: "Operational".to_string(),
        recent_events: vec!["System check completed".to_string()],
    };

    info!("Sending heartbeat: {:?}", heartbeat);

    let heartbeat_json = match serde_json::to_string(&heartbeat) {
        Ok(json) => json,
        Err(e) => {
            let error_msg = format!("Failed to serialize heartbeat to JSON: {}", e);
            log_to_file(&error_msg);
            error!("{}", error_msg);
            return Err(error_msg);
        }
    };

    let message = Message {
        to: "ollama".to_string(),
        from: "heartbeat".to_string(),
        data: heartbeat_json,
        timestamp: get_current_timestamp(),
    };
    bus.publish(message);
    info!("Heartbeat published to bus");

    // Publish live log update to web
    let log_data = serde_json::json!({
        "type": "log",
        "level": "info",
        "msg": "Heartbeat sent to Ollama"
    })
    .to_string();
    let log_msg = Message {
        to: "web_interface".to_string(),
        from: "logger".to_string(),
        data: log_data,
        timestamp: get_current_timestamp(),
    };
    bus.publish(log_msg);

    Ok(())
}

#[tokio::main]
async fn main() {
    fs::create_dir_all("logs").expect("Failed to create logs dir");

    // Load config
    let config_str = fs::read_to_string("config.toml").expect("Failed to read config.toml");
    let config: Config = toml::from_str(&config_str).expect("Failed to parse config.toml");

    let bus = Arc::new(Bus::new());

    // Set up custom logger
    let (log_tx, mut log_rx) = mpsc::unbounded_channel();
    let logger = WebLogger { tx: log_tx };
    set_boxed_logger(Box::new(logger)).unwrap();
    set_max_level(LevelFilter::Info);

    // Spawn log forwarder
    let bus_clone = bus.clone();
    tokio::spawn(async move {
        while let Some(log_msg) = log_rx.recv().await {
            let msg = Message {
                to: "web_interface".to_string(),
                from: "logger".to_string(),
                data: serde_json::json!({
                    "type": "log",
                    "level": log_msg.level,
                    "msg": log_msg.msg
                })
                .to_string(),
                timestamp: get_current_timestamp(),
            };
            bus_clone.publish(msg);
        }
    });

    info!("Starting bot with HTTPS web interface and Ollama chat");

    // Extract Ollama config values before any closures can partially capture `config`.
    let ollama_url = config.ollama.url.clone();
    let ollama_model = config.ollama.model.clone();

    // Spawn HTTPS Web Server
    let web_bus = bus.clone();
    let config_str_clone = config_str.clone();
    tokio::spawn(async move {
        if let Err(e) = start_web_server(web_bus, config.web.port, config_str_clone).await {
            error!("Web server failed: {}", e);
        }
    });
    info!(
        "HTTPS Web Server spawned - visit https://localhost:{} (accept self-signed cert warning)",
        config.web.port
    );

    // Spawn Ollama Handler — real async handler with health-check, timeout & retry.
    //
    // The bus uses a sync `std::sync::mpsc` channel internally.  We bridge it
    // to an async `tokio::sync::mpsc` channel so individual LLM requests can
    // be processed concurrently without blocking the Tokio runtime.
    let ollama_bus = bus.clone();
    let ollama_url_clone = ollama_url.clone();
    let ollama_model_clone = ollama_model.clone();
    tokio::spawn(async move {
        // ── Startup health + model check ──────────────────────────────────────
        // Advisory only — we warn but do not abort.  Per-request checks inside
        // `handle_ollama_message` will surface errors when real messages arrive.
        if check_ollama_health(&ollama_url_clone).await {
            info!("Ollama reachable at {} ✅", ollama_url_clone);

            // Fetch the installed model list so the operator can see at a glance
            // what is available — and get a clear warning if the configured model
            // is missing (avoiding three silent 404 retries later).
            match fetch_available_models(&ollama_url_clone).await {
                Ok(models) if models.is_empty() => {
                    warn!(
                        "Ollama has NO models installed. \
                         Run `ollama pull {}` to install the configured model.",
                        ollama_model_clone
                    );
                }
                Ok(models) => {
                    let names = models.join(", ");
                    info!("Ollama available models: [{}]", names);

                    // Check whether the configured model is in the list.
                    // We match on exact name OR base name (ignoring the tag),
                    // e.g. "llama3.2" matches "llama3.2:latest".
                    let model_base = ollama_model_clone
                        .split(':')
                        .next()
                        .unwrap_or(&ollama_model_clone);

                    let found = models.iter().any(|a| {
                        a == &ollama_model_clone || a.split(':').next().unwrap_or(a) == model_base
                    });

                    if found {
                        info!("Configured model '{}' is available ✅", ollama_model_clone);
                    } else {
                        let bullet_list = models
                            .iter()
                            .map(|m| format!("  • {}", m))
                            .collect::<Vec<_>>()
                            .join("\n");
                        warn!(
                            "Configured model '{}' is NOT installed in Ollama! ⚠️\n\
                             Fix options:\n\
                               1. Run:  ollama pull {}\n\
                               2. Or update config.toml → [ollama] model = \"<one of the below>\"\n\
                             Installed models:\n{}",
                            ollama_model_clone, ollama_model_clone, bullet_list
                        );
                    }
                }
                Err(e) => {
                    warn!("Could not fetch Ollama model list on startup: {}", e);
                }
            }
        } else {
            warn!(
                "Ollama NOT reachable at {} on startup ⚠️  — will retry per request",
                ollama_url_clone
            );
        }

        // Bridge: sync mpsc bus receiver  →  async tokio mpsc sender.
        let (tx_bridge, mut rx_bridge) = tokio::sync::mpsc::unbounded_channel::<Message>();
        let rx_sync = ollama_bus.subscribe("ollama");

        tokio::task::spawn_blocking(move || {
            // Blocking thread reads from the sync bus channel and forwards every
            // message to the async side.  Exits cleanly if the receiver drops.
            while let Ok(msg) = rx_sync.recv() {
                if tx_bridge.send(msg).is_err() {
                    break;
                }
            }
        });

        // Async dispatch loop — each message gets its own spawned task so that
        // one slow Ollama request never blocks subsequent ones.
        while let Some(msg) = rx_bridge.recv().await {
            // Heartbeat payloads are bookkeeping data — skip them to avoid
            // hammering Ollama with JSON it doesn't need to process.
            if msg.from == "heartbeat" {
                continue;
            }

            let bus_ref = ollama_bus.clone();
            let url = ollama_url_clone.clone();
            let model = ollama_model_clone.clone();

            tokio::spawn(async move {
                handle_ollama_message(msg, &bus_ref, &url, &model).await;
            });
        }
    });
    info!(
        "Ollama handler spawned — model='{}' url='{}'",
        ollama_model, ollama_url
    );

    // Heartbeat loop
    let mut interval = interval(TokioDuration::from_secs(config.heartbeat.interval_seconds));
    let mut consecutive_failures = 0;
    let max_consecutive_failures = 5;

    loop {
        interval.tick().await;
        match send_heartbeat(bus.clone()).await {
            Ok(()) => {
                info!("Heartbeat sent successfully");
                consecutive_failures = 0;
            }
            Err(e) => {
                let error_msg = format!("Failed to send heartbeat: {}", e);
                log_to_file(&error_msg);
                error!("{}", error_msg);
                consecutive_failures += 1;
                warn!(
                    "Consecutive failures: {}/{}",
                    consecutive_failures, max_consecutive_failures
                );
                if consecutive_failures >= max_consecutive_failures {
                    warn!(
                        "Maximum consecutive failures reached. Consider taking corrective action."
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_current_timestamp() {
        let timestamp = get_current_timestamp();
        assert_ne!(
            timestamp, 0,
            "Timestamp should not be zero unless there's an error"
        );
    }

    #[test]
    fn test_heartbeat_struct_creation() {
        let heartbeat = Heartbeat {
            timestamp: 1234567890,
            system_status: "Operational".to_string(),
            recent_events: vec!["Test event".to_string()],
        };
        assert_eq!(heartbeat.timestamp, 1234567890);
        assert_eq!(heartbeat.system_status, "Operational");
        assert_eq!(heartbeat.recent_events, vec!["Test event".to_string()]);
    }

    #[test]
    fn test_heartbeat_serialization() {
        let heartbeat = Heartbeat {
            timestamp: 1234567890,
            system_status: "Operational".to_string(),
            recent_events: vec!["Test event".to_string()],
        };
        let json_result = serde_json::to_string(&heartbeat);
        assert!(
            json_result.is_ok(),
            "Heartbeat should serialize to JSON without error"
        );
        let json = json_result.unwrap();
        assert!(
            json.contains("1234567890"),
            "JSON should contain the timestamp"
        );
        assert!(
            json.contains("Operational"),
            "JSON should contain the system status"
        );
        assert!(
            json.contains("Test event"),
            "JSON should contain the recent events"
        );
    }
}
