//! Simple rule-based planner for the Unified Agent Runtime.
//! This is a placeholder until we wire a real LLM planner.

use crate::agents::agent_state::AgentState;
use crate::agents::planner::Planner;
use crate::agents::planner_output::PlannerOutput;
use async_trait::async_trait;

pub struct SimplePlanner {
    pub goal: String,
}

impl SimplePlanner {
    pub fn new(goal: String) -> Self {
        Self { goal }
    }
}

#[async_trait]
impl Planner for SimplePlanner {
    async fn decide(&self, state: &AgentState) -> PlannerOutput {
        // Very simple logic: after 3 steps, finish
        if state.step_count >= 3 {
            return PlannerOutput::FinalAnswer {
                message: format!("Completed: {}", self.goal),
                reasoning: None,
            };
        }

        // Otherwise do a dummy tool call
        PlannerOutput::ToolCall {
            tool_name: "noop".to_string(),
            args_json: serde_json::json!({ "note": "simple planner step" }),
            reasoning: None,
        }
    }
}
