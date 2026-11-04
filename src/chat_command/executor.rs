//! Read-only command execution (tasks 298/304).
//!
//! No shell, ever, on this path. The command line is split on
//! whitespace, every token is screened for metacharacters, argv[0]
//! must be allowlisted, and the process is spawned with
//! `std::process::Command` directly. `run_shell` is deliberately NOT
//! used here — it runs via a shell and allows writes, so it cannot
//! enforce read-only.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Characters that must never appear in any token. Covers pipes,
/// redirection, command substitution, chaining, globbing, tilde
/// expansion, quoting, and subshell/grouping syntax. A token with one
/// of these is rejected whole — there is no escaping or quoting in v1.
pub const METACHARS: &[char] = &[
    '|', '>', '<', ';', '&', '$', '`', '"', '\'', '*', '?', '~', '(', ')', '{', '}',
];

/// Hardcoded LOCAL allowlist (code, NOT config — a config edit must
/// never silently widen remote execution; allowlist changes go through
/// code review).
pub const LOCAL_ALLOWLIST: &[&str] = &[
    "cat", "tail", "head", "grep", "ls", "ps", "df", "free", "uptime", "whoami", "hostname",
    "systemctl",
];

/// `systemctl` subcommands that only read state. Everything else
/// (`restart`, `stop`, `enable`, ...) is rejected.
pub const SYSTEMCTL_READ_ONLY: &[&str] = &["status", "is-active", "show", "list-units"];

/// What a run produced. `stdout`/`stderr` are lossy-UTF-8 decodes of
/// the bytes captured (capped per stream at the caller's cap).
#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    pub stdout: String,
    pub stderr: String,
    /// Process exit code (`None` when killed by a signal).
    pub exit_code: Option<i32>,
    /// Unix signal number when killed by a signal (`None` otherwise).
    #[allow(dead_code)]
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

/// Split a command line into argv. No quoting in v1: any token
/// containing a quote char is rejected by the metachar screen (quotes
/// are in `METACHARS`), so `split_whitespace` is the whole parser.
pub fn parse_command(line: &str) -> Result<Vec<String>, String> {
    let argv: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    if argv.is_empty() {
        return Err("empty command".to_string());
    }
    for tok in &argv {
        if tok.chars().any(|c| METACHARS.contains(&c) || c.is_control()) {
            return Err(format!("rejected: token {tok:?} contains a forbidden character"));
        }
    }
    Ok(argv)
}

/// Local allowlist check, including the `systemctl` subcommand
/// restriction. `systemctl` with no subcommand is rejected (there is
/// no read-only bare form worth allowing).
pub fn check_local_command(argv: &[String]) -> Result<(), String> {
    let Some(first) = argv.first() else {
        return Err("empty command".to_string());
    };
    if !LOCAL_ALLOWLIST.contains(&first.as_str()) {
        return Err(format!("rejected: `{first}` is not in the read-only allowlist"));
    }
    if first == "systemctl" {
        match argv.get(1).map(String::as_str) {
            Some(sub) if SYSTEMCTL_READ_ONLY.contains(&sub) => {}
            Some(sub) => {
                return Err(format!("rejected: `systemctl {sub}` is not a read-only subcommand"));
            }
            None => return Err("rejected: `systemctl` needs a read-only subcommand".to_string()),
        }
    }
    Ok(())
}

