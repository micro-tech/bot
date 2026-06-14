// router/override.rs - Full User Override & Session Preference System (Task 133)
use crate::router::LLMBackend;
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideTier {
    OneShot,     // Highest priority - cleared after use
    Session,     // Persists for the conversation
    Persistent,  // Stored in user profile / survives restarts
}

#[derive(Debug, Clone)]
pub struct UserOverride {
    pub tier: OverrideTier,
    pub backend: LLMBackend,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OverrideCommand {
    pub user_id: String,
    pub backend: Option<LLMBackend>,
    pub tier: OverrideTier,
    pub ttl_seconds: Option<i64>,
}

pub struct OverrideStore {
    overrides: HashMap<String, UserOverride>,
    one_shot_used: HashMap<String, bool>,
}

impl OverrideStore {
    pub fn new() -> Self {
        Self {
            overrides: HashMap::new(),
            one_shot_used: HashMap::new(),
        }
    }

    /// Resolve the effective override for a user (highest tier wins)
    pub fn resolve(&self, user_id: &str) -> Option<LLMBackend> {
        // Check one-shot first (highest priority)
        if let Some(used) = self.one_shot_used.get(user_id) {
            if *used {
                return None; // already consumed
            }
        }

        if let Some(ov) = self.overrides.get(user_id) {
            // Check expiration
            if let Some(exp) = ov.expires_at {
                if Utc::now() > exp {
                    return None;
                }
            }
            return Some(ov.backend.clone());
        }
        None
    }

    /// Apply a new override command
    pub fn apply(&mut self, cmd: OverrideCommand) -> Result<(), String> {
        if let Some(backend) = cmd.backend {
            let expires = cmd.ttl_seconds.map(|s| Utc::now() + Duration::seconds(s));

            let ov = UserOverride {
                tier: cmd.tier.clone(),
                backend,
                expires_at: expires,
                created_at: Utc::now(),
            };

            self.overrides.insert(cmd.user_id.clone(), ov);

            if cmd.tier == OverrideTier::OneShot {
                self.one_shot_used.insert(cmd.user_id, false);
            }

            Ok(())
        } else {
            // Clear override
            self.overrides.remove(&cmd.user_id);
            self.one_shot_used.remove(&cmd.user_id);
            Ok(())
        }
    }

    /// Mark one-shot override as consumed after use
    pub fn consume_one_shot(&mut self, user_id: &str) {
        self.one_shot_used.insert(user_id.to_string(), true);
    }

    /// Remove expired overrides
    pub fn cleanup_expired(&mut self) {
        let now = Utc::now();
        self.overrides.retain(|_, ov| {
            ov.expires_at.map_or(true, |exp| now <= exp)
        });
    }
}
