/// Helix/OS Installer
/// Authors: john mcconnell john.microtech@gmail.com
/// Repository: https://github.com/micro-tech/grok-cli
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();

    // ─────────────────────────────────────────────────────────────────────
    // Privilege escalation (critical fix for server installs)
    // If we're not root, re-execute ourselves under sudo.
    // This prompts the user ONCE for the sudo password instead of
    // triggering repeated Polkit dialogs for every systemctl call.
    // ─────────────────────────────────────────────────────────────────────
    if !is_root() {
        println!("Not running as root — requesting sudo privileges...");
        println!("  (You should only be prompted for your password once.)\n");

        let exe = env::current_exe().expect("could not determine installer path");

        let status = Command::new("sudo")
            .arg(&exe)
            .args(&args[1..]) // forward any --uninstall etc.
            .status()
            .expect("failed to execute installer via sudo");

        std::process::exit(status.code().unwrap_or(1));
    }

    if args.len() > 1 && args[1] == "--uninstall" {
        uninstall();
    } else {
        install();
    }
}

// ---------------------------------------------------------------------------
// Privilege helpers (critical on Linux to avoid Polkit hangs)
// ---------------------------------------------------------------------------

/// Returns true if the current process is running as root (uid 0).
/// Uses the `id -u` command so we don't need extra crates.
fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok().map(|s| s.trim() == "0"))
        .unwrap_or(false)
}

/// Run a systemctl command.
///
/// main() already re-execs under sudo if we weren't root, so we are root here.
/// We keep the wrapper for logging + future-proofing.
fn run_systemctl(args: &[&str]) -> Result<std::process::ExitStatus, String> {
    let mut cmd = Command::new("systemctl");
    cmd.args(args);
    cmd.status()
        .map_err(|e| format!("failed to execute systemctl: {}", e))
}

// ---------------------------------------------------------------------------
// Dynamic user / directory helpers (the #1 reason services fail after rename)
// ---------------------------------------------------------------------------

/// Returns the non-root user the service should run as.
/// Preference order: SUDO_USER (best when run via sudo) → $USER → fallback "helix".
fn get_runtime_user() -> String {
    if let Ok(u) = std::env::var("SUDO_USER") {
        if !u.is_empty() && u != "root" {
            return u;
        }
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "helix".to_string())
}

/// Returns the primary WorkingDirectory for the service.
/// - Dedicated "helix" user → /opt/helix (clean server layout)
/// - Normal user → /home/<user>/helix
fn get_primary_runtime_dir(user: &str) -> String {
    if user == "helix" || user == "root" {
        "/opt/helix".to_string()
    } else {
        format!("/home/{}/helix", user)
    }
}

/// Convenience wrapper that prints what it's doing.
fn systemctl(args: &[&str]) {
    println!("→ systemctl {}", args.join(" "));
    match run_systemctl(args) {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!(
            "   (systemctl {} exited with status {})",
            args.join(" "),
            status
        ),
        Err(e) => eprintln!("   WARNING: {}", e),
    }
}

/// Returns the default systemd unit content (used when no helix.service is present in source).
fn create_default_service() -> String {
    let user = get_runtime_user();
    let home = get_primary_runtime_dir(&user);

    format!(
        r#"[Unit]
Description=Helix Agent Service
After=network.target

[Service]
Type=simple
User={user}
WorkingDirectory={home}
ExecStart=/usr/local/bin/helix
Restart=always
RestartSec=5
Environment=RUST_LOG=info
# Uncomment the next line if you prefer an EnvironmentFile instead of inline vars
# EnvironmentFile={home}/.env

[Install]
WantedBy=multi-user.target
"#,
        user = user,
        home = home
    )
}

// ---------------------------------------------------------------------------
// Source directory discovery
// ---------------------------------------------------------------------------

