//! Simple rule-based planner for the Unified Agent Runtime.
//! This is a placeholder until we wire a real LLM planner.

use crate::agents::agent_state::AgentState;
use crate::agents::agent_step::AgentStep;
use async_trait::async_trait;
use crate::agents::planner::Planner;

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
    async fn decide(&self, state: &AgentState) -> AgentStep {
        // Very simple logic: after 3 steps, finish
        if state.step_count >= 3 {
            return AgentStep::FinalAnswer(format!("Completed: {}", self.goal));
        }

        // Otherwise do a dummy tool call
        AgentStep::ToolCall(crate::agents::agent_step::ToolInvocation {
            name: "noop".to_string(),
            args: serde_json::json!({ "note": "simple planner step" }),
            correlation_id: 0,
        })
    }
}
