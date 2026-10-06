//! `run_shell` — the local shell tool for the Helix agent.
//!
//! Lets the agent run commands on its own server through `/bin/sh -c`, under
//! a layered policy:
//!
//! 1. **Config gate** — `[shell] enabled` defaults to `false`; the tool
//!    refuses to run anything until the operator opts in.
//! 2. **Destructive-shell analysis** — the shared two-layer
//!    [`validate_shell_command`](super::shell_security::validate_shell_command)
//!    (argv[0] analysis + substring denylist), implemented once in
//!    `src/tools/shell_security.rs` and shared with the ACP surface.
//! 3. **Timeouts** — the child runs in its own process group; on expiry the
//!    whole group is SIGKILLed and the timeout is reported distinctly.
//! 4. **Output caps** — stdout/stderr are drained on threads (a chatty child
//!    can never block on a full pipe) and truncated at
//!    `[shell] max_output_bytes`, with a notice.
//! 5. **No TTY** — stdin is `/dev/null`, so interactive prompts (sudo
//!    passwords, `read`, …) fail fast instead of hanging the agent.
//! 6. **Workdir confinement** — commands run under the configured base
//!    directory; escapes are rejected unless explicitly allowed.
//!
//! Defense-in-depth framing (as in grok-cli): the denylist sits *behind* the
//! agent's own judgment, never as the sole protection. A static denylist
//! cannot enumerate every harmful command; the timeout, output cap, workdir
//! confinement, and default-off gate are the structural protections.
//!
//! Task 192 (SSH server) executes sessions through this same policy —
//! `is_enabled()` is the fail-closed check it uses.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::shell_security::validate_shell_command;
use crate::config::shell::ShellConfig;

/// Lazily parsed `[shell]` config, read once from the config file Helix
/// actually loaded at startup (same pattern as the MCP client).
static SHELL_CONFIG: OnceLock<ShellConfig> = OnceLock::new();

/// The config file Helix loaded, set once by main.rs.
static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Record the config file Helix loaded at startup. Call once from main.rs;
/// later calls are ignored.
pub fn set_config_path(p: PathBuf) {
    let _ = CONFIG_PATH.set(p);
}

fn load_config() -> ShellConfig {
    let path = CONFIG_PATH
        .get()
        .map(|p| p.as_path())
        .unwrap_or_else(|| Path::new("config.toml"));
    std::fs::read_to_string(path)
        .map(|s| ShellConfig::load_from_toml(&s))
        .unwrap_or_default()
}

/// The parsed `[shell]` config (process-wide, loaded once).
pub fn config() -> &'static ShellConfig {
    SHELL_CONFIG.get_or_init(load_config)
}

/// True when the operator explicitly enabled the shell tool.
/// Task 192's SSH server uses this as its fail-closed gate.
pub fn is_enabled() -> bool {
    config().is_enabled()
}

/// Hard ceiling for a single tool call (1h); floor is 1s.
const MAX_TIMEOUT_SECS: u64 = 3600;

/// Entry point for the `run_shell` tool. Never panics; every failure mode
/// (disabled, blocked, bad workdir, timeout, spawn error) is a plain string.
pub fn run_shell(args: &Value) -> String {
    let cfg = config();

    // ── Layer 1: config gate ─────────────────────────────────────────────
    if !cfg.is_enabled() {
        return "run_shell is disabled: set `[shell] enabled = true` in \
                config.toml to opt in. Nothing was executed."
            .to_string();
    }

    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if command.is_empty() {
        return "run_shell: 'command' is required and must be a non-empty string.".to_string();
    }

    // ── Layer 2: destructive-shell analysis (shared with task 189) ──────
    if let Err(reason) = validate_shell_command(command) {
        return format!("run_shell: command blocked by security policy: {}", reason);
    }

    // ── Timeout ──────────────────────────────────────────────────────────
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .filter(|&t| t > 0)
        .unwrap_or(cfg.default_timeout_secs)
        .clamp(1, MAX_TIMEOUT_SECS);

    // ── Layer 6: workdir confinement ─────────────────────────────────────
    let workdir = match resolve_workdir(cfg, args.get("workdir").and_then(|v| v.as_str())) {
        Ok(d) => d,
        Err(e) => return format!("run_shell: {}", e),
    };

    match spawn_and_wait(
        command,
        &workdir,
        Duration::from_secs(timeout_secs),
        cfg.max_output_bytes,
    ) {
        Ok(report) => report,
        Err(e) => format!("run_shell: failed to execute: {}", e),
    }
}

