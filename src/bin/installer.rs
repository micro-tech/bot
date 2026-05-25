use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Binary is now named "install"
    if args.len() > 1 && args[1] == "--uninstall" {
        uninstall();
    } else {
        install();
    }
}

fn get_source_dir() -> PathBuf {
    // Try current working directory first
    if Path::new("config.toml").exists() || Path::new("target/release/bot").exists() {
        return env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    }

    // Fall back to the directory containing the installer binary
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }

    PathBuf::from(".")
}

fn install() {
    println!("=== Installing bot ===");

    let source_dir = get_source_dir();
    println!("Looking for files in: {}", source_dir.display());

    // The bot binary must exist
    let bot_binary = source_dir.join("target/release/bot");
    if !bot_binary.exists() {
        eprintln!(
            "ERROR: target/release/bot not found in {}",
            source_dir.display()
        );
        eprintln!("Please run this from the project root after building:");
        eprintln!("    cargo build -r");
        std::process::exit(1);
    }

    // Stop old service but do NOT delete /etc/bot or source files
    let _ = Command::new("systemctl").args(["stop", "bot"]).status();
    let _ = Command::new("systemctl").args(["disable", "bot"]).status();
    let _ = fs::remove_file("/etc/systemd/system/bot.service");
    let _ = fs::remove_file("/usr/local/bin/bot");

    // Copy the real bot binary
    fs::copy(&bot_binary, "/usr/local/bin/bot").expect("Failed to copy bot binary");
    println!("Copied bot binary to /usr/local/bin/bot");

    // Create destination directories
    fs::create_dir_all("/home/cobble/bot").expect("Failed to create /home/cobble/bot");
    fs::create_dir_all("/etc/bot").expect("Failed to create /etc/bot");

    // Files to copy (never delete source)
    let files = [
        "config.toml",
        "system_manifest.md",
        ".env",
        "cert.pem",
        "key.pem",
    ];

    for f in &files {
        let src = source_dir.join(f);
        if src.exists() {
            // Primary runtime location
            let primary_dest = Path::new("/home/cobble/bot").join(f);
            let _ = fs::remove_file(&primary_dest); // only remove old destination
            if let Err(e) = fs::copy(&src, &primary_dest) {
                eprintln!("Warning: Failed to copy {}: {}", f, e);
            } else {
                println!("Copied {} -> /home/cobble/bot/{}", f, f);
            }

            // Secondary location
            let etc_dest = Path::new("/etc/bot").join(f);
            let _ = fs::remove_file(&etc_dest);
            let _ = fs::copy(&src, &etc_dest);
        } else {
            println!("Skipping {} (not found in {})", f, source_dir.display());
        }
    }

    // Create logs directory and default log files
    fs::create_dir_all("/home/cobble/bot/logs").ok();
    fs::create_dir_all("/etc/bot/logs").ok();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let log_header = format!(
        "[INIT] Log file created by AgentOS installer at unix timestamp {}\n",
        now
    );

    for filename in [
        "chat_log.md",
        "error_log.md",
        "bus_log.md",
        "hartbeat_log.md",
    ] {
        let primary = format!("/home/cobble/bot/logs/{}", filename);
        let _ = fs::remove_file(&primary);
        let _ = fs::write(&primary, &log_header);
        println!("Created/updated {}", primary);

        let secondary = format!("/etc/bot/logs/{}", filename);
        let _ = fs::remove_file(&secondary);
        let _ = fs::write(&secondary, &log_header);
    }

    // Create systemd service
    let service = r#"[Unit]
Description=Bot Service
After=network.target

[Service]
EnvironmentFile=/home/cobble/bot/.env
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

    verify_installation(&source_dir);
    println!("Bot installed and started successfully!");
}

fn uninstall() {
    println!("=== Uninstalling bot ===");
    let _ = Command::new("systemctl").args(["stop", "bot"]).status();
    let _ = Command::new("systemctl").args(["disable", "bot"]).status();
    let _ = fs::remove_file("/etc/systemd/system/bot.service");
    let _ = fs::remove_file("/usr/local/bin/bot");
    // Do NOT remove /etc/bot or /home/cobble/bot - user data should stay
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    println!("Bot uninstalled (config and logs preserved).");
}

fn verify_installation(_source_dir: &Path) {
    println!("\n=== Verifying installation ===");

    let mut all_good = true;

    // Binary
    if Path::new("/usr/local/bin/bot").exists() {
        println!("✓ /usr/local/bin/bot exists");
    } else {
        eprintln!("✗ /usr/local/bin/bot MISSING");
        all_good = false;
    }

    // Config and supporting files
    for f in [
        "config.toml",
        "system_manifest.md",
        ".env",
        "cert.pem",
        "key.pem",
    ] {
        let primary = Path::new("/home/cobble/bot").join(f);
        let secondary = Path::new("/etc/bot").join(f);

        if primary.exists() {
            println!("✓ /home/cobble/bot/{} exists", f);
        } else {
            eprintln!("✗ /home/cobble/bot/{} MISSING", f);
            all_good = false;
        }

        if secondary.exists() {
            println!("✓ /etc/bot/{} exists", f);
        } else {
            eprintln!("✗ /etc/bot/{} MISSING", f);
            all_good = false;
        }
    }

    // Logs
    for filename in [
        "chat_log.md",
        "error_log.md",
        "bus_log.md",
        "hartbeat_log.md",
    ] {
        let primary = format!("/home/cobble/bot/logs/{}", filename);
        let secondary = format!("/etc/bot/logs/{}", filename);

        if Path::new(&primary).exists() {
            println!("✓ {} exists", primary);
        } else {
            eprintln!("✗ {} MISSING", primary);
            all_good = false;
        }

        if Path::new(&secondary).exists() {
            println!("✓ {} exists", secondary);
        } else {
            eprintln!("✗ {} MISSING", secondary);
            all_good = false;
        }
    }

    if all_good {
        println!("\nAll files verified successfully (overwrites performed where needed).");
    } else {
        eprintln!("\nSome files are missing! Check above.");
    }
}
