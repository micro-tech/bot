//! Phase 2: ASUS SSH relay (tasks 304/305).
//!
//! `!asus: <command>` routes over Tailscale SSH to Windows OpenSSH
//! Server on the ASUS. The SAME sender gate (task 297) runs before
//! routing, and the SAME metachar rejection runs before ssh — the
//! remote shell only ever sees simple words, so remote-shell
//! invocation stays read-only. No shell runs on the R630 side either:
//! ssh itself is spawned with `std::process::Command` directly.

use crate::chat_command::executor::{self, ExecutionOutcome};
use std::time::Duration;

/// PowerShell allowlist, STARTER set (task 305 finalizes against the
/// CONFIRMED ASUS shell + PS version from tasks 302/303 — do not
/// assume beyond this: if the drill shows the default shell is
/// cmd.exe, replace this list with the cmd set and document it).
/// Matching is case-insensitive because PowerShell cmdlets are.
/// Single cmdlets with simple args only — no pipelines, redirection,
/// or substitution (already rejected by the metachar rule).
pub const ASUS_ALLOWLIST: &[&str] = &[
    "hostname",
    "whoami",
    "Get-Process",
    "Get-Service",
    "Get-ComputerInfo",
    "systeminfo",
];

/// ASUS allowlist check (case-insensitive — PowerShell is).
/// No `systemctl`-style subcommand rule: none of the allowlisted
/// cmdlets has a dangerous subcommand form in v1 usage.
pub fn check_asus_command(argv: &[String]) -> Result<(), String> {
    let Some(first) = argv.first() else {
        return Err("empty command".to_string());
    };
    if !ASUS_ALLOWLIST
        .iter()
        .any(|c| c.eq_ignore_ascii_case(first))
    {
        return Err(format!("rejected: `{first}` is not in the ASUS read-only allowlist"));
    }
    Ok(())
}

/// Build the ssh argv. BatchMode (fail fast, never hang on a password
/// prompt) + ConnectTimeout 15 s are mandatory, as is the command
/// timeout covering the whole call (enforced by the caller).
pub fn build_ssh_argv(
    ssh_bin: &str,
    key_path: &str,
    user: &str,
    host: &str,
    command_words: &[String],
) -> Vec<String> {
    let mut argv = vec![
        ssh_bin.to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
        "-i".to_string(),
        key_path.to_string(),
        format!("{user}@{host}"),
    ];
    argv.extend(command_words.iter().cloned());
    argv
}

/// Run a command on the ASUS via ssh. `ssh_bin` is injectable so
/// tests can point it at a stub script (production passes `"ssh"`).
pub fn run_asus_command(
    ssh_bin: &str,
    key_path: &str,
    user: &str,
    host: &str,
    command_words: &[String],
    timeout: Duration,
    cap_per_stream: usize,
) -> std::io::Result<ExecutionOutcome> {
    let argv = build_ssh_argv(ssh_bin, key_path, user, host, command_words);
    executor::run_command_with_timeout(&argv, timeout, cap_per_stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn asus_allowlist_is_case_insensitive() {
        // PowerShell cmdlets are case-insensitive; the check is too.
        for cmd in ["hostname", "whoami", "Get-Process", "get-process", "GET-SERVICE", "systeminfo"] {
            assert!(check_asus_command(&argv(&[cmd])).is_ok(), "{cmd} must be allowed");
        }
    }

    #[test]
    fn asus_allowlist_rejects_writes() {
        // (Pipeline tokens never reach this check: parse_command's
        // metachar screen rejects them first — covered by the
        // executor parse tests and the tick-level ASUS tests.)
        for words in [
            vec!["Remove-Item", "x"],
            vec!["Restart-Computer"],
            vec!["ssh", "other"],
        ] {
            assert!(check_asus_command(&argv(&words)).is_err(), "{words:?} must be rejected");
        }
    }

    #[test]
    fn ssh_argv_shape() {
        let argv = build_ssh_argv(
            "ssh",
            "/home/cobble/.ssh/id_helix_asus",
            "someone",
            "100.112.110.14",
            &argv(&["hostname"]),
        );
        assert_eq!(
            argv,
            vec![
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=15",
                "-i",
                "/home/cobble/.ssh/id_helix_asus",
                "someone@100.112.110.14",
                "hostname",
            ]
        );
    }
}
