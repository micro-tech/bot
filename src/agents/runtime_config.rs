//! Runtime configuration loaded from config.toml or defaults.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConfig {
    pub max_steps: u32,
    pub max_tool_retries: u32,
    pub trace_enabled: bool,
    pub timeout_seconds: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_steps: 50,
            max_tool_retries: 3,
            trace_enabled: true,
            timeout_seconds: 300,
        }
    }
}

impl RuntimeConfig {
    /// Load from a TOML string (or use defaults).
    pub fn from_toml(content: &str) -> Self {
        toml::from_str(content).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = RuntimeConfig::default();
        assert_eq!(cfg.max_steps, 50);
    }
}
