// router/override.rs - User Override & Session Preference (task 149)
use crate::router::LLMBackend;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum OverrideTier {
    OneShot,
    Session,
    Persistent,
}

#[derive(Debug, Clone)]
pub struct UserOverride {
    pub tier: OverrideTier,
    pub backend: LLMBackend,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct OverrideStore {
    pub overrides: HashMap<String, UserOverride>, // user_id -> override
}

impl OverrideStore {
    pub fn new() -> Self {
        Self { overrides: HashMap::new() }
    }

    pub fn resolve(&self, user_id: &str) -> Option<LLMBackend> {
        self.overrides.get(user_id).map(|o| o.backend.clone())
    }
}
