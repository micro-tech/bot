//! Planner trait — defines the interface for deciding the next AgentStep.
//!
//! Security: The planner must never return hallucinated tool names.
//! All tool names returned must be validated by the Rule Layer.

use crate::agents::agent_state::AgentState;
use crate::agents::agent_step::AgentStep;
use async_trait::async_trait;

#[async_trait]
pub trait Planner: Send + Sync {
    /// Given the current agent state, decide the next action.
    async fn decide(&self, state: &AgentState) -> AgentStep;
}
