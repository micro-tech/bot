use serde::Deserialize;

/// Configuration for the Helix OKF (Open Knowledge Format) Librarian System.
///
/// This section controls remote OKF bundle management, manifest loading,
/// registry building, and change detection.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OkfConfig {
    /// Master switch for the entire OKF system.
    pub enabled: Option<bool>,

    /// Base URL of the remote OKF server (e.g. "http://localhost:8080" or "https://okf.example.com")
    pub server_url: Option<String>,

    /// Automatically reload bundles when the remote server reports changes.
    pub auto_reload: Option<bool>,

    /// Path to write the generated index/registry file (relative to project root).
    pub index_file: Option<String>,

    /// Polling interval in seconds when using polling-based change detection.
    pub poll_interval_secs: Option<u64>,

    /// Optional authentication token for the OKF server.
    pub auth_token: Option<String>,

    /// Timeout for HTTP requests to the OKF server (in seconds).
    pub request_timeout_secs: Option<u64>,
}

impl OkfConfig {
    /// Load the `[helix.okf]` section from a full TOML string.
    pub fn load_from_toml(toml_str: &str) -> Self {
        toml::from_str::<toml::Value>(toml_str)
            .ok()
            .and_then(|v| v.get("helix")?.get("okf").cloned())
            .and_then(|r| r.try_into().ok())
            .unwrap_or_default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    pub fn server_url(&self) -> String {
        self.server_url
            .clone()
            .unwrap_or_else(|| "http://localhost:8080".to_string())
    }

    pub fn auto_reload(&self) -> bool {
        self.auto_reload.unwrap_or(true)
    }

    pub fn index_file(&self) -> String {
        self.index_file
            .clone()
            .unwrap_or_else(|| "okf_index.json".to_string())
    }

    pub fn poll_interval_secs(&self) -> u64 {
        self.poll_interval_secs.unwrap_or(60)
    }

    pub fn request_timeout_secs(&self) -> u64 {
        self.request_timeout_secs.unwrap_or(30)
    }
}
