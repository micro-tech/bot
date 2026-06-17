//! ReflectionPlannerAdapter
//!
//! Thin adapter that wraps the existing `planning::planner::Planner`
//! so it can be used by the Unified Agent Runtime Loop.

use crate::agents::agent_state::AgentState;
use crate::agents::agent_step::{AgentStep, ToolInvocation};
use crate::agents::planner::Planner;
use async_trait::async_trait;
use crate::planning::planner::{Planner as OldPlanner, PlannerInterface};
use crate::cpu::interfaces::LlmInterface;
use crate::io::ollama::llm::OllamaLlm;

/// Adapter that makes the legacy planner compatible with the new runtime.
pub struct ReflectionPlannerAdapter<L: LlmInterface + Send + Sync + 'static> {
    inner: OldPlanner,
    goal: String,
    step_index: std::sync::atomic::AtomicUsize,
    _marker: std::marker::PhantomData<L>,
}

impl<L: LlmInterface + Send + Sync + 'static> ReflectionPlannerAdapter<L> {
    pub fn new(goal: String, llm: OllamaLlm) -> Self {
        Self {
            inner: OldPlanner::new(llm),
            goal,
            step_index: std::sync::atomic::AtomicUsize::new(0),
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<L: LlmInterface + Send + Sync + 'static> Planner for ReflectionPlannerAdapter<L> {
    async fn decide(&self, _state: &AgentState) -> AgentStep {
        let idx = self.step_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if idx == 0 {
            match self.inner.decompose(&self.goal).await {
                Ok(subtasks) if !subtasks.is_empty() => {
                    return AgentStep::ToolCall(ToolInvocation {
                        name: "noop".to_string(),
                        args: serde_json::json!({ "subtask": subtasks[0] }),
                        correlation_id: 1,
                    });
                }
                _ => {
                    return AgentStep::FinalAnswer(format!("Completed: {}", self.goal));
                }
            }
        }

        AgentStep::FinalAnswer(format!("Completed: {}", self.goal))
    }
}
