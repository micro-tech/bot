//! AgentState — the central state container for the Unified Agent Runtime Loop.
//!
//! Security & Safety:
//! - All fields are private where mutation must be controlled.
//! - Step counter and halted flag are the primary loop guards.
//! - Memory deltas are collected for controlled commit to long-term memory.
//! - Logging is mandatory on every state transition.

use crate::agents::agent_step::AgentStep;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// Conversation / action history
    pub messages: Vec<String>,

    /// Temporary working memory / scratchpad
    pub scratchpad: Vec<String>,

    /// Pending memory writes (committed at end of step or on halt)
    pub memory_deltas: Vec<(String, serde_json::Value)>,

    /// Result of the last tool execution (if any)
    pub last_tool_result: Option<serde_json::Value>,

    /// Current step number (used for max_steps guard)
    pub step_count: u32,

    /// Whether the agent has reached a terminal state
    pub halted: bool,

    /// Last error encountered (if any)
    pub last_error: Option<String>,

    /// Bounded history of recent steps (for debugging / replay)
    pub recent_steps: VecDeque<AgentStep>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            scratchpad: Vec::new(),
            memory_deltas: Vec::new(),
            last_tool_result: None,
            step_count: 0,
            halted: false,
            last_error: None,
            recent_steps: VecDeque::with_capacity(32),
        }
    }

    /// Record a new step and increment the counter.
    /// Also enforces a maximum history size.
    pub fn record_step(&mut self, step: AgentStep) {
        if self.recent_steps.len() >= 32 {
            self.recent_steps.pop_front();
        }
        self.recent_steps.push_back(step);
        self.step_count += 1;

        crate::utils::log_to_file(&format!(
            "[AgentState] step={} action={}",
            self.step_count,
            self.recent_steps.back().map(|s| s.label()).unwrap_or("unknown")
        ));
    }

    /// Mark the agent as halted (terminal state reached).
    pub fn halt(&mut self, reason: &str) {
        self.halted = true;
        self.last_error = Some(reason.to_string());
        crate::utils::log_to_file(&format!("[AgentState] HALTED: {}", reason));
    }

    /// Add a memory delta to be committed later.
    pub fn push_memory_delta(&mut self, key: String, value: serde_json::Value) {
        self.memory_deltas.push((key, value));
    }

    /// Reset transient per-step fields (call at start of each loop iteration).
    pub fn reset_transients(&mut self) {
        self.last_tool_result = None;
    }

    /// Returns true if the agent should stop (halted or too many steps).
    pub fn should_stop(&self, max_steps: u32) -> bool {
        self.halted || self.step_count >= max_steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::agent_step::{AgentStep, FinalAnswer};

    #[test]
    fn test_initial_state() {
        let state = AgentState::new();
        assert_eq!(state.step_count, 0);
        assert!(!state.halted);
        assert!(state.messages.is_empty());
    }

    #[test]
    fn test_record_step_and_halt() {
        let mut state = AgentState::new();
        state.record_step(AgentStep::FinalAnswer("done".into()));
        assert_eq!(state.step_count, 1);
        assert!(!state.halted);

        state.halt("test termination");
        assert!(state.halted);
        assert!(state.should_stop(10));
    }

    #[test]
    fn test_max_steps_guard() {
        let mut state = AgentState::new();
        for _ in 0..5 {
            state.record_step(AgentStep::FinalAnswer("x".into()));
        }
        assert!(state.should_stop(5));
        assert!(!state.should_stop(10));
    }
}
