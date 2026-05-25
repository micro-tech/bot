/// AgentOS Installer
/// Authors: john mcconnell john.microtech@gmail.com
/// Repository: https://github.com/micro-tech/grok-cli
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "--uninstall" {
        uninstall();
    } else {
        install();
    }
}

/// Walk up from `start` looking for a directory that contains both
/// `Cargo.toml` and either `config.toml` or `target/release/bot`.
/// Falls back to the executable's own directory, then CWD.
fn get_source_dir() -> PathBuf {
    // Helper: does a path look like the project root?
    fn is_project_root(p: &Path) -> bool {
        p.join("Cargo.toml").exists()
            && (p.join("config.toml").exists() || p.join("target/release/bot").exists())
    }

    // 1. Walk up from the current working directory.
    if let Ok(cwd) = env::current_dir() {
        let mut candidate = cwd.clone();
        loop {
            if is_project_root(&candidate) {
                return candidate;
            }
            match candidate.parent() {
                Some(p) => candidate = p.to_path_buf(),
                None => break,
            }
        }
    }

    // 2. Walk up from the installer binary's own location.
    if let Ok(exe) = env::current_exe()
        && let Some(mut candidate) = exe.parent().map(|p| p.to_path_buf())
    {
        loop {
            if is_project_root(&candidate) {
                return candidate;
            }
            match candidate.parent() {
                Some(p) => candidate = p.to_path_buf(),
                None => break,
            }
        }
        // Last resort: return the binary's own directory even if incomplete.
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }

    PathBuf::from(".")
}

fn install() {
    println!("=== Installing AgentOS bot ===");

    let source_dir = get_source_dir();
    println!("Source directory : {}", source_dir.display());

    // The bot binary must exist before we do anything else.
    let bot_binary = source_dir.join("target/release/bot");
    if !bot_binary.exists() {
        eprintln!(
            "ERROR: target/release/bot not found in {}",
            source_dir.display()
        );
        eprintln!("Build the project first, then re-run:");
        eprintln!("    cargo build --release");
        std::process::exit(1);
    }

    // Stop / remove old service, but preserve existing runtime data.
    let _ = Command::new("systemctl").args(["stop", "bot"]).status();
    let _ = Command::new("systemctl").args(["disable", "bot"]).status();
    let _ = fs::remove_file("/etc/systemd/system/bot.service");
    let _ = fs::remove_file("/usr/local/bin/bot");

    // Deploy the bot binary.
    fs::copy(&bot_binary, "/usr/local/bin/bot").expect("Failed to copy bot binary");
    println!("Copied bot binary  -> /usr/local/bin/bot");

    // Create runtime directories.
    fs::create_dir_all("/home/cobble/bot").expect("Failed to create /home/cobble/bot");
    fs::create_dir_all("/etc/bot").expect("Failed to create /etc/bot");

    // ── Copy tracked source files (fall back to embedded defaults) ─────────
    // include_str! bakes these into the binary at compile time, so they are
    // always available even on a stale or incomplete Linux clone.
    const DEFAULT_CONFIG: &str = include_str!("../../config.toml");
    const DEFAULT_MANIFEST: &str = include_str!("../../system_manifest.md");

    deploy_tracked_file("config.toml", &source_dir, DEFAULT_CONFIG);
    deploy_tracked_file("system_manifest.md", &source_dir, DEFAULT_MANIFEST);

    // ── .env  (git-ignored — create template if absent) ───────────────────
    let env_src = source_dir.join(".env");
    if env_src.exists() {
        copy_to_both(".env", &env_src);
    } else {
        println!("No .env in source — writing template .env");
        let template = concat!(
            "# AgentOS environment variables\n",
            "# Fill in real values before starting the service.\n",
            "\n",
            "GEMINI_API_KEY=your_gemini_api_key_here\n",
            "GEMINI_MODEL=gemini-2.0-flash\n",
            "\n",
            "# Optional Ollama overrides\n",
            "# OLLAMA_URL=http://localhost:11434\n",
            "# OLLAMA_MODEL=qwen3.5:0.8b\n",
            "# OLLAMA_PRELOAD=true\n",
            "# OLLAMA_KEEP_ALIVE_SECS=240\n",
        );
        write_to_both(".env", template.as_bytes());
        println!("  !! Edit /home/cobble/bot/.env and add your real API keys before starting.");
    }

    // ── TLS certificates (git-ignored — generate self-signed if absent) ───
    let cert_src = source_dir.join("cert.pem");
    let key_src = source_dir.join("key.pem");
    if cert_src.exists() && key_src.exists() {
        copy_to_both("cert.pem", &cert_src);
        copy_to_both("key.pem", &key_src);
    } else {
        println!("TLS certs not found in source — attempting self-signed cert generation...");
        generate_self_signed_certs(&source_dir);
    }

    // ── Log files ─────────────────────────────────────────────────────────
    fs::create_dir_all("/home/cobble/bot/logs").ok();
    fs::create_dir_all("/etc/bot/logs").ok();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let log_header = format!(
        "[INIT] Log created by AgentOS installer at unix timestamp {}\n",
        now
    );

    for filename in [
        "chat_log.md",
        "error_log.md",
        "bus_log.md",
        "hartbeat_log.md",
    ] {
        let primary = format!("/home/cobble/bot/logs/{}", filename);
        let secondary = format!("/etc/bot/logs/{}", filename);
        let _ = fs::remove_file(&primary);
        let _ = fs::write(&primary, &log_header);
        let _ = fs::remove_file(&secondary);
        let _ = fs::write(&secondary, &log_header);
        println!("Created log: {}", primary);
    }

    // ── systemd service ───────────────────────────────────────────────────
    let service = r#"[Unit]
Description=AgentOS Bot Service
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

    let _ = Command::new("systemctl").arg("daemon-reload").status();
    let _ = Command::new("systemctl").args(["enable", "bot"]).status();
    let _ = Command::new("systemctl").args(["start", "bot"]).status();

    verify_installation();
    println!("\nInstallation complete!");
}

