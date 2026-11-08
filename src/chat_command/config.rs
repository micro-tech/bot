//! `[chat_command]` config section (tasks 295/304).
//!
//! Read from `config.toml` on EVERY tick (mirroring support_watch's
//! `live_config`), so edits apply without a restart. Every default is
//! fail-closed: an omitted section executes nothing.

use serde::Deserialize;

/// Parsed `[chat_command]` section.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCommandConfig {
    /// Hub room the loop watches. Rooms auto-create on first post.
    #[serde(default = "default_room")]
    pub room: String,

    /// Token names allowed to issue commands (hub stamps `sender` from
    /// the token name). Default EMPTY — nothing executes until John
    /// configures this. Expected values: `"cobble-jr"`, `"john"`.
    #[serde(default)]
    pub allowed_senders: Vec<String>,

    /// This agent's own hub name, for the `to` addressing field
    /// (spec §3). Default `"helix"` (the hub token name). A
    /// structured message whose `to` does not match this name is
    /// ignored entirely — no run, no reply, no error.
    #[serde(default = "default_agent_name")]
    pub agent_name: String,

    /// Per-command wall-clock timeout, seconds.
    #[serde(default = "default_timeout")]
    pub command_timeout_secs: u64,

    /// Cap on captured command output, bytes.
    #[serde(default = "default_max_output")]
    pub max_output_bytes: usize,

    /// Max bytes per chat reply chunk (hub cap is 8 KB; prefix included).
    #[serde(default = "default_chunk")]
    pub chunk_bytes: usize,

    /// Max commands executed per 1-minute tick; extras wait.
    #[serde(default = "default_max_cmds")]
    pub max_commands_per_tick: usize,

    /// Phase 2 master switch. Default false — John enables after
    /// setup/verify (tasks 302/303).
    #[serde(default)]
    pub asus_enabled: bool,

    /// ASUS SSH target. REQUIRED for Phase 2 (no defaults — a missing
    /// value means the relay stays off, never a guessed host).
    #[serde(default)]
    pub asus_host: Option<String>,
    #[serde(default)]
    pub asus_user: Option<String>,
    #[serde(default)]
    pub asus_key_path: Option<String>,
}

fn default_room() -> String {
    "command".to_string()
}
fn default_agent_name() -> String {
    "helix".to_string()
}
fn default_timeout() -> u64 {
    30
}
fn default_max_output() -> usize {
    131072
}
fn default_chunk() -> usize {
    7500
}
fn default_max_cmds() -> usize {
    20
}

impl Default for ChatCommandConfig {
    fn default() -> Self {
        Self {
            room: default_room(),
            allowed_senders: Vec::new(),
            agent_name: default_agent_name(),
            command_timeout_secs: default_timeout(),
            max_output_bytes: default_max_output(),
            chunk_bytes: default_chunk(),
            max_commands_per_tick: default_max_cmds(),
            asus_enabled: false,
            asus_host: None,
            asus_user: None,
            asus_key_path: None,
        }
    }
}

impl ChatCommandConfig {
    /// Load the top-level `[chat_command]` section from a full TOML
    /// string. Missing section (or unparseable input) yields the
    /// fail-closed default. Shape errors also fall back to defaults
    /// (via `config::load_section`, which never logs values — a wrong
    /// type could be a secret).
    pub fn load_from_toml(toml_str: &str) -> Self {
        crate::config::load_section(toml_str, "[chat_command]", |v| v.get("chat_command").cloned())
    }

    /// Read the live config from `<runtime_dir>/config.toml`.
    /// Missing/unreadable file → defaults (nothing executes).
    pub fn live_config_at(runtime_dir: &std::path::Path) -> Self {
        std::fs::read_to_string(runtime_dir.join("config.toml"))
            .map(|s| Self::load_from_toml(&s))
            .unwrap_or_default()
    }

    /// Phase 2 is usable only when enabled AND every required field is
    /// set. Anything less fails closed (no ssh attempted).
    pub fn asus_configured(&self) -> bool {
        self.asus_enabled
            && self.asus_host.as_ref().map(|s| !s.is_empty()).unwrap_or(false)
            && self.asus_user.as_ref().map(|s| !s.is_empty()).unwrap_or(false)
            && self
                .asus_key_path
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_section_yields_fail_closed_defaults() {
        let cfg = ChatCommandConfig::load_from_toml("[helix]\nname = \"x\"\n");
        assert_eq!(cfg.room, "command");
        assert!(cfg.allowed_senders.is_empty(), "empty senders = nothing executes");
        assert_eq!(cfg.agent_name, "helix", "own name defaults to the hub token name");
        assert_eq!(cfg.command_timeout_secs, 30);
        assert_eq!(cfg.max_output_bytes, 131072);
        assert_eq!(cfg.chunk_bytes, 7500);
        assert_eq!(cfg.max_commands_per_tick, 20);
        assert!(!cfg.asus_enabled);
        assert!(!cfg.asus_configured());
    }

    #[test]
    fn explicit_values_honored() {
        let cfg = ChatCommandConfig::load_from_toml(
            "[chat_command]\nroom = \"ops\"\nallowed_senders = [\"cobble-jr\", \"john\"]\nagent_name = \"helix\"\ncommand_timeout_secs = 10\nmax_output_bytes = 4096\nchunk_bytes = 1000\nmax_commands_per_tick = 3\nasus_enabled = true\nasus_host = \"100.112.110.14\"\nasus_user = \"someone\"\nasus_key_path = \"/home/cobble/.ssh/id_helix_asus\"\n",
        );
        assert_eq!(cfg.room, "ops");
        assert_eq!(cfg.allowed_senders, vec!["cobble-jr", "john"]);
        assert_eq!(cfg.agent_name, "helix");
        assert_eq!(cfg.command_timeout_secs, 10);
        assert_eq!(cfg.max_output_bytes, 4096);
        assert_eq!(cfg.chunk_bytes, 1000);
        assert_eq!(cfg.max_commands_per_tick, 3);
        assert!(cfg.asus_configured());
    }

    #[test]
    fn garbage_input_yields_defaults() {
        let cfg = ChatCommandConfig::load_from_toml("not toml ][[[");
        assert!(cfg.allowed_senders.is_empty());
    }

    #[test]
    fn partial_asus_config_stays_disabled() {
        let cfg = ChatCommandConfig::load_from_toml(
            "[chat_command]\nasus_enabled = true\nasus_host = \"100.112.110.14\"\n",
        );
        assert!(!cfg.asus_configured(), "missing user/key must fail closed");
    }
}
