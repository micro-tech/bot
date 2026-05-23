use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "--install" {
        install();
    } else if args.len() > 1 && args[1] == "--uninstall" {
        uninstall();
    } else {
        println!("Bot Installer");
        println!("Usage: installer --install | --uninstall");
    }
}

fn install() {
    println!("=== Installing bot ===");

    // The bot binary must exist in target/release/bot
    let bot_binary = Path::new("target/release/bot");
    if !bot_binary.exists() {
        eprintln!("ERROR: target/release/bot not found!");
        eprintln!("Please run this from the project root after building:");
        eprintln!("    cargo build -r");
        std::process::exit(1);
    }

    // Stop and remove old service
    uninstall();

    // Copy the real bot binary
    fs::copy(bot_binary, "/usr/local/bin/bot").expect("Failed to copy bot binary");
    println!("Copied bot binary to /usr/local/bin/bot");

    // Copy all support files
    fs::create_dir_all("/etc/bot").ok();
    let files = ["config.toml", "system_manifest.md", ".env", "cert.pem", "key.pem"];
    for f in &files {
        if Path::new(f).exists() {
            let _ = fs::copy(f, Path::new("/etc/bot").join(f));
            println!("Copied {}", f);
        }
    }

    // === Create logs directory and default log files ===
    fs::create_dir_all("/home/cobble/bot/logs").ok();
    fs::create_dir_all("/etc/bot/logs").ok();

    // Create default log files with initialization header
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let log_header = format!("[INIT] Log file created by AgentOS installer at unix timestamp {}\n", now);

    for filename in ["chat_log.md", "error_log.md", "bus_log.md", "hartbeat_log.md"] {
        // Primary location
        let primary = format!("/home/cobble/bot/logs/{}", filename);
        if !Path::new(&primary).exists() {
            let _ = fs::write(&primary, &log_header);
            println!("Created {}", primary);
        }

        // Secondary location
        let secondary = format!("/etc/bot/logs/{}", filename);
        if !Path::new(&secondary).exists() {
            let _ = fs::write(&secondary, &log_header);
        }
    }

    // Create systemd service with correct WorkingDirectory
    let service = r#"[Unit]
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
    fs::write("/etc/systemd/system/bot.service", service).expect("Failed to write service file");
    println!("Created systemd service file");

    // Enable and start
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    let _ = Command::new("systemctl").args(["enable", "bot"]).status();
    let _ = Command::new("systemctl").args(["start", "bot"]).status();

    println!("Bot installed and started successfully!");
}

fn uninstall() {
    let _ = Command::new("systemctl").args(["stop", "bot"]).status();
    let _ = Command::new("systemctl").args(["disable", "bot"]).status();
    let _ = fs::remove_file("/etc/systemd/system/bot.service");
    let _ = fs::remove_file("/usr/local/bin/bot");
    let _ = fs::remove_dir_all("/etc/bot");
    let _ = Command::new("systemctl").arg("daemon-reload").status();
}
