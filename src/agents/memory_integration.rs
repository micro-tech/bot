//! Memory integration for the runtime loop.
//! Commits memory deltas and triggers memory hooks.

use crate::agents::agent_state::AgentState;
use log::info;

pub fn commit_memory_deltas(state: &mut AgentState) {
    if state.memory_deltas.is_empty() {
        return;
    }

    for (key, value) in state.memory_deltas.drain(..) {
        info!("Committing memory delta: {} = {:?}", key, value);
        // In real implementation: write to MemoryManager
    }
}

pub fn trigger_memory_hooks(state: &AgentState) {
    if !state.memory_deltas.is_empty() {
        info!("Memory hooks triggered: {} deltas pending", state.memory_deltas.len());
    }
}