/// Drain a pipe to end, keeping at most `cap` bytes. Reading never
/// stops early: a chatty child must never block on a full pipe, so
/// bytes past the cap are read and discarded (and `truncated` is set).
fn drain_capped<R: Read>(mut reader: R, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if kept.len() < cap {
                    let room = cap - kept.len();
                    let take = n.min(room);
                    kept.extend_from_slice(&buf[..take]);
                    if n > take {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (kept, truncated)
}

#[cfg(unix)]
fn kill_tree(pid: u32) {
    // The child runs in its own process group (see pre_exec below),
    // so SIGKILL to the group reaps backgrounded grandchildren too.
    unsafe {
        libc::killpg(pid as i32, libc::SIGKILL);
    }
}

/// Run argv directly (NO shell) with a wall-clock timeout and a
/// per-stream output cap. stdin is /dev/null (no TTY — interactive
/// prompts fail fast). On timeout the process (group) is killed and
/// `timed_out` is set; partial output captured so far is returned.
pub fn run_command_with_timeout(
    argv: &[String],
    timeout: Duration,
    cap_per_stream: usize,
) -> std::io::Result<ExecutionOutcome> {
    let Some((program, args)) = argv.split_first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty argv",
        ));
    };
    let started = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            // New process group: on timeout we SIGKILL the group, not
            // just the child, so backgrounded grandchildren die too.
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;
    let pid = child.id();
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle =
        std::thread::spawn(move || out_pipe.as_mut().map(|p| drain_capped(p, cap_per_stream)).unwrap_or_default());
    let err_handle =
        std::thread::spawn(move || err_pipe.as_mut().map(|p| drain_capped(p, cap_per_stream)).unwrap_or_default());

    let mut timed_out = false;
    let status;
    loop {
        match child.try_wait()? {
            Some(s) => {
                status = Some(s);
                break;
            }
            None => {
                if started.elapsed() >= timeout {
                    timed_out = true;
                    #[cfg(unix)]
                    kill_tree(pid);
                    #[cfg(not(unix))]
                    let _ = child.kill();
                    // Reap: closes the pipes so the drain threads see EOF.
                    status = child.wait().ok();
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }

    let (stdout_bytes, stdout_truncated) = out_handle.join().unwrap_or_default();
    let (stderr_bytes, stderr_truncated) = err_handle.join().unwrap_or_default();
    Ok(ExecutionOutcome {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code: status.and_then(|s| s.code()),
        #[cfg(unix)]
        signal: {
            use std::os::unix::process::ExitStatusExt;
            status.and_then(|s| s.signal())
        },
        #[cfg(not(unix))]
        signal: None,
        timed_out,
        duration_ms: started.elapsed().as_millis(),
        stdout_truncated,
        stderr_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn parse_splits_on_whitespace() {
        assert_eq!(parse_command("ps aux").unwrap(), argv(&["ps", "aux"]));
        assert_eq!(parse_command("  ls   -la  ").unwrap(), argv(&["ls", "-la"]));
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(parse_command("").is_err());
        assert!(parse_command("   ").is_err());
    }

    #[test]
    fn parse_rejects_every_metachar_class() {
        // Pipe, redirect, chain, substitution, glob, quote, tilde, group.
        for cmd in [
            "ps aux | grep x",
            "ls > /tmp/x",
            "ls 2> /tmp/x",
            "ps; ls",
            "ps && ls",
            "echo $(whoami)",
            "echo `whoami`",
            "ls *.rs",
            "ls ~/x",
            "echo \"hi\"",
            "echo 'hi'",
            "ps (aux)",
            "echo {a,b}",
            "echo a?b",
        ] {
            assert!(parse_command(cmd).is_err(), "must reject: {cmd}");
        }
    }

    #[test]
    fn parse_rejects_control_chars() {
        // Whitespace controls (\n, \t) are separators by construction
        // (split_whitespace) — the screen's job is NON-whitespace
        // controls smuggled inside a token (escape, NUL, ...).
        assert!(parse_command("ls\x1b/tmp").is_err());
        assert!(parse_command("ls\x00x").is_err());
        assert!(parse_command("ls\x07x").is_err());
    }

    #[test]
    fn local_allowlist_accepts_each_command() {
        for cmd in [
            "cat", "tail", "head", "grep", "ls", "ps", "df", "free", "uptime", "whoami",
            "hostname",
        ] {
            assert!(check_local_command(&argv(&[cmd])).is_ok(), "{cmd} must be allowed");
        }
        assert!(check_local_command(&argv(&["systemctl", "status"])).is_ok());
        assert!(check_local_command(&argv(&["systemctl", "is-active", "helix"])).is_ok());
        assert!(check_local_command(&argv(&["systemctl", "show", "helix"])).is_ok());
        assert!(check_local_command(&argv(&["systemctl", "list-units"])).is_ok());
    }

    #[test]
    fn local_allowlist_rejects_writes_and_bad_systemctl() {
        for words in [
            vec!["rm", "-rf", "/"],
            vec!["cp", "a", "b"],
            vec!["mv", "a", "b"],
            vec!["touch", "x"],
            vec!["systemctl", "restart", "helix"],
            vec!["systemctl", "stop", "helix"],
            vec!["systemctl", "enable", "helix"],
            vec!["systemctl"],
            vec!["chmod", "+x", "x"],
            vec!["curl", "http://example.com"],
        ] {
            assert!(check_local_command(&argv(&words)).is_err(), "{words:?} must be rejected");
        }
    }

    #[test]
    fn run_executes_allowlisted_command() {
        let out = run_command_with_timeout(&argv(&["hostname"]), Duration::from_secs(10), 4096)
            .expect("hostname must run");
        assert!(!out.timed_out);
        assert_eq!(out.exit_code, Some(0));
        assert!(!out.stdout.trim().is_empty(), "hostname prints something");
    }

    #[test]
    fn run_reports_nonzero_exit() {
        // `grep` with no match exits 1 — a real non-zero path, not a stub.
        let out = run_command_with_timeout(
            &argv(&["grep", "definitely-not-present-zzz", "/etc/hostname"]),
            Duration::from_secs(10),
            4096,
        )
        .expect("grep must run");
        assert!(!out.timed_out);
        assert_eq!(out.exit_code, Some(1));
    }

    #[test]
    fn run_times_out_without_hanging() {
        let start = Instant::now();
        let out = run_command_with_timeout(
            &argv(&["sleep", "30"]),
            Duration::from_secs(1),
            4096,
        )
        .expect("sleep must spawn");
        assert!(out.timed_out, "30s sleep must time out at 1s");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timeout must bound the run, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn run_caps_output_and_marks_truncation() {
        // `seq` is not allowlisted, but the runner itself takes argv
        // directly — the cap behavior is independent of the allowlist.
        let out = run_command_with_timeout(
            &argv(&["seq", "1", "100000"]),
            Duration::from_secs(15),
            1024,
        )
        .expect("seq must run");
        assert!(out.stdout_truncated, "100k lines must exceed a 1K cap");
        assert!(out.stdout.len() <= 1024);
    }
}
