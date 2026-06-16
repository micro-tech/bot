//! Bus event emission for observability.

use crate::agents::agent_state::AgentState;
use crate::agents::agent_step::AgentStep;
use log::info;

pub fn emit_step_event(state: &AgentState, step: &AgentStep) {
    info!(
        "BUS_EVENT step={} action={} halted={}",
        state.step_count,
        step.label(),
        state.halted
    );
    // Real implementation would publish to the Bus
}

pub fn emit_terminal_event(state: &AgentState, reason: &str) {
    info!("BUS_EVENT terminal reason='{}' steps={}", reason, state.step_count);
}
