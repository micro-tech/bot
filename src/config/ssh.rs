//! `[ssh]` configuration — the SSH server into Helix.
//!
//! ```toml
//! [ssh]
//! enabled = false                    # master switch; default off
//! bind = "127.0.0.1"                 # loopback by default; "0.0.0.0" needs explicit opt-in + warns loudly
//! port = 2222                        # avoids colliding with system sshd on 22
//! authorized_keys = "/etc/helix/ssh/authorized_keys"  # dedicated Helix-managed file
//! host_key = "/etc/helix/ssh/ssh_host_key"            # server host key (generated if missing)
//! max_sessions = 8                   # concurrent session cap
//! idle_timeout_secs = 900            # reap sessions idle this long
//! auth_rate_limit_per_minute = 10    # per-IP failed-auth throttle
//! ```
//!
//! THREAT MODEL (see also docs/ssh.md):
//! - Asset: shell on the Helix service user's machine.
//! - Adversaries: LAN attacker, compromised Tailscale peer pivoting,
//!   prompt-injection / malicious web content trying to widen the bind or
//!   add keys.
//! - Controls: public-key auth ONLY (no passwords, no keyboard-interactive,
//!   ever); keys from the dedicated Helix-managed file (never
//!   `~/.ssh/authorized_keys`); loopback bind by default; fail closed with
//!   no keys; bind comes ONLY from the config file (never settable from
//!   chat/tool calls — prompt-injection guard); sessions execute through the
//!   `run_shell` policy (task 191) — one shell path, not two.

use serde::Deserialize;

/// Configuration for the Helix SSH server.
#[derive(Debug, Clone, Deserialize)]
pub struct SshConfig {
    /// Master switch. Default false — the listener never starts unasked.
    #[serde(default)]
    pub enabled: bool,

    /// Address to bind. Defaults to loopback-only. `"0.0.0.0"` is accepted
    /// only as an explicit config choice and triggers a loud startup
    /// warning. The only documented non-loopback option is the Tailscale
    /// interface IP.
    #[serde(default = "default_bind")]
    pub bind: String,

    /// Port to listen on. Default 2222 avoids the system sshd on 22.
    #[serde(default = "default_port")]
    pub port: u16,

    /// Dedicated Helix-managed authorized_keys file. Relative paths resolve
    /// against the directory containing the config file Helix loaded.
    /// Default: `<config-dir>/ssh/authorized_keys`.
    #[serde(default)]
    pub authorized_keys: Option<String>,

    /// Server host key (OpenSSH private key format). Generated on first
    /// start if missing. Default: `<config-dir>/ssh/ssh_host_key`.
    #[serde(default)]
    pub host_key: Option<String>,

    /// Max concurrent SSH sessions; further channels are rejected.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,

    /// Sessions idle this long are reaped (russh inactivity timeout).
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,

    /// Failed auth attempts allowed per IP per minute before throttling.
    #[serde(default = "default_auth_rate_limit")]
    pub auth_rate_limit_per_minute: u32,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    2222
}

fn default_max_sessions() -> usize {
    8
}

fn default_idle_timeout_secs() -> u64 {
    900
}

fn default_auth_rate_limit() -> u32 {
    10
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_bind(),
            port: default_port(),
            authorized_keys: None,
            host_key: None,
            max_sessions: default_max_sessions(),
            idle_timeout_secs: default_idle_timeout_secs(),
            auth_rate_limit_per_minute: default_auth_rate_limit(),
        }
    }
}

impl SshConfig {
    /// Load the top-level `[ssh]` section from a full TOML string.
    /// Missing section (or unparseable input) yields the disabled default.
    pub fn load_from_toml(toml_str: &str) -> Self {
        toml::from_str::<toml::Value>(toml_str)
            .ok()
            .and_then(|v| v.get("ssh").cloned())
            .and_then(|s| s.try_into().ok())
            .unwrap_or_default()
    }

    /// True when the operator explicitly enabled the SSH server.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled_loopback() {
        let cfg = SshConfig::load_from_toml("[helix]\nname = \"x\"\n");
        assert!(!cfg.enabled);
        assert_eq!(cfg.bind, "127.0.0.1");
        assert_eq!(cfg.port, 2222);
        assert_eq!(cfg.max_sessions, 8);
    }

    #[test]
    fn parses_full_section() {
        let cfg = SshConfig::load_from_toml(
            "[ssh]\nenabled = true\nbind = \"0.0.0.0\"\nport = 2223\n\
             authorized_keys = \"/etc/helix/ssh/authorized_keys\"\nmax_sessions = 4\n",
        );
        assert!(cfg.enabled);
        assert_eq!(cfg.bind, "0.0.0.0");
        assert_eq!(cfg.port, 2223);
        assert_eq!(
            cfg.authorized_keys.as_deref(),
            Some("/etc/helix/ssh/authorized_keys")
        );
        assert_eq!(cfg.max_sessions, 4);
    }
}