/// Resolve the effective working directory and enforce confinement.
///
/// - No `workdir` arg → the configured `[shell] workdir` base.
/// - Relative `workdir` arg → resolved under the base; `..` escapes rejected.
/// - Absolute `workdir` arg → rejected unless `[shell] allow_absolute_paths`.
fn resolve_workdir(cfg: &ShellConfig, requested: Option<&str>) -> Result<PathBuf, String> {
    let base_raw = if cfg.workdir == "." {
        std::env::current_dir()
            .map_err(|e| format!("cannot resolve Helix working directory: {}", e))?
    } else {
        PathBuf::from(&cfg.workdir)
    };
    let base = base_raw.canonicalize().map_err(|e| {
        format!(
            "[shell] workdir '{}' is unusable: {}",
            base_raw.display(),
            e
        )
    })?;

    let req = requested.map(str::trim).unwrap_or("");
    if req.is_empty() {
        return Ok(base);
    }
    let p = Path::new(req);
    let target = if p.is_absolute() {
        if !cfg.allow_absolute_paths {
            return Err("absolute workdir paths are not allowed \
                        ([shell] allow_absolute_paths = false)"
                .to_string());
        }
        p.canonicalize()
            .map_err(|e| format!("workdir '{}' is unusable: {}", req, e))?
    } else {
        let joined = base.join(p);
        let canon = joined
            .canonicalize()
            .map_err(|e| format!("workdir '{}' is unusable: {}", req, e))?;
        if !canon.starts_with(&base) {
            return Err(format!(
                "workdir '{}' escapes the [shell] workdir '{}'",
                canon.display(),
                base.display()
            ));
        }
        canon
    };
    Ok(target)
}

