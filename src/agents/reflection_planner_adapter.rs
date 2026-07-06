//! ReflectionPlannerAdapter
//!
//! Thin adapter that wraps any Planner and adds real reflection capability.
//! This is the key component that gives Hermies/OpenClaw-style self-correction behavior.
//!
//! The adapter:
//!   1. Calls the inner planner's decide()
//!   2. Occasionally triggers reflect() on the inner planner
//!   3. Parses reflection text to potentially override with Replan or improved reasoning
//!
//! IMPORTANT: This adapter does NOT modify planning/planner.rs or planning/reflection.rs.

use crate::agents::agent_state::AgentState;
use crate::agents::planner::Planner;
use crate::agents::planner_output::PlannerOutput;
use async_trait::async_trait;
use std::sync::Arc;

/// Adapter that adds reflection + potential replanning on top of any inner Planner.
pub struct ReflectionPlannerAdapter<P> {
    inner: Arc<P>,
    /// How often to trigger reflection (every N steps). 0 = never, 1 = every step, 3 = every 3rd step, etc.
    reflection_interval: u32,
}

impl<P> ReflectionPlannerAdapter<P> {
    pub fn new(inner: P) -> Self {
        Self {
            inner: Arc::new(inner),
            reflection_interval: 2, // reflect every 2 steps by default
        }
    }

    /// Create adapter with custom reflection frequency.
    pub fn with_reflection_interval(inner: P, interval: u32) -> Self {
        Self {
            inner: Arc::new(inner),
            reflection_interval: interval,
        }
    }
}

#[async_trait]
impl<P> Planner for ReflectionPlannerAdapter<P>
where
    P: Planner + Send + Sync,
{
    async fn decide(&self, state: &AgentState) -> PlannerOutput {
        // Get the base decision from the inner planner
        let mut decision = self.inner.decide(state).await;

        // Trigger reflection on a schedule (or when we have history)
        let should_reflect = self.reflection_interval > 0
            && state.step_count > 0
            && (state.step_count % self.reflection_interval == 0);

        if should_reflect {
            if let Some(reflection_text) = self.inner.reflect(state).await {
                // Parse the reflection text for signals that we should replan
                if self.should_replan(&reflection_text) {
                    return PlannerOutput::Replan {
                        new_goal_focus: self.extract_focus(&reflection_text),
                        reasoning: Some(reflection_text),
                    };
                }

                // Otherwise attach the reflection as reasoning to the existing decision
                decision = self.attach_reasoning(decision, reflection_text);
            }
        }

        decision
    }

    async fn reflect(&self, state: &AgentState) -> Option<String> {
        // Delegate to inner planner's reflect implementation
        self.inner.reflect(state).await
    }
}

impl<P> ReflectionPlannerAdapter<P> {
    /// Heuristic: does this reflection text suggest we should change course?
    fn should_replan(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        lower.contains("better approach")
            || lower.contains("try a different")
            || lower.contains("replan")
            || lower.contains("alternative")
            || lower.contains("mistake")
            || lower.contains("failed because")
    }

    /// Extract a new focus / sub-goal from reflection text.
    fn extract_focus(&self, text: &str) -> String {
        // Very lightweight extraction — in a real system this would be LLM-assisted
        if let Some(idx) = text.to_lowercase().find("focus on") {
            let tail = &text[idx + 8..];
            return tail.split('.').next().unwrap_or(tail).trim().to_string();
        }
        if let Some(idx) = text.to_lowercase().find("instead") {
            let tail = &text[idx + 7..];
            return tail.split('.').next().unwrap_or(tail).trim().to_string();
        }
        "revised approach based on reflection".to_string()
    }

    /// Attach reflection text as reasoning to a PlannerOutput.
    fn attach_reasoning(&self, mut output: PlannerOutput, reasoning: String) -> PlannerOutput {
        match &mut output {
            PlannerOutput::LLMCall { reasoning: r, .. } => *r = Some(reasoning),
            PlannerOutput::ToolCall { reasoning: r, .. } => *r = Some(reasoning),
            PlannerOutput::FinalAnswer { reasoning: r, .. } => *r = Some(reasoning),
            PlannerOutput::Error { .. } => { /* leave as-is */ }
            PlannerOutput::Replan { reasoning: r, .. } => *r = Some(reasoning),
        }
        output
    }
}
