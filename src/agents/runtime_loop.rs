//! Core runtime loop skeleton for the Unified Agent Runtime.
//! This is the "brainstem" that orchestrates everything.

use crate::agents::agent_state::AgentState;
use crate::agents::agent_step::AgentStep;
use crate::agents::planner::Planner;
use crate::agents::rule_layer::RuleLayer;
use log::{debug, info, warn};

pub struct RuntimeLoop<P: Planner> {
    planner: P,
    rule_layer: RuleLayer,
    max_steps: u32,
    trace: bool,
}

impl<P: Planner> RuntimeLoop<P> {
    pub fn new(planner: P, rule_layer: RuleLayer, max_steps: u32) -> Self {
        Self { planner, rule_layer, max_steps, trace: false }
    }

    /// Enable or disable detailed per-step trace logging.
    pub fn with_trace(mut self, enabled: bool) -> Self {
        self.trace = enabled;
        self
    }

    /// Run one full agent execution cycle.
    pub async fn run(&self, mut state: AgentState) -> AgentState {
        info!("[Runtime] Starting agent run (max_steps={})", self.max_steps);

        while !state.should_stop(self.max_steps) {
            state.reset_transients();

            if self.trace {
                debug!("[Runtime] Step {} — asking planner for next action", state.step_count);
            }

            let step = self.planner.decide(&state).await;

            if self.trace {
                match &step {
                    AgentStep::ToolCall(inv) => {
                        info!("[Runtime] Planner chose ToolCall: {} (args={})",
                              inv.name, serde_json::to_string(&inv.args).unwrap_or_default());
                    }
                    AgentStep::LLMCall(spec) => {
                        info!("[Runtime] Planner chose LLMCall (prompt_len={})", spec.prompt.len());
                    }
                    AgentStep::FinalAnswer(answer) => {
                        info!("[Runtime] Planner chose FinalAnswer: {}", answer);
                    }
                    AgentStep::Error(err) => {
                        warn!("[Runtime] Planner returned Error: {}", err);
                    }
                }
            }

            if let Err(e) = self.rule_layer.validate(&step, state.step_count) {
                warn!("[Runtime] Rule violation at step {}: {}", state.step_count, e);
                state.halt(&format!("Rule violation: {}", e));
                break;
            }

            match &step {
                AgentStep::FinalAnswer(answer) => {
                    state.halt("Terminal step reached");
                    crate::agents::bus_events::emit_agent_finished(&state, answer);
                    info!("[Runtime] Agent finished successfully");
                }
                AgentStep::Error(err) => {
                    state.halt("Terminal step reached");
                    crate::agents::bus_events::emit_agent_error(&state, err);
                    warn!("[Runtime] Agent halted with error: {}", err);
                }
                AgentStep::ToolCall(inv) => {
                    match crate::agents::tool_path::execute_tool_call(inv).await {
                        Ok(result) => {
                            if self.trace {
                                info!("[Runtime] Tool '{}' executed → {:?}", inv.name, result);
                            }
                            state.last_tool_result = Some(result);
                        }
                        Err(e) => {
                            warn!("[Runtime] Tool '{}' failed: {}", inv.name, e);
                            state.halt(&format!("Tool execution failed: {}", e));
                        }
                    }
                }
                AgentStep::LLMCall(_) => {
                    if self.trace {
                        debug!("[Runtime] LLMCall step (not yet executed in loop)");
                    }
                }
            }

            state.record_step(step.clone());

            if self.trace {
                debug!("[Runtime] Step {} recorded", state.step_count);
            }
        }

        info!("[Runtime] Agent run completed. Halted={}, steps={}",
              state.halted, state.step_count);

        state
    }
}
