//! Planner trait for the Unified Agent Runtime Loop.
//! This is the decision interface used by RuntimeLoop.

use crate::agents::agent_state::AgentState;
use crate::agents::planner_output::PlannerOutput;
use async_trait::async_trait;

#[async_trait]
pub trait Planner: Send + Sync {
    /// Decide the next action based on current agent state.
    async fn decide(&self, state: &AgentState) -> PlannerOutput;
}
