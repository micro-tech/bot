//! Safety rules and loop guards for the runtime.
//!
//! Enforces hard limits to prevent runaway agents.

use crate::agents::agent_state::AgentState;

pub struct SafetyRules {
    pub max_steps: u32,
    pub max_tool_retries: u32,
    pub max_consecutive_errors: u32,
}

impl Default for SafetyRules {
    fn default() -> Self {
        Self {
            max_steps: 50,
            max_tool_retries: 3,
            max_consecutive_errors: 5,
        }
    }
}

impl SafetyRules {
    pub fn check(&self, state: &AgentState) -> Result<(), String> {
        if state.step_count >= self.max_steps {
            return Err("Maximum step count exceeded".into());
        }
        if let Some(err) = &state.last_error {
            // Simple consecutive error check (can be improved)
            if err.contains("error") && state.step_count > self.max_consecutive_errors {
                return Err("Too many consecutive errors".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_steps() {
        let rules = SafetyRules { max_steps: 2, ..Default::default() };
        let mut state = AgentState::new();
        state.step_count = 3;
        assert!(rules.check(&state).is_err());
    }
}
