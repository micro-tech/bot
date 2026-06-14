// router/health.rs - Backend Health Checking (task 148)
use std::collections::HashMap;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub reachable: bool,
    pub latency_ms: u32,
    pub last_success: Option<DateTime<Utc>>,
    pub error_rate: f32,
    pub degraded: bool,
    pub capabilities: Vec<String>,
}

pub struct HealthStore {
    statuses: HashMap<String, HealthStatus>,
}

impl HealthStore {
    pub fn new() -> Self {
        Self { statuses: HashMap::new() }
    }

    pub fn get(&self, backend: &str) -> Option<HealthStatus> {
        self.statuses.get(backend).cloned()
    }

    // TODO: async probe loop for Ollama/Gemini/Grok
}
