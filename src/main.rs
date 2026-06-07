use std::fs;
// use std::env;
// use std::path::Path;
// use std::process::{Command, Stdio};
use tokio;
use toml;

mod config;
mod bus;
mod io;
mod cpu;
mod hy_evo;
mod tools;
mod utils;
mod memory;
mod skills;
mod hooks;
mod bayesian;

#[tokio::main]
async fn main() {
    run_bot().await;
}

async fn run_bot() {
    // Ensure required directories exist very early (prevents panics)
    let _ = std::fs::create_dir_all("logs");
    let _ = std::fs::create_dir_all("/etc/bot/logs");
    let _ = std::fs::create_dir_all("/etc/bot");

    println!("Bot is running...");

    // Try multiple locations for config.toml
    let config_paths = [
        "config.toml",
        "/etc/bot/config.toml",
        "/usr/local/etc/bot/config.toml",
    ];

    let config_str = config_paths
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();

    if config_str.is_empty() {
        eprintln!("Warning: Could not find config.toml in any standard location.");
    }

    let bus = std::sync::Arc::new(crate::bus::Bus::new());

    // Parse port from config
    let port: u16 = toml::from_str::<toml::Value>(&config_str)
        .ok()
        .and_then(|v| v.get("web")?.get("port")?.as_integer()?.try_into().ok())
        .unwrap_or(8443);

    // ── Parse Ollama backends from config ─────────────────────────────────────
    let ollama_backends: Vec<(String, String, String)> = toml::from_str::<toml::Value>(&config_str)
        .ok()
        .and_then(|v| v.get("ollama").and_then(|o| o.as_array()).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_string();
            let url = entry.get("url")?.as_str()?.to_string();
            let model = entry.get("model")?.as_str()?.to_string();
            Some((name, url, model))
        })
        .collect();

    // ── CPU response forwarder (handles llm_response → web_interface) ────────
    {
        let bus_clone = bus.clone();
        tokio::spawn(async move {
            let rx = bus_clone.subscribe("cpu");
            println!("CPU response forwarder started (subscribed to 'cpu')");

            while let Ok(msg) = rx.recv() {
                if msg.data.contains("\"type\":\"llm_response\"") {
                    // Forward the response to the web UI
                    let payload: serde_json::Value =
                        serde_json::from_str(&msg.data).unwrap_or_default();
                    let text = payload["msg"].as_str().unwrap_or("").to_string();
                    let correlation_id = payload["correlation_id"].as_u64().unwrap_or(0);

                    let ui_msg = crate::bus::Message {
                        to: "web_interface".to_string(),
                        from: msg.from.clone(),
                        data: serde_json::json!({
                            "type": "llm_output",
                            "msg": text
                        })
                        .to_string(),
                        timestamp: crate::utils::now_ms(),
                    };

                    // Also write response to chat log
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("logs/chat_log.md")
                    {
                        use std::io::Write;
                        let _ = writeln!(f, "[{}] Bot: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), text);
                    }

                    let _ = bus_clone.publish(ui_msg);
                    println!("[CPU-Forwarder] Forwarded LLM response to web_interface");
                }
            }
        });
    }

    // ── Spawn one listener per Ollama backend ─────────────────────────────────
    for (name, url, model) in ollama_backends.clone() {
        let bus_clone = bus.clone();
        let backend_name = name.clone();

        tokio::spawn(async move {
            let topic = format!("ollama_{}", backend_name);
            let rx = bus_clone.subscribe(&topic);

            println!("Ollama listener started for {}", topic);

            while let Ok(msg) = rx.recv() {
                // Only handle chat requests
                if msg.data.contains("\"type\":\"chat_request\"") {
                    let _ = crate::io::ollama::handle_ollama_message(
                        msg,
                        &bus_clone,
                        &url,
                        &model,
                        &backend_name,
                    )
                    .await;
                }
            }
        });
    }

    // ── Spawn Gemini listener ────────────────────────────────────────────────
    {
        let bus_clone = bus.clone();
        // Resolve model: env var > config.toml [gemini] model > default
        let gemini_model: String = std::env::var("GEMINI_MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                toml::from_str::<toml::Value>(&config_str)
                    .ok()
                    .and_then(|v| {
                        v.get("gemini")?
                            .get("model")?
                            .as_str()
                            .map(|s| s.to_string())
                    })
            })
            .unwrap_or_else(|| "gemini-2.0-flash".to_string());

        tokio::spawn(async move {
            let rx = bus_clone.subscribe("gemini");
            println!("Gemini listener started (subscribed to 'gemini', model={})", gemini_model);

            while let Ok(msg) = rx.recv() {
                if msg.data.contains("\"type\":\"chat_request\"") {
                    crate::io::llm_gemini::handle_gemini_bus_message(msg, &bus_clone, &gemini_model).await;
                }
            }
        });
    }

    // Start the web server (this blocks)
    if let Err(e) = crate::io::web_server::start_web_server(bus, port, config_str).await {
        eprintln!("Failed to start web server: {}", e);
    }
}