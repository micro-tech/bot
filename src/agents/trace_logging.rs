//! Runtime Trace Logging — structured JSON per step for debugging and replay.

use crate::agents::agent_state::AgentState;
use crate::agents::agent_step::AgentStep;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub struct RuntimeTrace {
    pub timestamp: u128,
    pub step: u32,
    pub action: String,
    pub memory_writes: usize,
    pub halted: bool,
}

pub fn log_trace(state: &AgentState, step: &AgentStep) {
    let trace = RuntimeTrace {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        step: state.step_count,
        action: step.label().to_string(),
        memory_writes: state.memory_deltas.len(),
        halted: state.halted,
    };

    let json = serde_json::to_string(&trace).unwrap_or_default();
    crate::utils::log_to_file(&format!("[TRACE] {}", json));
}
