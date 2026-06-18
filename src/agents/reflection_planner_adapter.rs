//! ReflectionPlannerAdapter
//!
//! Thin adapter that wraps the existing planning module so it can be used
//! by the Unified Agent Runtime Loop. It parses reflection text into
//! structured PlannerOutput actions.
//!
//! IMPORTANT: This adapter does NOT modify planning/planner.rs or planning/reflection.rs.

use crate::agents::agent_state::AgentState;
use crate::agents::planner::Planner;
use crate::agents::planner_output::PlannerOutput;
use async_trait::async_trait;
use std::sync::Arc;

/// Adapter that converts reflection-based planner output into structured PlannerOutput.
pub struct ReflectionPlannerAdapter<P> {
    inner: Arc<P>,
}

impl<P> ReflectionPlannerAdapter<P> {
    pub fn new(inner: P) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl<P> Planner for ReflectionPlannerAdapter<P>
where
    P: Send + Sync,
{
    async fn decide(&self, state: &AgentState) -> PlannerOutput {
        // For now we use a simple heuristic-based parser on the last message.
        // In a real implementation this would call the inner planner's reflect() method.

        let last_message = state
            .messages
            .last()
            .cloned()
            .unwrap_or_default();

        // Simple string-based parsing of reflection text
        let lower = last_message.to_lowercase();

        if lower.contains("use tool:") {
            return PlannerOutput::ToolCall {
                tool_name: "noop".to_string(),
                args_json: serde_json::json!({}),
                reasoning: Some(last_message),
            };
        }

        if lower.contains("ask llm:") {
            return PlannerOutput::LLMCall {
                prompt: "follow-up question".to_string(),
                reasoning: Some(last_message),
            };
        }

        if lower.contains("final answer:") {
            return PlannerOutput::FinalAnswer {
                message: last_message.replace("final answer:", "").trim().to_string(),
                reasoning: Some(last_message),
            };
        }

        // Default: return error if we cannot interpret the reflection
        PlannerOutput::Error {
            message: "Planner could not interpret reflection output".into(),
        }
    }
}
