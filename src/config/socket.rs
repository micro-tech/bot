use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct SocketConfig {
    pub path: String,
    pub group: String,
    pub mode: u32,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            path: "/var/run/bot.sock".to_string(),
            group: "botctl".to_string(),
            mode: 0o660,
        }
    }
}
