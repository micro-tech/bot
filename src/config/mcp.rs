use serde::Deserialize;
use std::collections::HashMap;

/// Configuration for the Helix MCP (Model Context Protocol) client.
///
/// This section lets Helix spawn external MCP servers as subprocesses over
/// stdio and expose their tools to the agent. Note: this is *not* related to
/// `src/mcp/` ("Master Control Program", the internal bus router).
///
/// ```toml
/// [mcp]
/// enabled = true
///
/// [[mcp.servers]]
/// name = "google"
/// command = "/path/to/google-mcp"
/// args = []
///
/// [mcp.servers.env]
/// GOOGLE_CLIENT_ID = "..."
/// ```
#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpConfig {
    /// Master switch. When false (the default), the MCP client is a silent
    /// no-op: no servers are spawned and no tools are advertised.
    #[serde(default)]
    pub enabled: bool,

    /// MCP servers to spawn. Each becomes a tool namespace `mcp__<name>__*`.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// One external MCP server, spawned as a subprocess over stdio.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpServerConfig {
    /// Logical server name. Used in tool namespacing (`mcp__<name>__<tool>`).
    /// Must not contain `__`.
    #[serde(default)]
    pub name: String,

    /// Executable to spawn (absolute path or resolvable via PATH).
    #[serde(default)]
    pub command: String,

    /// Extra argv entries for the server process.
    #[serde(default)]
    pub args: Vec<String>,

    /// Extra environment variables for the server process
    /// (in addition to the inherited environment).
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl McpConfig {
    /// Load the top-level `[mcp]` section from a full TOML string.
    /// Missing section (or unparseable input) yields the disabled default.
    pub fn load_from_toml(toml_str: &str) -> Self {
        toml::from_str::<toml::Value>(toml_str)
            .ok()
            .and_then(|v| v.get("mcp").cloned())
            .and_then(|t| t.try_into().ok())
            .unwrap_or_default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Server entries with a usable name + command. Entries whose name
    /// contains the `__` namespace separator are dropped (they would make
    /// tool names ambiguous).
    pub fn usable_servers(&self) -> Vec<McpServerConfig> {
        self.servers
            .iter()
            .filter(|s| !s.name.trim().is_empty() && !s.command.trim().is_empty())
            .filter(|s| !s.name.contains("__"))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
[helix]
name = "Helix"

[mcp]
enabled = true

[[mcp.servers]]
name = "google"
command = "/opt/helix/bin/google-mcp"
args = ["--verbose"]

[mcp.servers.env]
GOOGLE_CLIENT_ID = "id-123"

[[mcp.servers]]
name = "bare"
command = "my-server"
"#;

    #[test]
    fn parses_enabled_and_servers() {
        let cfg = McpConfig::load_from_toml(FULL);
        assert!(cfg.is_enabled());
        assert_eq!(cfg.servers.len(), 2);

        let google = &cfg.servers[0];
        assert_eq!(google.name, "google");
        assert_eq!(google.command, "/opt/helix/bin/google-mcp");
        assert_eq!(google.args, vec!["--verbose".to_string()]);
        assert_eq!(
            google.env.get("GOOGLE_CLIENT_ID").map(String::as_str),
            Some("id-123")
        );

        // args/env default to empty when omitted
        let bare = &cfg.servers[1];
        assert_eq!(bare.name, "bare");
        assert!(bare.args.is_empty());
        assert!(bare.env.is_empty());
    }

    #[test]
    fn missing_section_is_disabled_by_default() {
        let cfg = McpConfig::load_from_toml("[helix]\nname = \"Helix\"\n");
        assert!(!cfg.is_enabled());
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn garbage_input_is_disabled_by_default() {
        let cfg = McpConfig::load_from_toml("this is not toml ][[[");
        assert!(!cfg.is_enabled());
    }

    #[test]
    fn usable_servers_filters_bad_entries() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![
                McpServerConfig {
                    name: "ok".into(),
                    command: "/bin/ok".into(),
                    ..Default::default()
                },
                // empty command -> dropped
                McpServerConfig {
                    name: "nocmd".into(),
                    ..Default::default()
                },
                // name contains the namespace separator -> dropped
                McpServerConfig {
                    name: "bad__name".into(),
                    command: "/bin/x".into(),
                    ..Default::default()
                },
            ],
        };
        let usable = cfg.usable_servers();
        assert_eq!(usable.len(), 1);
        assert_eq!(usable[0].name, "ok");
    }
}
