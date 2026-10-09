#![allow(dead_code)]

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
mod planning;
mod reasoning;
mod agents;
mod okf;
mod mcp_client;
mod router;
mod acp;
mod a2a;
mod ssh;
mod cron;

#[tokio::main]
async fn main() {
    // `helix acp` — ACP server mode (Zed / ACP clients drive Helix over
    // stdio). Handled before normal startup; see run_acp_server().
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("acp") {
        run_acp_server().await;
        return;
    }

    // Load .env early (for GEMINI_API_KEY etc.)
    let _ = dotenv::dotenv();

    // Permanently fix rustls CryptoProvider (ring) at the absolute earliest point.
    // This resolves conflicts caused by lettre + imap pulling in aws-lc-rs.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install global rustls crypto provider (ring)");

    run_helix().await;
}

/// `helix acp` entry point: run the ACP server on stdio.
///
/// Opt-in via the `[acp]` config section (`enabled = true`). When disabled
/// (the default) this prints a hint and exits — an ACP server is a
/// remote-control surface and must never start unasked.
///
/// Logging goes to stderr so the JSON-RPC stream on stdout stays clean.
async fn run_acp_server() {
    // Same config resolution as run_helix: first match wins.
    let config_paths = [
        "config.toml",
        "/etc/helix/config.toml",
        "/usr/local/etc/helix/config.toml",
    ];
    let config_str = config_paths
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();

    let acp_cfg = crate::config::acp::AcpConfig::load_from_toml(&config_str);
    if !acp_cfg.is_enabled() {
        eprintln!("ACP server is disabled. Enable it with [acp] enabled = true in config.toml.");
        std::process::exit(2);
    }

    eprintln!(
        "Helix ACP server starting (max_steps={}) — waiting for client on stdio…",
        acp_cfg.max_steps
    );
    let agent = crate::acp::HelixAcpAgent::new(acp_cfg.max_steps);
    if let Err(e) = crate::acp::server::run_acp_stdio(agent).await {
        eprintln!("ACP server error: {:#}", e);
        std::process::exit(1);
    }
}

/// Directory Helix considers its runtime home: the parent of the config file
/// it actually loaded (task 188). Heartbeat/cron files land here, never in
/// a CWD-relative path that may not be the live one.
fn runtime_dir(config_path_used: &str) -> std::path::PathBuf {
    if config_path_used == "none" {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    } else {
        std::path::Path::new(config_path_used)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }
}