/// Copy a file to both /home/cobble/bot/<f> and /etc/bot/<f>.
fn copy_to_both(name: &str, src: &Path) {
    for dest_dir in ["/home/cobble/bot", "/etc/bot"] {
        let dest = Path::new(dest_dir).join(name);
        let _ = fs::remove_file(&dest);
        match fs::copy(src, &dest) {
            Ok(_) => println!("Copied {} -> {}", name, dest.display()),
            Err(e) => eprintln!(
                "WARNING: failed to copy {} to {}: {}",
                name,
                dest.display(),
                e
            ),
        }
    }
}

/// Deploy a tracked (git-committed) config file.
/// Prefers the live copy in `source_dir`; falls back to the content embedded
/// in the binary at compile time via `include_str!`.
fn deploy_tracked_file(name: &str, source_dir: &Path, embedded: &str) {
    let src = source_dir.join(name);
    if src.exists() {
        copy_to_both(name, &src);
    } else {
        println!(
            "{} not found in source dir — deploying embedded default",
            name
        );
        write_to_both(name, embedded.as_bytes());
        println!(
            "  !! Review /home/cobble/bot/{} and update settings for your environment.",
            name
        );
    }
}

/// Write raw bytes to both runtime directories.
fn write_to_both(name: &str, data: &[u8]) {
    for dest_dir in ["/home/cobble/bot", "/etc/bot"] {
        let dest = Path::new(dest_dir).join(name);
        let _ = fs::remove_file(&dest);
        match fs::write(&dest, data) {
            Ok(_) => println!("Wrote template {} -> {}", name, dest.display()),
            Err(e) => eprintln!("WARNING: failed to write {}: {}", dest.display(), e),
        }
    }
}

