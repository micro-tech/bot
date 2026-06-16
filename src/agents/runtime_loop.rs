//! Core runtime loop skeleton for the Unified Agent Runtime.
//! This is the "brainstem" that orchestrates everything.

use crate::agents::agent_state::AgentState;
use crate::agents::agent_step::AgentStep;
use crate::agents::planner::Planner;
use crate::agents::rule_layer::RuleLayer;
use log::info;

pub struct RuntimeLoop<P: Planner> {
    planner: P,
    rule_layer: RuleLayer,
    max_steps: u32,
}

impl<P: Planner> RuntimeLoop<P> {
    pub fn new(planner: P, rule_layer: RuleLayer, max_steps: u32) -> Self {
        Self { planner, rule_layer, max_steps }
    }

    /// Run one full agent execution cycle.
    pub async fn run(&self, mut state: AgentState) -> AgentState {
        while !state.should_stop(self.max_steps) {
            // Pre-step hook placeholder
            state.reset_transients();

            let step = self.planner.decide(&state).await;

            if let Err(e) = self.rule_layer.validate(&step, 0) {
                state.halt(&format!("Rule violation: {}", e));
                break;
            }

            match &step {
                AgentStep::FinalAnswer(_) | AgentStep::Error(_) => {
                    state.halt("Terminal step reached");
                }
                _ => {}
            }

            state.record_step(step.clone());

            // Post-step hook placeholder
            info!("Runtime step {} completed", state.step_count);
        }
        state
    }
}
