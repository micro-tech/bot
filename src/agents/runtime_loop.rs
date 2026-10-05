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

        // Task 210: consecutive tool-call counter feeding the rule layer's
        // runaway-loop check. Attempts count even when rejected, and any
        // non-tool decision resets it.
        let mut consecutive_tool_calls: u32 = 0;

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
                    PlannerOutput::Replan { new_goal_focus, .. } => {
                        info!("[Runtime] Planner chose Replan: focus on '{}'", new_goal_focus);
                    }
                }
            }

            // Convert PlannerOutput into runtime action
            let mut step = RuntimeStep {
                step_number: state.step_count,
                planner_output: Some(decision.clone()),
                ..Default::default()
            };

            // Task 210: the rule layer's consecutive-call check only makes
            // sense across back-to-back tool decisions — any other decision
            // (answer, replan, LLM call, error) breaks the streak.
            if !matches!(decision, PlannerOutput::ToolCall { .. }) {
                consecutive_tool_calls = 0;
            }

            match decision {
                PlannerOutput::FinalAnswer { message, .. } => {
                    state.halt("Terminal step reached");
                    crate::agents::bus_events::emit_agent_finished(&state, &message);
                    info!("[Runtime] Agent finished successfully");
                    step.halted = true;
                    self.runtime_trace.push(step);
                    // The terminal answer is a step of work too: record it so
                    // state.step_count agrees with the runtime trace (the
                    // break below would otherwise skip the bottom-of-loop
                    // record_step, leaving a finished run at step 0).
                    state.record_step(crate::agents::agent_step::AgentStep::LLMCall(
                        crate::agents::agent_step::LLMCallSpec {
                            backend: "planner".into(),
                            prompt: "final answer".into(),
                            correlation_id: state.step_count as u64,
                        },
                    ));
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
                PlannerOutput::Replan { new_goal_focus, reasoning } => {
                    // Reflection suggested a better path — update state and continue
                    if self.trace {
                        info!("[Runtime] Reflection triggered REPLAN → focus: {}", new_goal_focus);
                    }
                    state.messages.push(format!(
                        "Reflection: replan with focus '{}' — {}",
                        new_goal_focus,
                        reasoning.as_deref().unwrap_or("no reasoning provided")
                    ));
                    step.planner_output = Some(PlannerOutput::Replan {
                        new_goal_focus: new_goal_focus.clone(),
                        reasoning: reasoning.clone(),
                    });
                    // Push the step and continue to next iteration (reflection-driven replan)
                    self.runtime_trace.push(step);
                    continue;
                }
                PlannerOutput::ToolCall { tool_name, args_json, .. } => {
                    step.tool_name = Some(tool_name.clone());
                    step.tool_args = Some(args_json.clone());

                    // Task 210: the rule layer was constructed but never
                    // consulted — its hallucinated-tool / runaway-loop /
                    // malformed-args checks were dead code. Validate BEFORE
                    // execution so a rejection is recoverable: it lands in
                    // the transcript and the planner can pick a valid tool
                    // next step. (ToolSupervisorV2 still validates at
                    // execution time, but a failure there halts the run.)
                    let invocation = crate::agents::agent_step::AgentStep::ToolCall(
                        crate::agents::agent_step::ToolInvocation {
                            name: tool_name.clone(),
                            args: args_json.clone(),
                            correlation_id: state.step_count as u64,
                        },
                    );
                    if let Err(reason) = self.rule_layer.validate(&invocation, consecutive_tool_calls) {
                        // The attempt counts: a planner hammering rejected
                        // calls must still trip the consecutive-call limit.
                        consecutive_tool_calls += 1;
                        warn!("[Runtime] Rule layer rejected tool '{}': {}", tool_name, reason);
                        let msg = format!("Rule layer rejected tool call '{}': {}", tool_name, reason);
                        state.messages.push(msg.clone());
                        state.last_tool_result =
                            Some(serde_json::json!({ "error": msg, "rejected": true }));
                        step.error = Some(msg);
                        self.runtime_trace.push(step);
                        // Record the step so a stuck planner burns its step
                        // budget instead of spinning forever on rejections.
                        state.record_step(crate::agents::agent_step::AgentStep::LLMCall(
                            crate::agents::agent_step::LLMCallSpec {
                                backend: "rule-layer".into(),
                                prompt: "rejected tool call".into(),
                                correlation_id: state.step_count as u64,
                            },
                        ));
                        continue;
                    }
                    consecutive_tool_calls += 1;

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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// Planner that tries a hallucinated tool once, then answers.
    struct HallucinatingPlanner;
    #[async_trait]
    impl Planner for HallucinatingPlanner {
        async fn decide(&self, state: &AgentState) -> PlannerOutput {
            if state.step_count == 0 {
                return PlannerOutput::ToolCall {
                    tool_name: "delete_everything".to_string(),
                    args_json: serde_json::json!({}),
                    reasoning: None,
                };
            }
            PlannerOutput::FinalAnswer {
                message: "done".to_string(),
                reasoning: None,
            }
        }
    }

    /// Planner stuck in a tool loop: calls system_status every step.
    struct LoopyPlanner;
    #[async_trait]
    impl Planner for LoopyPlanner {
        async fn decide(&self, _state: &AgentState) -> PlannerOutput {
            PlannerOutput::ToolCall {
                tool_name: "system_status".to_string(),
                args_json: serde_json::json!({}),
                reasoning: None,
            }
        }
    }

    #[tokio::test]
    async fn rule_layer_rejects_hallucinated_tool_before_execution() {
        // GIVEN a loop whose rule layer only allows system_status:
        let mut runtime = RuntimeLoop::new(
            HallucinatingPlanner,
            RuleLayer::new(vec!["system_status".into()]),
            10,
        );
        // WHEN the planner tries a hallucinated tool, then answers:
        let final_state = runtime.run(AgentState::new()).await;
        // THEN the run still completes via FinalAnswer (no fatal halt)...
        assert!(final_state.halted);
        assert_eq!(runtime.runtime_trace.steps.len(), 2);
        // ...AND the bad call was rejected before execution — never ran.
        let first = &runtime.runtime_trace.steps[0];
        let err = first.error.as_ref().expect("rejected step should carry an error");
        assert!(err.contains("Rule layer rejected"), "unexpected error: {err}");
        assert!(err.contains("delete_everything"), "unexpected error: {err}");
        assert!(first.tool_result.is_none(), "rejected call must not execute");
    }

    #[tokio::test]
    async fn rule_layer_stops_consecutive_tool_loop() {
        // GIVEN a planner stuck calling an allowed tool forever, 10 max steps:
        let mut runtime = RuntimeLoop::new(
            LoopyPlanner,
            RuleLayer::new(vec!["system_status".into()]),
            10,
        );
        // WHEN the run executes:
        let final_state = runtime.run(AgentState::new()).await;
        // THEN the 6th consecutive call trips the runaway-loop check...
        let runaway_rejections = runtime
            .runtime_trace
            .steps
            .iter()
            .filter_map(|s| s.error.as_ref())
            .filter(|e| e.contains("Too many consecutive tool calls"))
            .count();
        assert!(
            runaway_rejections > 0,
            "expected a consecutive-call rejection in the trace"
        );
        // ...AND the step budget still bounds the run (no infinite spin).
        assert!(final_state.step_count <= 10);
        assert_eq!(runtime.runtime_trace.steps.len(), 10);
    }
}
