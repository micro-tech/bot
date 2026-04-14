use dotenvy;
use log::{LevelFilter, error, info, set_boxed_logger, set_max_level, warn};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use toml;

// Include modules
mod bayesian;
mod bus;
mod config;
mod cpu;
mod hooks;
mod hy_evo;
mod io;

mod memory;
mod skills;
mod tools;
mod utils;

use crate::bus::{Bus, Message};
use crate::cpu::Cpu;
use crate::cpu::executor::CpuExecutor;
use crate::cpu::time_scheduler::TimeScheduler;
use crate::hooks::HookRegistry;
use crate::hy_evo::HyEvoIntegration;
use crate::hy_evo::engine::HyEvoEngine;
use crate::io::llm_gemini::handle_gemini_bus_message;
use crate::io::ollama::llm::OllamaLlm;
use crate::io::ollama::{check_ollama_health, handle_ollama_message};
use crate::io::web_server::start_web_server;
use crate::memory::MemoryHandle;
use crate::skills::SkillRegistry;
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
    ollama: Vec<OllamaConfig>,
    gemini: Option<GeminiConfig>,
    web: WebConfig,
    heartbeat: HeartbeatConfig,
}

#[derive(Deserialize, Clone, Debug)]
struct OllamaConfig {
    name: String,
    url: String,
    model: String,
}

#[derive(Deserialize)]
struct BotConfig {
    name: String,
}

#[derive(Deserialize)]
struct WebConfig {
    port: u16,
}

#[derive(Deserialize)]
struct HeartbeatConfig {
    interval_seconds: u64,
}

#[derive(Deserialize, Clone, Debug)]
struct GeminiConfig {
    model: String,
}

#[derive(Clone)]
struct LogMsg {
    level: String,
    msg: String,
}

#[derive(Clone)]
struct OllamaRouter {
    backends: Vec<OllamaConfig>,
}

impl OllamaRouter {
    fn new(config: &Config) -> Self {
        Self {
            backends: config.ollama.clone(),
        }
    }

    fn get_by_name(&self, name: &str) -> Option<&OllamaConfig> {
        self.backends.iter().find(|b| b.name == name)
    }

