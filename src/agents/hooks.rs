//! Hook system for pre-step and post-step execution.
//!
//! Hooks allow external code to observe or modify state before/after each step.

use crate::agents::agent_state::AgentState;
use crate::agents::agent_step::AgentStep;

pub trait PreStepHook: Send + Sync {
    fn before_step(&self, state: &mut AgentState);
}

pub trait PostStepHook: Send + Sync {
    fn after_step(&self, state: &mut AgentState, step: &AgentStep);
}

/// Simple no-op hook implementations for testing.
pub struct NoopPreHook;
impl PreStepHook for NoopPreHook {
    fn before_step(&self, _state: &mut AgentState) {}
}

pub struct NoopPostHook;
impl PostStepHook for NoopPostHook {
    fn after_step(&self, _state: &mut AgentState, _step: &AgentStep) {}
}
