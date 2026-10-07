//! `[shell]` configuration — the gate and policy for the `run_shell` tool.
//!
//! ```toml
//! [shell]
//! enabled = true              # default false: the tool refuses to run unless opted in
//! default_timeout_secs = 60   # per-command timeout when the caller doesn't set one
//! workdir = "."               # base directory commands run in (relative to Helix's CWD)
//! allow_absolute_paths = false # when false, workdir args must stay under `workdir`
//! max_output_bytes = 32768    # per-stream (stdout/stderr) capture cap
//! ```
//!
//! Task 192 (SSH server) executes sessions through the `run_shell` policy,
//! so it fails closed with a clear error when `[shell] enabled = false`.

use serde::Deserialize;

/// Configuration for the local shell tool.
#[derive(Debug, Clone, Deserialize)]
pub struct ShellConfig {
    /// Master switch. Default false — `run_shell` refuses to execute
    /// anything unless the operator explicitly opts in.
    #[serde(default)]
    pub enabled: bool,

    /// Timeout (seconds) applied when the tool call doesn't specify one.
    #[serde(default = "default_timeout_secs")]
    pub default_timeout_secs: u64,

    /// Base working directory for shell commands. Relative paths resolve
    /// against Helix's working directory at call time.
    #[serde(default = "default_workdir")]
    pub workdir: String,

    /// When false (default), a `workdir` argument that escapes the base
    /// directory (absolute path or `..` traversal) is rejected.
    #[serde(default)]
    pub allow_absolute_paths: bool,

    /// Per-stream capture cap for stdout/stderr, in bytes.
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
}

fn default_timeout_secs() -> u64 {
    60
}

fn default_workdir() -> String {
    ".".to_string()
}

fn default_max_output_bytes() -> usize {
    32 * 1024
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_timeout_secs: default_timeout_secs(),
            workdir: default_workdir(),
            allow_absolute_paths: false,
            max_output_bytes: default_max_output_bytes(),
        }
    }
}

impl ShellConfig {
    /// Load the top-level `[shell]` section from a full TOML string.
    /// Missing section (or unparseable input) yields the disabled default.
    pub fn load_from_toml(toml_str: &str) -> Self {
        toml::from_str::<toml::Value>(toml_str)
            .ok()
            .and_then(|v| v.get("shell").cloned())
            .and_then(|s| s.try_into().ok())
            .unwrap_or_default()
    }

    /// True when the operator explicitly enabled the shell tool.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled() {
        let cfg = ShellConfig::load_from_toml("[helix]\nname = \"x\"\n");
        assert!(!cfg.enabled);
        assert_eq!(cfg.default_timeout_secs, 60);
        assert_eq!(cfg.max_output_bytes, 32 * 1024);
    }

    #[test]
    fn parses_full_section() {
        let cfg = ShellConfig::load_from_toml(
            "[shell]\nenabled = true\ndefault_timeout_secs = 10\nworkdir = \"/tmp\"\nallow_absolute_paths = true\nmax_output_bytes = 1024\n",
        );
        assert!(cfg.enabled);
        assert_eq!(cfg.default_timeout_secs, 10);
        assert_eq!(cfg.workdir, "/tmp");
        assert!(cfg.allow_absolute_paths);
        assert_eq!(cfg.max_output_bytes, 1024);
    }
}