    fn default(&self) -> Option<&OllamaConfig> {
        self.backends.first()
    }
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

#[tokio::main]
async fn main() {
    // Load .env file so GEMINI_API_KEY and other secrets are available via std::env::var.
    // .ok() silences the error when the file is absent (e.g. in CI / production where
    // env-vars are injected directly).
    dotenvy::dotenv().ok();

    fs::create_dir_all("logs").expect("Failed to create logs dir");

    // 1. Load config FIRST
    let config_str = fs::read_to_string("config.toml").expect("Failed to read config.toml");
    let config: Config = toml::from_str(&config_str).expect("Failed to parse config.toml");

    // 2. Build Ollama router AFTER config is loaded
    let router = Arc::new(OllamaRouter::new(&config));

    // 3. Create bus
    let bus = Arc::new(Bus::new());

    // 4. Set up logger
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
            println!(
                "Publishing log to bus: {}",
                &msg.data[..50.min(msg.data.len())]
            );
            let _ = bus_clone.publish(msg);
        }
    });

    info!("Starting bot with HTTPS web interface and Ollama chat");

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

    // Spawn one Ollama handler per configured backend.
    // Each handler subscribes to "ollama_{name}" so the web_server can route
    // to the correct backend by channel name (e.g. "ollama_server", "ollama_local3090").
    for backend in config.ollama.iter() {
        let channel_name = format!("ollama_{}", backend.name);
        let bus_clone = bus.clone();
        let url = backend.url.clone();
        let model = backend.model.clone();
        let name = backend.name.clone();

        tokio::spawn(async move {
            // Startup health check — advisory only, per-request retries handle
            // transient failures when real messages arrive.
            if check_ollama_health(&url).await {
                info!("Ollama '{}' reachable at {} ✅", name, url);
            } else {
                warn!(
                    "Ollama '{}' NOT reachable at {} on startup ⚠️ — will retry per request",
                    name, url
                );
            }

            // Bridge: sync bus receiver → async tokio channel so individual LLM
            // requests are processed concurrently without blocking the runtime.
            let (tx_bridge, mut rx_bridge) = tokio::sync::mpsc::unbounded_channel::<Message>();
            let rx_sync = bus_clone.subscribe(&channel_name);

            tokio::task::spawn_blocking(move || {
                while let Ok(msg) = rx_sync.recv() {
                    if tx_bridge.send(msg).is_err() {
                        break;
                    }
                }
            });

            while let Some(msg) = rx_bridge.recv().await {
                if msg.from == "heartbeat" {
                    continue;
                }
                let bus_ref = bus_clone.clone();
                let url_c = url.clone();
                let model_c = model.clone();
                let name_c = name.clone();
                tokio::spawn(async move {
                    handle_ollama_message(msg, &bus_ref, &url_c, &model_c, &name_c).await;
                });
            }
        });
        info!(
            "Ollama handler '{}' spawned on bus channel 'ollama_{}'",
            backend.name, backend.name
        );
    }

    // Resolve Gemini model: GEMINI_MODEL env var > config.toml [gemini] model > built-in default.
    let gemini_model = std::env::var("GEMINI_MODEL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| config.gemini.as_ref().map(|g| g.model.clone()))
        .unwrap_or_else(|| "gemini-2.0-flash".to_string());
    info!("Gemini model: '{}'", gemini_model);

    // Spawn Gemini handler — subscribes to the "gemini" bus channel.
    let gemini_bus = bus.clone();
    let gemini_model_clone = gemini_model.clone();
    tokio::spawn(async move {
        let (tx_bridge, mut rx_bridge) = tokio::sync::mpsc::unbounded_channel::<Message>();
        let rx_sync = gemini_bus.subscribe("gemini");
        tokio::task::spawn_blocking(move || {
            while let Ok(msg) = rx_sync.recv() {
                if tx_bridge.send(msg).is_err() {
                    break;
                }
            }
        });
        while let Some(msg) = rx_bridge.recv().await {
            if msg.from == "heartbeat" {
                continue;
            }
            let bus_ref = gemini_bus.clone();
            let model_ref = gemini_model_clone.clone();
            tokio::spawn(async move {
                handle_gemini_bus_message(msg, &bus_ref, &model_ref).await;
            });
        }
    });
    info!(
        "Gemini handler spawned on bus channel 'gemini' (model='{}')",
        gemini_model
    );

    // ─────────────────────────────────────────────────────────────
    // Build CPU + start heartbeat scheduler
    // ─────────────────────────────────────────────────────────────

    // Build subsystems
    let memory = crate::memory::MemoryManager::new(50, 1000);
    let skills = Box::new(SkillRegistry::new()) as Box<dyn crate::cpu::interfaces::SkillInterface>;
    let llm = OllamaLlm::new(router.clone());

    // Build HyEvo integration
    let engine = HyEvoEngine::new(llm.clone());
    let hyevo = HyEvoIntegration::new(engine);

    // Build CPU
    let cpu = Arc::new(Mutex::new(
        Cpu::new(
            memory,
            skills,
            llm,
            bus.clone(),
            hyevo,
            "system_manifest.md",
        )
        .unwrap(),
    ));
    let cpu_bus = bus.clone();
    let cpu_instance = cpu.clone();

    tokio::spawn(async move {
        let rx = cpu_bus.subscribe("cpu");

        while let Ok(msg) = rx.recv() {
            cpu_instance.lock().unwrap().handle_bus_message(msg);
        }
    });

    // Start time-based heartbeat scheduler (blocks until shutdown)
    TimeScheduler::start(cpu.clone(), 1000).await;
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
