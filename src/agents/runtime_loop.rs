//! Core runtime loop skeleton for the Unified Agent Runtime.
//! This is the "brainstem" that orchestrates everything.

use crate::agents::agent_state::AgentState;
use crate::agents::planner::Planner;
use crate::agents::planner_output::PlannerOutput;
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

            // NEW: Planner now returns PlannerOutput (not AgentStep)
            let decision = self.planner.decide(&state).await;

            if self.trace {
                match &decision {
                    PlannerOutput::ToolCall { tool_name, args_json, .. } => {
                        info!("[Runtime] Planner chose ToolCall: {} (args={})", tool_name, args_json);
                    }
                    PlannerOutput::LLMCall { prompt, .. } => {
                        info!("[Runtime] Planner chose LLMCall (prompt_len={})", prompt.len());
                    }
                    PlannerOutput::FinalAnswer { message, .. } => {
                        info!("[Runtime] Planner chose FinalAnswer: {}", message);
                    }
                    PlannerOutput::Error { message } => {
                        warn!("[Runtime] Planner returned Error: {}", message);
                    }
                }
            }

            // Convert PlannerOutput into runtime action
            match decision {
                PlannerOutput::FinalAnswer { message, .. } => {
                    state.halt("Terminal step reached");
                    crate::agents::bus_events::emit_agent_finished(&state, &message);
                    info!("[Runtime] Agent finished successfully");
                    break;
                }
                PlannerOutput::Error { message } => {
                    state.halt("Terminal step reached");
                    crate::agents::bus_events::emit_agent_error(&state, &message);
                    warn!("[Runtime] Agent halted with error: {}", message);
                    break;
                }
                PlannerOutput::ToolCall { tool_name, args_json, .. } => {
                    // Execute tool via the tool path
                    let inv = crate::agents::agent_step::ToolInvocation {
                        name: tool_name,
                        args: args_json,
                        correlation_id: state.step_count as u64,
                    };
                    match crate::agents::tool_path::execute_tool_call(&inv).await {
                        Ok(result) => {
                            if self.trace {
                                info!("[Runtime] Tool '{}' executed → {:?}", inv.name, result);
                            }
                            state.last_tool_result = Some(result);
                        }
                        Err(e) => {
                            warn!("[Runtime] Tool '{}' failed: {}", inv.name, e);
                            state.halt(&format!("Tool execution failed: {}", e));
                            break;
                        }
                    }
                }
                PlannerOutput::LLMCall { prompt, .. } => {
                    if self.trace {
                        debug!("[Runtime] LLMCall step (prompt_len={})", prompt.len());
                    }
                    // In a full implementation this would call the LLM path here
                }
            }

            // Record a lightweight step marker
            state.record_step(crate::agents::agent_step::AgentStep::LLMCall(
                crate::agents::agent_step::LLMCallSpec {
                    backend: "planner".into(),
                    prompt: "planner decision".into(),
                    correlation_id: state.step_count as u64,
                },
            ));

            if self.trace {
                debug!("[Runtime] Step {} recorded", state.step_count);
            }
        }

        info!("[Runtime] Agent run completed. Halted={}, steps={}",
              state.halted, state.step_count);

        state
    }
}
