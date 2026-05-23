use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
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
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "--install" {
        install();
    } else {
        run_bot().await;
    }
}

fn install() {
    println!("=== Installing bot ===");

    let current_exe = env::current_exe().unwrap();
    let install_path = Path::new("/usr/local/bin/bot");

    // Copy binary
    if let Err(e) = fs::copy(&current_exe, &install_path) {
        eprintln!("Failed to copy bot binary: {}", e);
        return;
    }

    // Make executable
    let _ = Command::new("chmod")
        .arg("+x")
        .arg(&install_path)
        .status();

    // Create logs directory in the correct location
    let _ = fs::create_dir_all("/home/cobble/bot/logs");
    let _ = fs::create_dir_all("/etc/bot/logs");

    // Create improved systemd service file
    let service_content = r#"[Unit]
Description=Bot Service
After=network.target

[Service]
ExecStart=/usr/local/bin/bot
WorkingDirectory=/home/cobble/bot
Restart=always
User=cobble

[Install]
WantedBy=multi-user.target
"#;

    let service_path = Path::new("/etc/systemd/system/bot.service");
    if let Err(e) = fs::write(&service_path, service_content) {
        eprintln!("Failed to write service file: {}", e);
        return;
    }

    // Reload and enable service
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    let _ = Command::new("systemctl").arg("enable").arg("bot").status();
    let _ = Command::new("systemctl").arg("start").arg("bot").status();

    println!("Bot installed and started as a service.");
    println!("Logs directory created at /home/cobble/bot/logs");
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

    // Start the web server (this blocks)
    if let Err(e) = crate::io::web_server::start_web_server(bus, port, config_str).await {
        eprintln!("Failed to start web server: {}", e);
    }
}