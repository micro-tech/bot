use serde::Deserialize;
use std::collections::HashMap;

/// Configuration for the Helix Project Scanner (Task 171).
/// Supports scanning the Main PC from Dell 630 over the network (SSH preferred).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProjectScannerConfig {
    /// Master switch for the project scanner system.
    pub enabled: Option<bool>,

    /// Default max recursion depth for scans.
    pub default_max_depth: Option<usize>,

    /// Identifier for the machine performing the scan (e.g. "dell630").
    pub scanned_from: Option<String>,

    /// Default remote access protocol (ssh | smb | http | local).
    pub default_protocol: Option<String>,

    /// Optional per-machine connection settings (future expansion).
    pub machines: Option<HashMap<String, MachineConfig>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MachineConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub ssh_key_path: Option<String>,
    pub auth_method: Option<String>, // "key" | "password" (password not recommended)
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProjectConfig {
    pub name: String,
    pub target_machine: String,
    pub remote_path: String,
    pub protocol: Option<String>,
    pub include_globs: Option<Vec<String>>,
    pub exclude_globs: Option<Vec<String>>,
    pub max_depth: Option<usize>,
    pub description: Option<String>,
}

impl ProjectScannerConfig {
    /// Load the `[helix.project_scanner]` section from a full TOML string.
    pub fn load_from_toml(toml_str: &str) -> Self {
        toml::from_str::<toml::Value>(toml_str)
            .ok()
            .and_then(|v| v.get("helix")?.get("project_scanner").cloned())
            .and_then(|r| r.try_into().ok())
            .unwrap_or_default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    pub fn default_max_depth(&self) -> usize {
        self.default_max_depth.unwrap_or(5)
    }

    pub fn scanned_from(&self) -> String {
        self.scanned_from
            .clone()
            .unwrap_or_else(|| "dell630".to_string())
    }

    pub fn default_protocol(&self) -> String {
        self.default_protocol
            .clone()
            .unwrap_or_else(|| "ssh".to_string())
    }

    /// Parse the full [[helix.projects]] array.
    pub fn load_projects(toml_str: &str) -> Vec<ProjectConfig> {
        toml::from_str::<toml::Value>(toml_str)
            .ok()
            .and_then(|v| {
                v.get("helix")
                    .and_then(|h| h.get("projects"))
                    .and_then(|p| p.as_array())
                    .cloned()
            })
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| {
                let name = entry.get("name")?.as_str()?.to_string();
                let target_machine = entry.get("target_machine")?.as_str()?.to_string();
                let remote_path = entry.get("remote_path")?.as_str()?.to_string();

                Some(ProjectConfig {
                    name,
                    target_machine,
                    remote_path,
                    protocol: entry.get("protocol").and_then(|p| p.as_str()).map(|s| s.to_string()),
                    include_globs: entry.get("include_globs").and_then(|g| {
                        g.as_array().map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    }),
                    exclude_globs: entry.get("exclude_globs").and_then(|g| {
                        g.as_array().map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    }),
                    max_depth: entry.get("max_depth").and_then(|d| d.as_integer()).map(|i| i as usize),
                    description: entry.get("description").and_then(|d| d.as_str()).map(|s| s.to_string()),
                })
            })
            .collect()
    }
}

impl ProjectConfig {
    pub fn effective_protocol(&self, default: &str) -> String {
        self.protocol.clone().unwrap_or_else(|| default.to_string())
    }

    pub fn effective_max_depth(&self, default: usize) -> usize {
        self.max_depth.unwrap_or(default)
    }
}