async fn run_helix() {
    // Ensure required directories exist very early (prevents panics)
    let _ = std::fs::create_dir_all("logs");
    let _ = std::fs::create_dir_all("/etc/helix/logs");
    let _ = std::fs::create_dir_all("/etc/helix");

    println!("Helix is running...");

    // Try multiple locations for config.toml
    let config_paths = [
        "config.toml",
        "/etc/helix/config.toml",
        "/usr/local/etc/helix/config.toml",
    ];

    let config_str = config_paths
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();

    let config_path_used = config_paths
        .iter()
        .find(|p| fs::read_to_string(p).is_ok())
        .copied()
        .unwrap_or("none");

    println!("Using config file: {}", config_path_used);

    // Dual-deploy coherence: the installer keeps ./config.toml (the primary,
    // under the service WorkingDirectory) and /etc/helix/config.toml as
    // byte-identical copies, primary winning on first-exists-wins. If they
    // ever diverge, say so loudly instead of silently running on one while
    // the operator edits the other.
    if config_path_used == "config.toml" {
        let primary = fs::read("config.toml");
        let fallback = fs::read("/etc/helix/config.toml");
        if let (Ok(a), Ok(b)) = (primary, fallback) {
            if a != b {
                eprintln!(
                    "WARNING: ./config.toml and /etc/helix/config.toml differ — \
                     using ./config.toml (first match); the /etc/helix copy is stale. \
                     Reconcile them or reinstall."
                );
            }
        }
    }

    // Tell the MCP client which file is live, so it reads [mcp] from the same
    // config Helix loaded. "none" (no config anywhere) falls back to the
    // CWD-relative config.toml, matching previous behavior.
    let mcp_config_path = if config_path_used == "none" {
        std::path::PathBuf::from("config.toml")
    } else {
        std::path::PathBuf::from(config_path_used)
    };
    crate::mcp_client::set_config_path(mcp_config_path.clone());
    crate::tools::shell_tool::set_config_path(mcp_config_path);

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

    println!("=== Parsed Ollama backends ({} total) ===", ollama_backends.len());
    for (i, (name, url, model)) in ollama_backends.iter().enumerate() {
        println!("  [{}] name='{}' url='{}' model='{}'", i, name, url, model);
    }
    if ollama_backends.is_empty() {
        println!("WARNING: No [[ollama]] entries found in config!");
    }

    // ── Task 215: commander mode gate ([helix.commander] enabled, default true) ──
    {
        let enabled = toml::from_str::<toml::Value>(&config_str)
            .ok()
            .and_then(|v| v.get("helix")?.get("commander")?.get("enabled")?.as_bool())
            .unwrap_or(true);
        crate::tools::subagent_tool::set_commander_enabled(enabled);
        println!("commander mode: {}", if enabled { "enabled" } else { "disabled" });
    }

    // ── Task 217: daily API spend cap ([router] daily_api_cap_usd, default 5.00) ──
    {
        let cap = toml::from_str::<toml::Value>(&config_str)
            .ok()
            .and_then(|v| v.get("router")?.get("daily_api_cap_usd")?.as_float());
        if let Some(v) = cap {
            crate::router::spend::set_daily_api_cap_usd(v);
            println!("daily API spend cap: ${:.2}", v);
        }
    }

    // ── Task 214/217: register Ollama endpoints for the spawn_subagent tool ──
    // Maps [[ollama]] entries onto the commander's backend names by matching
    // the entry name. Task 217.1 role decision: "local"/"desktop" -> the fast
    // interactive box (desktop RTX 3060); "lan"/"server" -> the ollama server
    // box (overflow/embeddings). Unmatched entries fall back positionally:
    // first -> local, second -> lan.
    {
        use crate::tools::subagent_tool::{set_ollama_endpoints, OllamaEndpoint};
        let mut local: Option<OllamaEndpoint> = None;
        let mut lan: Option<OllamaEndpoint> = None;
        for (name, url, model) in &ollama_backends {
            let ep = OllamaEndpoint { url: url.clone(), model: model.clone() };
            let n = name.to_lowercase();
            if (n.contains("local") || n.contains("desktop")) && local.is_none() {
                local = Some(ep);
            } else if (n.contains("lan") || n.contains("server")) && lan.is_none() {
                lan = Some(ep);
            }
        }
        let mut iter = ollama_backends.iter();
        if local.is_none() {
            if let Some((_, url, model)) = iter.next() {
                local = Some(OllamaEndpoint { url: url.clone(), model: model.clone() });
            }
        }
        if lan.is_none() {
            if let Some((_, url, model)) = iter.next() {
                lan = Some(OllamaEndpoint { url: url.clone(), model: model.clone() });
            }
        }
        set_ollama_endpoints(local.clone(), lan.clone());
        println!(
            "spawn_subagent backends: local={} lan={}",
            local.map(|e| format!("{} {}", e.url, e.model)).unwrap_or_else(|| "(env/default)".to_string()),
            lan.map(|e| format!("{} {}", e.url, e.model)).unwrap_or_else(|| "(env/default)".to_string()),
        );
    }

    // ── Heartbeat: construct the live Cpu and drive it on a real tick loop ──
    // Task 199. John's intent: the heartbeat is the drumbeat that keeps the
    // bot busy moving along through different jobs — a scheduler tick that
    // advances queued work, NOT just a liveness ping. Until now nothing
    // ticked in production: Cpu was never constructed, handle_heartbeat had
    // zero callers, and TimeScheduler::start was an uncalled placeholder.
    {
        let heartbeat_interval_secs: u64 = toml::from_str::<toml::Value>(&config_str)
            .ok()
            .and_then(|v| v.get("heartbeat")?.get("interval_seconds")?.as_integer())
            .and_then(|i| u64::try_from(i).ok())
            .filter(|&i| i > 0)
            .unwrap_or(300);

        // Manifest: prefer the runtime dir (task 188); fall back to CWD.
        let rt_dir = runtime_dir(config_path_used);
        let manifest_candidates = [
            rt_dir.join("system_manifest.md"),
            std::path::PathBuf::from("system_manifest.md"),
        ];
        let manifest_path = manifest_candidates
            .iter()
            .find(|p| p.is_file())
            .cloned()
            .unwrap_or_else(|| manifest_candidates[0].clone());
        let manifest_path_str = manifest_path.to_string_lossy().to_string();

        let memory = memory::MemoryManager::new(1000, 500);
        let skills: Box<dyn cpu::interfaces::SkillInterface> =
            Box::new(skills::SkillRegistry::new());

        let mut ollama_router = io::ollama::OllamaRouter::new();
        for (_name, url, model) in &ollama_backends {
            ollama_router.add_backend(url.clone(), model.clone());
        }
        let ollama_router = std::sync::Arc::new(ollama_router);
        let llm = io::ollama::llm::OllamaLlm::new(ollama_router);
        let hyevo = hy_evo::integration::HyEvoIntegration::new(hy_evo::engine::HyEvoEngine::new(
            llm.clone(),
        ));
        let reasoning_config = config::reasoning::ReasoningConfig::load_from_toml(&config_str);

        match cpu::Cpu::new(
            memory,
            skills,
            llm,
            bus.clone(),
            hyevo,
            &manifest_path_str,
            reasoning_config,
        ) {
            Ok(cpu) => {
                let cpu = std::sync::Arc::new(tokio::sync::Mutex::new(cpu));
                println!(
                    "Heartbeat: live Cpu constructed (manifest: {}), ticking every {}s",
                    manifest_path_str, heartbeat_interval_secs
                );
                // Files live next to the config Helix actually loaded (task 188),
                // never in a CWD-relative path that may not be the live one.
                let hb_dir = rt_dir.clone();
                let hb_bus = bus.clone();
                let hb_interval = heartbeat_interval_secs;
                // Clones for the watchdog spawn below (the tick spawn moves
                // hb_dir/hb_bus into its async block).
                let wd_dir = hb_dir.clone();
                let wd_bus = hb_bus.clone();
                // Task 202: routine dispatch. The cron registry scheduler
                // (task 201) publishes `routine_run` to "cpu"; this task
                // delivers them to the Cpu. (The bus router broadcasts to
                // every subscriber of "cpu", so this coexists with the
                // llm_response forwarder below.)
                let routine_cpu = cpu.clone();
                let routine_bus = bus.clone();
                tokio::spawn(async move {
                    let rx = routine_bus.subscribe("cpu");
                    while let Ok(msg) = rx.recv() {
                        if !msg.data.contains("\"type\":\"routine_run\"") {
                            continue;
                        }
                        let payload: serde_json::Value =
                            serde_json::from_str(&msg.data).unwrap_or_default();
                        let routine_id = payload["routine_id"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let routine_name = payload["routine_name"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        if routine_id.is_empty() {
                            continue;
                        }
                        let cpu_clone = routine_cpu.clone();
                        tokio::spawn(async move {
                            let mut guard = cpu_clone.lock().await;
                            // Task 216: the full payload (incl. agent job) goes through.
                            guard.handle_routine_run(&payload).await;
                        });
                    }
                });
                // Consecutive beat failures, reported in heartbeat.md.
                let beat_errors = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let beat_errors_clone = beat_errors.clone();
                tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
                        heartbeat_interval_secs,
                    ));
                    // Skip the immediate first tick — startup shouldn't wait on it.
                    ticker.tick().await;
                    loop {
                        ticker.tick().await;
                        let (tick, uptime_secs) = {
                            let mut guard = cpu.lock().await;
                            guard.handle_heartbeat().await;
                            (guard.state.tick_count, guard.state.uptime.as_secs())
                        };
                        // Task 200: observability. Every beat writes
                        // heartbeat.md and appends to logs/hartbeat_log.md.
                        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
                        let errors = beat_errors_clone
                            .load(std::sync::atomic::Ordering::Relaxed);
                        let hb_md = crate::utils::format_heartbeat_md(
                            tick, &ts, uptime_secs, errors
                        );
                        if std::fs::write(hb_dir.join("heartbeat.md"), &hb_md).is_err() {
                            beat_errors_clone
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        let log_line = format!(
                            "[{}] tick={} uptime_secs={} errors={}\n",
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                            tick,
                            uptime_secs,
                            errors
                        );
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(hb_dir.join("logs/hartbeat_log.md"))
                        {
                            use std::io::Write as _;
                            if f.write_all(log_line.as_bytes()).is_err() {
                                beat_errors_clone
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        // Slow-cadence broadcast so the UI stays live without
                        // a message per beat.
                        if tick % 10 == 0 {
                            let _ = hb_bus.publish(crate::bus::Message {
                                to: "web_interface".to_string(),
                                from: "heartbeat".to_string(),
                                data: serde_json::json!({
                                    "type": "heartbeat_status",
                                    "tick": tick,
                                    "uptime_secs": uptime_secs,
                                    "errors": errors,
                                })
                                .to_string(),
                                timestamp: crate::utils::now_ms(),
                            });
                        }
                    }
                });
                // Missed-beat watchdog: if the beat file goes stale (> 3x the
                // interval), the tick loop is stuck — log loudly and alert.
                tokio::spawn(async move {
                    let mut check = tokio::time::interval(std::time::Duration::from_secs(
                        hb_interval,
                    ));
                    check.tick().await; // let beats land before first check
                    loop {
                        check.tick().await;
                        let stale = crate::utils::heartbeat_stale_secs(
                            &wd_dir.join("heartbeat.md"),
                            std::time::SystemTime::now(),
                        );
                        if crate::utils::is_heartbeat_missed(stale, hb_interval) {
                            eprintln!(
                                "Heartbeat watchdog: MISSED BEAT — last beat {:?} (threshold {}s)",
                                stale.map(|s| format!("{}s ago", s)).unwrap_or_else(|| "never".to_string()),
                                3 * hb_interval
                            );
                            let _ = wd_bus.publish(crate::bus::Message {
                                to: "web_interface".to_string(),
                                from: "heartbeat".to_string(),
                                data: serde_json::json!({
                                    "type": "heartbeat_missed",
                                    "last_beat_secs_ago": stale,
                                    "threshold_secs": 3 * hb_interval,
                                })
                                .to_string(),
                                timestamp: crate::utils::now_ms(),
                            });
                        }
                    }
                });
            }
            Err(e) => {
                eprintln!(
                    "Heartbeat: Cpu construction failed ({}); heartbeat disabled, reactive paths unaffected",
                    e
                );
            }
        }
    }

    // ── CPU response forwarder (handles llm_response → web_interface) ────────
    {
        let bus_clone = bus.clone();
        // Load logging paths (fall back to defaults)
        let chat_log_path: String = toml::from_str::<toml::Value>(&config_str)
            .ok()
            .and_then(|v| v.get("logging")?.get("chat_log")?.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "logs/chat_log.md".to_string());

        tokio::spawn(async move {
            let rx = bus_clone.subscribe("cpu");
            println!("CPU response forwarder started (subscribed to 'cpu')");

            while let Ok(msg) = rx.recv() {
                println!("[CPU-Forwarder] received message to cpu | from='{}' data_preview='{}'", msg.from, crate::utils::truncate_str(&msg.data, 120));
                if msg.data.contains("\"type\":\"llm_response\"") {
                    let payload: serde_json::Value =
                        serde_json::from_str(&msg.data).unwrap_or_default();

                    // Robust extraction: try msg, data, content, etc.
                    let text = payload["msg"]
                        .as_str()
                        .or_else(|| payload["data"].as_str())
                        .or_else(|| payload["content"].as_str())
                        .or_else(|| payload["text"].as_str())
                        .unwrap_or("")
                        .to_string();

                    if text.is_empty() {
                        println!("[CPU-Forwarder] llm_response but no text, skipping");
                        continue;
                    }

                    // Normalize the 'from' label for the UI
                    // "ollama_server" → "server", "ollama_desktop" → "desktop", "gemini" → "gemini"
                    let display_from = if msg.from.starts_with("ollama_") {
                        msg.from.strip_prefix("ollama_").unwrap_or(&msg.from).to_string()
                    } else {
                        msg.from.clone()
                    };

                    let ui_msg = crate::bus::Message {
                        to: "web_interface".to_string(),
                        from: display_from.clone(),
                        data: serde_json::json!({
                            "type": "llm_output",
                            "data": text
                        })
                        .to_string(),
                        timestamp: crate::utils::now_ms(),
                    };

                    // Write to chat log (use configured path)
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&chat_log_path)
                    {
                        use std::io::Write;
                        let _ = writeln!(f, "[{}] {}: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), display_from, text);
                    }

                    let _ = bus_clone.publish(ui_msg);
                    println!("[CPU-Forwarder] ✅ Forwarded LLM response from {} ({} chars) to web_interface", display_from, text.len());
                }
            }
        });
    }

    // ── Spawn one listener per Ollama backend ─────────────────────────────────
    for (name, url, model) in ollama_backends.clone() {
        let bus_clone = bus.clone();
        let backend_name = name.clone();
        let backend_url = url.clone();
        let backend_model = model.clone();

        tokio::spawn(async move {
            let topic = format!("ollama_{}", backend_name);
            let rx = bus_clone.subscribe(&topic);

            println!("✅ Ollama listener SUBSCRIBED for topic='{}'  url={}  model={}", topic, backend_url, backend_model);
            println!("   (waiting for messages on this bus topic...)");

            // Startup health probe with extra visibility for desktop
            let health_ok = crate::io::ollama::check_ollama_health(&backend_url).await;
            if health_ok {
                println!("[{}] startup health probe: OK ✅", topic);
            } else {
                println!("[{}] startup health probe: FAILED ❌  (Ollama not reachable at {})", topic, backend_url);
                println!("    → Desktop/remote Ollama usually needs: OLLAMA_HOST=0.0.0.0 ollama serve");
            }

            // One-time startup health probe (very useful for diagnosis)
            if crate::io::ollama::check_ollama_health(&backend_url).await {
                println!("[{}] startup health probe: OK", topic);
            } else {
                println!("[{}] startup health probe: FAILED (Ollama not reachable at this URL)", topic);
            }

            while let Ok(msg) = rx.recv() {
                println!("[{}] 📥 RECEIVED message from='{}'  preview='{}'", 
                    topic, msg.from, crate::utils::truncate_str(&msg.data, 160));
                
                // Only handle chat requests
                if msg.data.contains("\"type\":\"chat_request\"") {
                    println!("[{}] ✅ Processing chat_request...", topic);
                    let _ = crate::io::ollama::handle_ollama_message(
                        msg,
                        &bus_clone,
                        &backend_url,
                        &backend_model,
                        &backend_name,
                    )
                    .await;
                } else {
                    println!("[{}] (ignoring non-chat message)", topic);
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

    // Start the SSH server (task 192) alongside the web server. It runs as a
    // background task; fail-closed refusals are loud but don't take down
    // the rest of Helix.
    {
        let ssh_cfg = crate::config::ssh::SshConfig::load_from_toml(&config_str);
        if ssh_cfg.is_enabled() {
            // Helix-managed ssh material lives next to the config file Helix
            // actually loaded (same resolution as the web editor's path).
            let config_dir = std::path::Path::new(config_path_used)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| {
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
                });
            tokio::spawn(async move {
                if let Err(e) = crate::ssh::start_ssh_server(ssh_cfg, config_dir).await {
                    eprintln!("SSH server failed to start: {}", e);
                }
            });
        }
    }

    // Task 201: routine registry + scheduler. The registry lives in the
    // runtime dir next to the config; the scheduler fires due jobs on their
    // own schedules (no global 5s tick). The same Arc is handed to the web
    // server so the routines WS API toggles the live registry.
    let rt_dir = runtime_dir(config_path_used);
    let routines = std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::cron::registry::RoutineRegistry::load_or_seed(rt_dir.join("routines.json")),
    ));
    crate::cron::registry::spawn_scheduler(bus.clone(), routines.clone());

    // Start the web server (this blocks). The resolved live config path goes
    // along so the web config editor reads/writes the file Helix actually
    // loaded, not a hardcoded CWD-relative "config.toml".
    let web_config_path = config_path_used.to_string();
    if let Err(e) = crate::io::web_server::start_web_server(
        bus,
        port,
        config_str,
        web_config_path,
        routines,
    )
    .await
    {
        eprintln!("Failed to start web server: {}", e);
    }
}