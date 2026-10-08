use serde::Deserialize;

/// Configuration for the Helix ACP (Agent Client Protocol) server.
///
/// The ACP server lets editors like Zed drive Helix natively over stdio
/// JSON-RPC. This is a remote-control surface and is **opt-in**: it does
/// nothing unless `enabled = true`.
///
/// ```toml
/// [acp]
/// enabled = true
/// max_steps = 25
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct AcpConfig {
    /// Master switch. Default false — the ACP server never starts unasked.
    #[serde(default)]
    pub enabled: bool,

    /// Max runtime-loop steps per `session/prompt` turn.
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
}

fn default_max_steps() -> u32 {
    25
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_steps: default_max_steps(),
        }
    }
}

impl AcpConfig {
    /// Load the top-level `[acp]` section from a full TOML string.
    /// Missing section (or unparseable input) yields the disabled default.
    pub fn load_from_toml(toml_str: &str) -> Self {
        toml::from_str::<toml::Value>(toml_str)
            .ok()
            .and_then(|v| v.get("acp").cloned())
            .and_then(|t| t.try_into().ok())
            .unwrap_or_default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_config_defaults_to_disabled() {
        // GIVEN no [acp] section:
        let cfg = AcpConfig::load_from_toml("[helix]\nname = \"x\"\n");
        // THEN the server stays off:
        assert!(!cfg.is_enabled());
        assert_eq!(cfg.max_steps, 25);
    }

    #[test]
    fn acp_config_parses_enabled() {
        // GIVEN an enabled section:
        let cfg = AcpConfig::load_from_toml("[acp]\nenabled = true\nmax_steps = 10\n");
        // THEN it parses:
        assert!(cfg.is_enabled());
        assert_eq!(cfg.max_steps, 10);
    }

    #[test]
    fn acp_config_garbage_is_disabled() {
        // GIVEN garbage input:
        let cfg = AcpConfig::load_from_toml("not toml [[[");
        // THEN we fail closed:
        assert!(!cfg.is_enabled());
    }
}
