//! Core runtime loop skeleton for the Unified Agent Runtime.
//! This is the "brainstem" that orchestrates everything.

use crate::agents::agent_state::AgentState;
use crate::agents::planner::Planner;
use crate::agents::planner_output::PlannerOutput;
use crate::agents::rule_layer::RuleLayer;
use crate::agents::runtime_trace::{RuntimeStep, RuntimeTrace};
use log::{debug, info, warn};

pub struct RuntimeLoop<P: Planner> {
    planner: P,
    rule_layer: RuleLayer,
    max_steps: u32,
    trace: bool,
    pub runtime_trace: RuntimeTrace,   // NEW: structured trace for 157
}

impl<P: Planner> RuntimeLoop<P> {
    pub fn new(planner: P, rule_layer: RuleLayer, max_steps: u32) -> Self {
        Self {
            planner,
            rule_layer,
            max_steps,
            trace: false,
            runtime_trace: RuntimeTrace::new(),
        }
    }

    /// Enable or disable detailed per-step trace logging.
    pub fn with_trace(mut self, enabled: bool) -> Self {
        self.trace = enabled;
        self
    }

    /// Run one full agent execution cycle.
    pub async fn run(&mut self, mut state: AgentState) -> AgentState {
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
            let mut step = RuntimeStep {
                step_number: state.step_count,
                planner_output: Some(decision.clone()),
                ..Default::default()
            };

            match decision {
                PlannerOutput::FinalAnswer { message, .. } => {
                    state.halt("Terminal step reached");
                    crate::agents::bus_events::emit_agent_finished(&state, &message);
                    info!("[Runtime] Agent finished successfully");
                    step.halted = true;
                    self.runtime_trace.push(step);
                    break;
                }
                PlannerOutput::Error { message } => {
                    state.halt("Terminal step reached");
                    crate::agents::bus_events::emit_agent_error(&state, &message);
                    warn!("[Runtime] Agent halted with error: {}", message);
                    step.error = Some(message);
                    step.halted = true;
                    self.runtime_trace.push(step);
                    break;
                }
                PlannerOutput::ToolCall { tool_name, args_json, .. } => {
                    step.tool_name = Some(tool_name.clone());
                    step.tool_args = Some(args_json.clone());

                    // Use ToolSupervisorV2 for validated + retried execution
                    let supervisor = crate::agents::tool_supervisor_v2::ToolSupervisorV2::default();
                    let tool_result = supervisor.execute(&tool_name, &args_json).await;

                    match tool_result {
                        crate::agents::tool_supervisor_v2::ToolResult::Success(value) => {
                            if self.trace {
                                info!("[Runtime] Tool '{}' succeeded → {:?}", tool_name, value);
                            }
                            state.last_tool_result = Some(value.clone());
                            state.messages.push(format!(
                                "Tool '{}' returned: {}",
                                tool_name,
                                serde_json::to_string(&value).unwrap_or_default()
                            ));
                            step.tool_result = Some(value);
                        }
                        crate::agents::tool_supervisor_v2::ToolResult::RetryableError(err) => {
                            warn!("[Runtime] Tool '{}' retryable error: {}", tool_name, err.message());
                            state.last_tool_result = Some(serde_json::json!({
                                "error": err.message(),
                                "retryable": true
                            }));
                            step.error = Some(err.message().to_string());
                        }
                        crate::agents::tool_supervisor_v2::ToolResult::FatalError(err) => {
                            warn!("[Runtime] Tool '{}' fatal error: {}", tool_name, err.message());
                            state.halt(&format!("Tool failed: {}", err.message()));
                            step.error = Some(err.message().to_string());
                            step.halted = true;
                            self.runtime_trace.push(step);
                            break;
                        }
                    }
                }
                PlannerOutput::LLMCall { prompt, .. } => {
                    if self.trace {
                        debug!("[Runtime] LLMCall step (prompt_len={})", prompt.len());
                    }
                    state.messages.push(format!("LLM called with prompt: {}", prompt));
                    step.llm_prompt = Some(prompt);
                }
            }

            self.runtime_trace.push(step);

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