/// Walk up the directory tree looking for the project root.
/// A directory qualifies when it has `Cargo.toml` AND either
/// `config.toml` or `target/release/helix`.
fn get_source_dir() -> PathBuf {
    fn is_project_root(p: &Path) -> bool {
        p.join("Cargo.toml").exists()
            && (p.join("config.toml").exists() || p.join("target/release/helix").exists())
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
    println!(
        "=== Installing Helix (built {}) ===",
        env!("BUILD_TIMESTAMP")
    );

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

    // The helix binary must exist before we do anything else.
    let helix_binary = source_dir.join("target/release/helix");
    if !helix_binary.exists() {
        eprintln!(
            "ERROR: target/release/helix not found in {}",
            source_dir.display()
        );
        eprintln!("Build the project first, then re-run:");
        eprintln!("    cargo build --release");
        std::process::exit(1);
    }

    // Stop / remove old service using our privilege-aware helper.
    // This avoids the repeated Polkit "Authentication is required" prompts + timeouts.
    systemctl(&["stop", "helix"]);
    systemctl(&["disable", "helix"]);
    let _ = fs::remove_file("/etc/systemd/system/helix.service");
    let _ = fs::remove_file("/usr/local/bin/helix");

    // Deploy the helix binary.
    fs::copy(&helix_binary, "/usr/local/bin/helix").expect("Failed to copy helix binary");
    println!("Copied Helix binary -> /usr/local/bin/helix");

    // Determine the runtime user and primary directory dynamically.
    // This is the biggest source of "installer ran but service won't start"
    // after the bot → helix rename + moving to different servers.
    let runtime_user = get_runtime_user();
    let primary_runtime_dir = get_primary_runtime_dir(&runtime_user);

    println!("Runtime user        : {}", runtime_user);
    println!("Primary runtime dir : {}", primary_runtime_dir);

    // Create runtime directories.
    // Dual layout kept for compatibility (/primary + /etc/helix).
    // The actual WorkingDirectory is controlled by helix.service.
    for dir in [&primary_runtime_dir, "/etc/helix"] {
        fs::create_dir_all(dir).unwrap_or_else(|e| eprintln!("WARNING: mkdir {}: {}", dir, e));
    }
    set_ownership(&primary_runtime_dir, &runtime_user, &runtime_user);
    set_ownership("/etc/helix", &runtime_user, &runtime_user);

    // ── Tracked config files — embedded at compile time as fallback ─────────
    // include_str! bakes the file into the binary when compiled on the dev
    // machine, so the installer always has a valid default even on a fresh
    // Linux clone that hasn't been `git pull`-ed yet.
    const DEFAULT_CONFIG: &str = include_str!("../../config.toml");
    const DEFAULT_MANIFEST: &str = include_str!("../../.grok/docs/system_manifest_root.md");

    deploy_tracked_file("config.toml", &source_dir, DEFAULT_CONFIG, &primary_runtime_dir);
    deploy_tracked_file("system_manifest.md", &source_dir, DEFAULT_MANIFEST, &primary_runtime_dir);

    // ── .env — git-ignored, create template when absent ────────────────────
    let env_src = source_dir.join(".env");
    if env_src.exists() {
        safe_copy_to_both(".env", &env_src, &primary_runtime_dir);
    } else {
        println!("No .env in source — writing template");
        let template = concat!(
            "# Helix environment variables\n",
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
        safe_write_to_both(".env", template.as_bytes(), &primary_runtime_dir);
        println!("  !! Edit {}/.env — add your real API keys before starting.", primary_runtime_dir);
    }

    // ── TLS certificates — generate self-signed when absent ────────────────
    let cert_src = source_dir.join("cert.pem");
    let key_src = source_dir.join("key.pem");
    if cert_src.exists() && key_src.exists() {
        safe_copy_to_both("cert.pem", &cert_src, &primary_runtime_dir);
        safe_copy_to_both("key.pem", &key_src, &primary_runtime_dir);
    } else {
        println!("TLS certs not found in source — generating self-signed cert...");
        generate_self_signed_certs(&source_dir, &primary_runtime_dir);
    }

    // ── Log files — always reset on install ────────────────────────────────
    let log_primary_dir = format!("{}/logs", primary_runtime_dir);
    let log_secondary_dir = "/etc/helix/logs".to_string();

    for dir in [&log_primary_dir, &log_secondary_dir] {
        fs::create_dir_all(dir).ok();
    }
    set_ownership(&log_primary_dir, &runtime_user, &runtime_user);
    set_ownership(&log_secondary_dir, &runtime_user, &runtime_user);

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
        let primary_log = format!("{}/{}", log_primary_dir, filename);
        let secondary_log = format!("{}/{}", log_secondary_dir, filename);
        if let Err(e) = fs::write(&primary_log, &log_header) {
            eprintln!("WARNING: could not write {}: {}", primary_log, e);
        }
        if let Err(e) = fs::write(&secondary_log, &log_header) {
            eprintln!("WARNING: could not write {}: {}", secondary_log, e);
        }
        println!("Created log: {}", primary_log);
    }

    // ── systemd service ─────────────────────────────────────────────────────
    // Prefer the service file from the source tree if present (more up-to-date).
    let service_src = source_dir.join("helix.service");
    let service_content = if service_src.exists() {
        match fs::read_to_string(&service_src) {
            Ok(s) => {
                println!("Using helix.service from source tree");
                s
            }
            Err(_) => create_default_service(),
        }
    } else {
        println!("Using built-in default service definition");
        create_default_service()
    };

    if let Err(e) = fs::write("/etc/systemd/system/helix.service", &service_content) {
        eprintln!("WARNING: could not write service file: {}", e);
    } else {
        println!("Created /etc/systemd/system/helix.service");
    }

    systemctl(&["daemon-reload"]);
    systemctl(&["enable", "helix"]);
    systemctl(&["start", "helix"]);

    verify_installation(&primary_runtime_dir);
    println!("\nInstallation complete!");
}

// ---------------------------------------------------------------------------
// File helpers  — NO pre-delete; overwrite in place
// ---------------------------------------------------------------------------

/// Copy `src` to <primary_runtime_dir>/<name> and /etc/helix/<name>.
/// Does NOT delete the destination first — overwrites in place.
fn safe_copy_to_both(name: &str, src: &Path, primary_runtime_dir: &str) {
    let src_canonical = src.canonicalize().ok();

    for dest_dir in [primary_runtime_dir, "/etc/helix"] {
        let dest = Path::new(dest_dir).join(name);

        if let Some(ref sc) = src_canonical {
            if dest.canonicalize().ok().as_ref() == Some(sc) {
                println!("Skipped {} (already in place at {})", name, dest.display());
                continue;
            }
        }

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

/// Write `data` to <primary>/<name> and /etc/helix/<name>.
/// Does NOT delete the destination first — overwrites in place.
fn safe_write_to_both(name: &str, data: &[u8], primary_runtime_dir: &str) {
    for dest_dir in [primary_runtime_dir, "/etc/helix"] {
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
fn deploy_tracked_file(name: &str, source_dir: &Path, embedded: &str, primary_runtime_dir: &str) {
    let src = source_dir.join(name);
    if src.exists() {
        safe_copy_to_both(name, &src, primary_runtime_dir);
    } else {
        println!("{} — deploying embedded default", name);
        safe_write_to_both(name, embedded.as_bytes(), primary_runtime_dir);
        println!(
            "  !! Review {}/{} and update any environment-specific settings.",
            primary_runtime_dir, name
        );
    }
}

/// Run `chown <user>:<group> <path>` so that the runtime user can write files
/// there directly (e.g. via SFTP).
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

fn generate_self_signed_certs(source_dir: &Path, primary_runtime_dir: &str) {
    let tmp_cert = "/tmp/helix_cert.pem";
    let tmp_key = "/tmp/helix_key.pem";

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
            safe_copy_to_both("cert.pem", Path::new(tmp_cert), primary_runtime_dir);
            safe_copy_to_both("key.pem", Path::new(tmp_key), primary_runtime_dir);
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
            safe_write_to_both("cert.pem", placeholder, primary_runtime_dir);
            safe_write_to_both("key.pem", placeholder, primary_runtime_dir);
        }
    }
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

fn uninstall() {
    println!("=== Uninstalling Helix ===");

    // Use privilege-aware helper (we are already root because of the early sudo re-exec).
    systemctl(&["stop", "helix"]);
    systemctl(&["disable", "helix"]);

    let _ = fs::remove_file("/etc/systemd/system/helix.service");
    let _ = fs::remove_file("/usr/local/bin/helix");

    // Config and logs are intentionally kept (in the primary dir chosen by helix.service).
    systemctl(&["daemon-reload"]);

    println!("Helix uninstalled. Config and logs preserved.");
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

fn verify_installation(primary_runtime_dir: &str) {
    println!("\n=== Verifying installation ===");
    let mut all_good = true;

    check_path("/usr/local/bin/helix", true, &mut all_good);

    // Tracked config files — required.
    for f in ["config.toml", "system_manifest.md"] {
        check_path(&format!("{}/{}", primary_runtime_dir, f), true, &mut all_good);
        check_path(&format!("/etc/helix/{}", f), true, &mut all_good);
    }

    // User-edited files — present but may contain placeholder values.
    for f in [".env", "cert.pem", "key.pem"] {
        let primary = format!("{}/{}", primary_runtime_dir, f);
        let secondary = format!("/etc/helix/{}", f);
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
            &format!("{}/logs/{}", primary_runtime_dir, filename),
            true,
            &mut all_good,
        );
        check_path(
            &format!("/etc/helix/logs/{}", filename),
            true,
            &mut all_good,
        );
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
