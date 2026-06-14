// task 127 core wiring - RoutingContext + LLMBackend enum
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum LLMBackend {
    LocalOllama,
    LanOllama,
    Gemini,
    Grok,
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingContext {
    pub prompt: String,
    pub token_estimate: usize,
    pub has_code: bool,
    pub complexity_score: f32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub user_override: Option<LLMBackend>,
    pub telemetry: Option<serde_json::Value>,
    pub health: Option<serde_json::Value>,
}

impl Default for RoutingContext {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            token_estimate: 0,
            has_code: false,
            complexity_score: 0.0,
            timestamp: chrono::Utc::now(),
            user_override: None,
            telemetry: None,
            health: None,
        }
    }
}
