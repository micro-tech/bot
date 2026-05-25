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

// ---------------------------------------------------------------------------
// Source directory discovery
// ---------------------------------------------------------------------------

/// Walk up the directory tree looking for the project root.
/// A directory qualifies when it has `Cargo.toml` AND either
/// `config.toml` or `target/release/bot`.
fn get_source_dir() -> PathBuf {
    fn is_project_root(p: &Path) -> bool {
        p.join("Cargo.toml").exists()
            && (p.join("config.toml").exists() || p.join("target/release/bot").exists())
    }

    // 1. Walk up from CWD.
    if let Ok(cwd) = env::current_dir() {
        let mut candidate = cwd;
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
        // Last resort: binary's own directory even if incomplete.
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }

    PathBuf::from(".")
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

fn install() {
    println!("=== Installing AgentOS bot ===");

    let source_dir = get_source_dir();
    println!("Source directory : {}", source_dir.display());

    // Show which config files are present in the source dir so failures are
    // easy to diagnose.
    for name in [
        "config.toml",
        "system_manifest.md",
        ".env",
        "cert.pem",
        "key.pem",
    ] {
        if source_dir.join(name).exists() {
            println!("  [found] {}", name);
        } else {
            println!(
                "  [missing in source] {} — will use embedded/template",
                name
            );
        }
    }

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

    // Stop / remove old service; never touch existing config files.
    let _ = Command::new("systemctl").args(["stop", "bot"]).status();
    let _ = Command::new("systemctl").args(["disable", "bot"]).status();
    let _ = fs::remove_file("/etc/systemd/system/bot.service");
    let _ = fs::remove_file("/usr/local/bin/bot");

    // Deploy the bot binary.
    fs::copy(&bot_binary, "/usr/local/bin/bot").expect("Failed to copy bot binary");
    println!("Copied bot binary -> /usr/local/bin/bot");

    // Create runtime directories and fix ownership so `cobble` can always
    // read/write both locations (FileZilla uploads included).
    for dir in ["/home/cobble/bot", "/etc/bot"] {
        fs::create_dir_all(dir).unwrap_or_else(|e| eprintln!("WARNING: mkdir {}: {}", dir, e));
    }
    set_ownership("/home/cobble/bot", "cobble", "cobble");
    set_ownership("/etc/bot", "cobble", "cobble");

    // ── Tracked config files — embedded at compile time as fallback ─────────
    // include_str! bakes the file into the binary when compiled on the dev
    // machine, so the installer always has a valid default even on a fresh
    // Linux clone that hasn't been `git pull`-ed yet.
    const DEFAULT_CONFIG: &str = include_str!("../../config.toml");
    const DEFAULT_MANIFEST: &str = include_str!("../../system_manifest.md");

    deploy_tracked_file("config.toml", &source_dir, DEFAULT_CONFIG);
    deploy_tracked_file("system_manifest.md", &source_dir, DEFAULT_MANIFEST);

    // ── .env — git-ignored, create template when absent ────────────────────
    let env_src = source_dir.join(".env");
    if env_src.exists() {
        safe_copy_to_both(".env", &env_src);
    } else {
        println!("No .env in source — writing template");
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
        safe_write_to_both(".env", template.as_bytes());
        println!("  !! Edit /home/cobble/bot/.env — add your real API keys before starting.");
    }

    // ── TLS certificates — generate self-signed when absent ────────────────
    let cert_src = source_dir.join("cert.pem");
    let key_src = source_dir.join("key.pem");
    if cert_src.exists() && key_src.exists() {
        safe_copy_to_both("cert.pem", &cert_src);
        safe_copy_to_both("key.pem", &key_src);
    } else {
        println!("TLS certs not found in source — generating self-signed cert...");
        generate_self_signed_certs(&source_dir);
    }

    // ── Log files — always reset on install ────────────────────────────────
    for dir in ["/home/cobble/bot/logs", "/etc/bot/logs"] {
        fs::create_dir_all(dir).ok();
    }
    set_ownership("/home/cobble/bot/logs", "cobble", "cobble");
    set_ownership("/etc/bot/logs", "cobble", "cobble");

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
        // Logs intentionally overwritten on every install.
        let primary = format!("/home/cobble/bot/logs/{}", filename);
        let secondary = format!("/etc/bot/logs/{}", filename);
        if let Err(e) = fs::write(&primary, &log_header) {
            eprintln!("WARNING: could not write {}: {}", primary, e);
        }
        if let Err(e) = fs::write(&secondary, &log_header) {
            eprintln!("WARNING: could not write {}: {}", secondary, e);
        }
        println!("Created log: {}", primary);
    }

    // ── systemd service ─────────────────────────────────────────────────────
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
    if let Err(e) = fs::write("/etc/systemd/system/bot.service", service) {
        eprintln!("WARNING: could not write service file: {}", e);
    } else {
        println!("Created systemd service file");
    }

    let _ = Command::new("systemctl").arg("daemon-reload").status();
    let _ = Command::new("systemctl").args(["enable", "bot"]).status();
    let _ = Command::new("systemctl").args(["start", "bot"]).status();

    verify_installation();
    println!("\nInstallation complete!");
}

