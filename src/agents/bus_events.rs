//! Bus event types for the Unified Agent Runtime.

use crate::agents::agent_state::AgentState;
use log::info;
use serde::{Deserialize, Serialize};

/// Event emitted when an agent run completes successfully.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFinished {
    pub goal: String,
    pub result: String,
    pub steps_taken: u32,
    pub correlation_id: u64,
}

/// Event emitted when an agent run fails or is halted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentError {
    pub goal: String,
    pub error: String,
    pub steps_taken: u32,
    pub correlation_id: u64,
}

/// Emit AgentFinished event (placeholder — real impl would publish to bus).
pub fn emit_agent_finished(state: &AgentState, result: &str) {
    info!(
        "[BUS] AgentFinished: steps={}, result={}",
        state.step_count, result
    );
    // TODO: publish to actual bus when available
}

/// Emit AgentError event (placeholder — real impl would publish to bus).
pub fn emit_agent_error(state: &AgentState, error: &str) {
    info!(
        "[BUS] AgentError: steps={}, error={}",
        state.step_count, error
    );
    // TODO: publish to actual bus when available
}