/// Try to generate a self-signed certificate with openssl.
/// If openssl is not available, write placeholder files and print instructions.
fn generate_self_signed_certs(source_dir: &Path) {
    // Generate into a temp path first, then copy to both destinations.
    let tmp_cert = "/tmp/bot_cert.pem";
    let tmp_key = "/tmp/bot_key.pem";

    let status = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-keyout",
            tmp_key,
            "-out",
            tmp_cert,
            "-days",
            "3650",
            "-nodes",
            "-subj",
            "/CN=localhost/O=AgentOS/C=US",
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            // Copy generated certs to both destinations.
            copy_to_both("cert.pem", Path::new(tmp_cert));
            copy_to_both("key.pem", Path::new(tmp_key));
            // Also save back to source dir for future installs.
            let _ = fs::copy(tmp_cert, source_dir.join("cert.pem"));
            let _ = fs::copy(tmp_key, source_dir.join("key.pem"));
            let _ = fs::remove_file(tmp_cert);
            let _ = fs::remove_file(tmp_key);
            println!("Self-signed TLS certificate generated (valid 10 years).");
            println!("  Replace with a real cert when deploying publicly.");
        }
        _ => {
            eprintln!("WARNING: openssl not found or failed — writing placeholder cert files.");
            eprintln!("  Generate real certs with:");
            eprintln!("    openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem \\");
            eprintln!("      -days 3650 -nodes -subj '/CN=localhost'");
            eprintln!("  Then re-run the installer.");
            let placeholder = b"# Placeholder - replace with real TLS certificate\n";
            write_to_both("cert.pem", placeholder);
            write_to_both("key.pem", placeholder);
        }
    }
}

fn uninstall() {
    println!("=== Uninstalling AgentOS bot ===");
    let _ = Command::new("systemctl").args(["stop", "bot"]).status();
    let _ = Command::new("systemctl").args(["disable", "bot"]).status();
    let _ = fs::remove_file("/etc/systemd/system/bot.service");
    let _ = fs::remove_file("/usr/local/bin/bot");
    // Preserve /etc/bot and /home/cobble/bot — user data stays intact.
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    println!("Bot uninstalled. Config and logs preserved in /home/cobble/bot and /etc/bot.");
}

fn verify_installation() {
    println!("\n=== Verifying installation ===");
    let mut all_good = true;

    // Binary
    check_path("/usr/local/bin/bot", true, &mut all_good);

    // Config files that are tracked in git — must be present.
    for f in ["config.toml", "system_manifest.md"] {
        check_path(&format!("/home/cobble/bot/{}", f), true, &mut all_good);
        check_path(&format!("/etc/bot/{}", f), true, &mut all_good);
    }

    // Files that may be templates / placeholders — warn but don't fail.
    for f in [".env", "cert.pem", "key.pem"] {
        let primary = format!("/home/cobble/bot/{}", f);
        let secondary = format!("/etc/bot/{}", f);
        let p_exists = Path::new(&primary).exists();
        let s_exists = Path::new(&secondary).exists();
        print_check(&primary, p_exists);
        print_check(&secondary, s_exists);
        if !p_exists || !s_exists {
            eprintln!("  ↳ Edit this file with real values before starting the service.");
        }
    }

    // Logs
    for filename in [
        "chat_log.md",
        "error_log.md",
        "bus_log.md",
        "hartbeat_log.md",
    ] {
        check_path(
            &format!("/home/cobble/bot/logs/{}", filename),
            true,
            &mut all_good,
        );
        check_path(&format!("/etc/bot/logs/{}", filename), true, &mut all_good);
    }

    if all_good {
        println!("\n✓ All required files verified.");
    } else {
        eprintln!("\n✗ Some required files are missing — see above.");
    }
}

fn check_path(path: &str, required: bool, all_good: &mut bool) {
    if Path::new(path).exists() {
        println!("✓ {} exists", path);
    } else {
        eprintln!("✗ {} MISSING", path);
        if required {
            *all_good = false;
        }
    }
}

fn print_check(path: &str, exists: bool) {
    if exists {
        println!("✓ {} exists", path);
    } else {
        eprintln!("✗ {} MISSING (template expected)", path);
    }
}