// ---------------------------------------------------------------------------
// File helpers  — NO pre-delete; overwrite in place
// ---------------------------------------------------------------------------

/// Copy `src` to /home/cobble/bot/<name> and /etc/bot/<name>.
/// Does NOT delete the destination first — overwrites atomically in place.
/// If the copy fails the original file is untouched.
fn safe_copy_to_both(name: &str, src: &Path) {
    for dest_dir in ["/home/cobble/bot", "/etc/bot"] {
        let dest = Path::new(dest_dir).join(name);
        match fs::copy(src, &dest) {
            Ok(_) => println!("Copied {} -> {}", name, dest.display()),
            Err(e) => eprintln!(
                "WARNING: failed to copy {} -> {}: {}",
                name,
                dest.display(),
                e
            ),
        }
    }
}

/// Write `data` to /home/cobble/bot/<name> and /etc/bot/<name>.
/// Does NOT delete the destination first — overwrites in place.
fn safe_write_to_both(name: &str, data: &[u8]) {
    for dest_dir in ["/home/cobble/bot", "/etc/bot"] {
        let dest = Path::new(dest_dir).join(name);
        match fs::write(&dest, data) {
            Ok(_) => println!("Wrote {} -> {}", name, dest.display()),
            Err(e) => eprintln!("WARNING: failed to write {}: {}", dest.display(), e),
        }
    }
}

/// Deploy a tracked config file.
/// Uses the live copy from `source_dir` if present; otherwise writes the
/// content embedded in the binary at compile time.
fn deploy_tracked_file(name: &str, source_dir: &Path, embedded: &str) {
    let src = source_dir.join(name);
    if src.exists() {
        safe_copy_to_both(name, &src);
    } else {
        println!("{} — deploying embedded default", name);
        safe_write_to_both(name, embedded.as_bytes());
        println!(
            "  !! Review /home/cobble/bot/{} and update any environment-specific settings.",
            name
        );
    }
}

/// Run `chown <user>:<group> <path>` so that the runtime user can write files
/// there directly (e.g. via FileZilla SFTP as `cobble`).
fn set_ownership(path: &str, user: &str, group: &str) {
    let owner = format!("{}:{}", user, group);
    match Command::new("chown").args(["-R", &owner, path]).status() {
        Ok(s) if s.success() => {}
        Ok(_) | Err(_) => eprintln!("WARNING: could not chown {} to {}", path, owner),
    }
}

// ---------------------------------------------------------------------------
// TLS
// ---------------------------------------------------------------------------

fn generate_self_signed_certs(source_dir: &Path) {
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
            safe_copy_to_both("cert.pem", Path::new(tmp_cert));
            safe_copy_to_both("key.pem", Path::new(tmp_key));
            // Save back so future installs can copy instead of regenerating.
            let _ = fs::copy(tmp_cert, source_dir.join("cert.pem"));
            let _ = fs::copy(tmp_key, source_dir.join("key.pem"));
            let _ = fs::remove_file(tmp_cert);
            let _ = fs::remove_file(tmp_key);
            println!("Self-signed TLS certificate generated (valid 10 years).");
            println!("  Replace with a real cert when deploying publicly.");
        }
        _ => {
            eprintln!("WARNING: openssl not found or failed — writing placeholder cert files.");
            eprintln!("  Generate certs manually with:");
            eprintln!("    openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem \\");
            eprintln!("      -days 3650 -nodes -subj '/CN=localhost'");
            eprintln!("  Then re-run the installer.");
            let placeholder = b"# Placeholder - replace with a real TLS certificate\n";
            safe_write_to_both("cert.pem", placeholder);
            safe_write_to_both("key.pem", placeholder);
        }
    }
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

fn uninstall() {
    println!("=== Uninstalling AgentOS bot ===");
    let _ = Command::new("systemctl").args(["stop", "bot"]).status();
    let _ = Command::new("systemctl").args(["disable", "bot"]).status();
    let _ = fs::remove_file("/etc/systemd/system/bot.service");
    let _ = fs::remove_file("/usr/local/bin/bot");
    // Config and logs in /home/cobble/bot and /etc/bot are intentionally kept.
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    println!("Bot uninstalled. Config and logs preserved.");
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

fn verify_installation() {
    println!("\n=== Verifying installation ===");
    let mut all_good = true;

    check_path("/usr/local/bin/bot", true, &mut all_good);

    // Tracked config files — required.
    for f in ["config.toml", "system_manifest.md"] {
        check_path(&format!("/home/cobble/bot/{}", f), true, &mut all_good);
        check_path(&format!("/etc/bot/{}", f), true, &mut all_good);
    }

    // User-edited files — present but may contain placeholder values.
    for f in [".env", "cert.pem", "key.pem"] {
        let primary = format!("/home/cobble/bot/{}", f);
        let secondary = format!("/etc/bot/{}", f);
        print_check(&primary, Path::new(&primary).exists());
        print_check(&secondary, Path::new(&secondary).exists());
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
        eprintln!("✗ {} MISSING", path);
    }
}
