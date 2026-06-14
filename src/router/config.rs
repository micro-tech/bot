// router/config.rs - TOML/JSON config with hot-reload stub
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfigFile {
    pub complexity_threshold: f32,
    pub token_threshold: usize,
    pub time_windows: Vec<TimeWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start: String,
    pub end: String,
    pub profile: String,
}

impl RouterConfigFile {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        // TODO: real serde + notify watcher
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        toml::from_str(&content).map_err(|e| e.to_string())
    }
}
