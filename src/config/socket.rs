use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct SocketConfig {
    pub path: String,
    pub group: String,
    pub mode: u32,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            path: "/var/run/helix.sock".to_string(),
            group: "helix".to_string(),
            mode: 0o660,
        }
    }
}