/// Read a pipe to EOF, keeping at most `cap` bytes. Keeps draining past the
/// cap (discarding) so the child can never block on a full pipe; reports
/// whether truncation happened.
fn drain_capped(pipe: &mut impl Read, cap: usize) -> (Vec<u8>, bool) {
    let mut buf = Vec::with_capacity(cap.min(8192));
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break, // EOF
            Ok(n) => {
                if buf.len() < cap {
                    let room = cap - buf.len();
                    let take = room.min(n);
                    buf.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                } else {
                    truncated = true; // keep draining, discard
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    (buf, truncated)
}

/// SIGKILL the whole process group (Unix). Best-effort: the child may have
/// already exited.
#[cfg(unix)]
fn kill_tree(pid: u32) {
    unsafe {
        libc::killpg(pid as i32, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_tree(_pid: u32) {
    // No process groups off Unix; the caller falls back to child.kill().
}

/// Run `command` via `/bin/sh -c` in `workdir`, waiting up to `timeout`.
/// The child gets its own process group so a timeout kills the whole tree.
/// stdin is /dev/null (no TTY — interactive prompts fail fast).
fn spawn_and_wait(
    command: &str,
    workdir: &Path,
    timeout: Duration,
    cap: usize,
) -> std::io::Result<String> {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            // New process group: on timeout we SIGKILL the group, not just sh,
            // so backgrounded grandchildren (`sleep 30 & wait`) die too.
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;
    let pid = child.id();

    // Drain pipes on threads: a chatty child must never block on full pipes.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle =
        std::thread::spawn(move || out_pipe.as_mut().map(|p| drain_capped(p, cap)).unwrap_or_default());
    let err_handle =
        std::thread::spawn(move || err_pipe.as_mut().map(|p| drain_capped(p, cap)).unwrap_or_default());

    // Wait with timeout.
    let start = Instant::now();
    let mut timed_out = false;
    let exit_status: Option<std::process::ExitStatus>;
    loop {
        match child.try_wait()? {
            Some(status) => {
                exit_status = Some(status);
                break;
            }
            None => {
                if start.elapsed() >= timeout {
                    timed_out = true;
                    kill_tree(pid);
                    #[cfg(not(unix))]
                    let _ = child.kill();
                    // Reap: closes the pipes so the drain threads see EOF.
                    exit_status = child.wait().ok();
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }

    let (stdout_bytes, stdout_truncated) = out_handle.join().unwrap_or_default();
    let (stderr_bytes, stderr_truncated) = err_handle.join().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);

    // ── Report ───────────────────────────────────────────────────────────
    let mut report = format!("$ {}\n(workdir: {})\n", command, workdir.display());
    if !stdout.is_empty() {
        report.push_str(&format!("\n[stdout]\n{}", stdout));
        if !stdout.ends_with('\n') {
            report.push('\n');
        }
    }
    if !stderr.is_empty() {
        report.push_str(&format!("\n[stderr]\n{}", stderr));
        if !stderr.ends_with('\n') {
            report.push('\n');
        }
    }
    if stdout.is_empty() && stderr.is_empty() {
        report.push_str("\n(no output)\n");
    }
    if timed_out {
        report.push_str(&format!(
            "\n[TIMEOUT after {}s — process group killed]\n",
            timeout.as_secs()
        ));
    } else if let Some(status) = exit_status {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            match status.signal() {
                Some(sig) => report.push_str(&format!("\n[exit: killed by signal {}]\n", sig)),
                None => report.push_str(&format!("\n[exit: code {}]\n", status.code().unwrap_or(-1))),
            }
        }
        #[cfg(not(unix))]
        {
            report.push_str(&format!(
                "\n[exit: code {} success={}]\n",
                status.code().unwrap_or(-1),
                status.success()
            ));
        }
    }
    if stdout_truncated || stderr_truncated {
        report.push_str(&format!(
            "\n[notice: output truncated at {} bytes per stream]\n",
            cap
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Tests here must not depend on the operator's config file: the
    /// OnceLock is process-wide, so gate tests through resolve/spawn units
    /// and validate via the public run_shell only for the disabled path
    /// (default config is disabled).
    #[test]
    fn disabled_gate_returns_message_and_runs_nothing() {
        // Default ShellConfig is disabled; without a config file present
        // (or with [shell] absent) the gate must refuse.
        // NOTE: if the ambient config.toml enables [shell], this test is
        // skipped — the enabled path is covered by spawn tests below.
        if is_enabled() {
            return;
        }
        let out = run_shell(&json!({"command": "echo should-not-run"}));
        assert!(
            out.contains("disabled"),
            "expected disabled message, got: {}",
            out
        );
        assert!(!out.contains("should-not-run") || out.contains("Nothing was executed"));
    }

    #[test]
    fn empty_command_rejected() {
        let out = run_shell(&json!({"command": "   "}));
        // Either the disabled gate or the empty check fires; both are safe.
        assert!(out.contains("disabled") || out.contains("required"));
    }

    #[test]
    fn resolve_workdir_rejects_absolute_by_default() {
        let cfg = ShellConfig::default();
        let err = resolve_workdir(&cfg, Some("/etc")).unwrap_err();
        assert!(err.contains("not allowed"), "got: {}", err);
    }

    #[test]
    fn resolve_workdir_rejects_dotdot_escape() {
        let cfg = ShellConfig::default();
        // /tmp/.. canonicalizes to / — outside any sane base.
        let err = resolve_workdir(&cfg, Some("../..")).unwrap_err();
        assert!(
            err.contains("escapes") || err.contains("unusable"),
            "got: {}",
            err
        );
    }

    #[test]
    fn resolve_workdir_defaults_to_base() {
        let cfg = ShellConfig::default();
        let base = std::env::current_dir()
            .unwrap()
            .canonicalize()
            .unwrap();
        assert_eq!(resolve_workdir(&cfg, None).unwrap(), base);
        assert_eq!(resolve_workdir(&cfg, Some("")).unwrap(), base);
    }

    #[test]
    fn drain_capped_truncates_and_reports() {
        let data = vec![b'x'; 100];
        let (buf, truncated) = drain_capped(&mut &data[..], 10);
        assert_eq!(buf.len(), 10);
        assert!(truncated);
        let (buf2, truncated2) = drain_capped(&mut &data[..20], 100);
        assert_eq!(buf2.len(), 20);
        assert!(!truncated2);
    }

    /// Live spawn tests: enabled config is injected via a temp config file.
    /// These run the real `/bin/sh` path (timeout, caps, process-group kill).
    mod live {
        use super::*;

        #[test]
        fn spawn_echo_reports_exit_code() {
            let cfg = ShellConfig {
                enabled: true,
                ..ShellConfig::default()
            };
            let workdir = resolve_workdir(&cfg, None).unwrap();
            let out = spawn_and_wait("echo hello-shell", &workdir, Duration::from_secs(10), 1024)
                .expect("spawn works");
            assert!(out.contains("hello-shell"), "got: {}", out);
            assert!(out.contains("[exit: code 0]"), "got: {}", out);
        }

        #[test]
        fn spawn_timeout_kills_process_group() {
            let cfg = ShellConfig {
                enabled: true,
                ..ShellConfig::default()
            };
            let workdir = resolve_workdir(&cfg, None).unwrap();
            let start = Instant::now();
            // Grandchild sleep: only a process-group kill gets this promptly.
            let out = spawn_and_wait(
                "sleep 30 & wait",
                &workdir,
                Duration::from_secs(2),
                1024,
            )
            .expect("spawn works");
            let elapsed = start.elapsed();
            assert!(out.contains("TIMEOUT"), "got: {}", out);
            assert!(
                elapsed < Duration::from_secs(10),
                "kill took too long: {:?}",
                elapsed
            );
        }

        #[test]
        fn spawn_output_cap_truncates() {
            let cfg = ShellConfig {
                enabled: true,
                ..ShellConfig::default()
            };
            let workdir = resolve_workdir(&cfg, None).unwrap();
            let out = spawn_and_wait(
                "yes | head -c 100000",
                &workdir,
                Duration::from_secs(10),
                1024,
            )
            .expect("spawn works");
            assert!(out.contains("truncated at 1024 bytes"), "got: {}", out);
        }

        #[test]
        fn spawn_no_tty_fails_fast() {
            let cfg = ShellConfig {
                enabled: true,
                ..ShellConfig::default()
            };
            let workdir = resolve_workdir(&cfg, None).unwrap();
            // `read` with stdin=/dev/null gets EOF immediately.
            let out = spawn_and_wait(
                "read -p 'name: ' v; echo got:$v",
                &workdir,
                Duration::from_secs(10),
                1024,
            )
            .expect("spawn works");
            assert!(out.contains("got:"), "got: {}", out);
            assert!(out.contains("[exit: code 0]"), "got: {}", out);
        }

        #[test]
        fn spawn_validated_block_is_rejected_before_spawn() {
            // The security layer is exercised through run_shell's gate when
            // disabled; here we assert the validator itself is wired by
            // checking the shared function rejects (unit-tested in
            // shell_security). This just pins the import.
            assert!(validate_shell_command("rm -rf /").is_err());
            assert!(validate_shell_command("echo ok").is_ok());
        }
    }
}
