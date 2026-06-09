use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReasoningConfig {
    pub enabled: Option<bool>,
    pub default_goal: Option<String>,
    pub cycle_interval_ticks: Option<u64>,
    pub metrics_interval_ticks: Option<u64>,
    pub health_check_interval_ticks: Option<u64>,
}

impl ReasoningConfig {
    pub fn load_from_toml(toml_str: &str) -> Self {
        toml::from_str::<toml::Value>(toml_str)
            .ok()
            .and_then(|v| v.get("reasoning").cloned())
            .and_then(|r| r.try_into().ok())
            .unwrap_or_default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn default_goal(&self) -> String {
        self.default_goal.clone().unwrap_or_else(|| {
            "Improve agent reliability, safety, and self-correction".to_string()
        })
    }

    pub fn cycle_interval(&self) -> u64 {
        self.cycle_interval_ticks.unwrap_or(20)
    }

    pub fn metrics_interval(&self) -> u64 {
        self.metrics_interval_ticks.unwrap_or(40)
    }

    pub fn health_check_interval(&self) -> u64 {
        self.health_check_interval_ticks.unwrap_or(100)
    }
}
