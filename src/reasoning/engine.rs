//! ReasoningEngine implementation (Tasks 68-74)
//!
//! Core reasoning engine that manages state, integrates with Bayesian reasoning,
//! memory, and skill arbitration, supports self-correction, and provides observability.

use crate::reasoning::state::{ReasoningState, ReasoningPhase, Hypothesis, Plan, PlanStep};
use crate::bus::Bus;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, debug};
use uuid::Uuid;

/// Main reasoning engine
pub struct ReasoningEngine {
    state: Arc<RwLock<ReasoningState>>,
    bus: Bus,
    /// Maximum self-correction cycles before giving up
    max_correction_cycles: u32,
}

impl ReasoningEngine {
    pub fn new(goal: impl Into<String>, bus: Bus) -> Self {
        let state = ReasoningState::new(goal);
        Self {
            state: Arc::new(RwLock::new(state)),
            bus,
            max_correction_cycles: 3,
        }
    }

    /// Get current state snapshot
    pub async fn get_state(&self) -> ReasoningState {
        self.state.read().await.clone()
    }

    /// Start the reasoning process
    pub async fn start(&self) -> Result<()> {
        let mut state = self.state.write().await;
        state.transition(ReasoningPhase::ExpandOptions).map_err(|e| anyhow::anyhow!(e))?;
        info!(correlation_id = %state.correlation_id, goal = %state.goal, "Reasoning started");

        // Task 74: Emit observability event
        crate::reasoning::observability::ReasoningObserver::log_event(
            &state,
            "reasoning_started",
            None,
        );

        Ok(())
    }

    /// Add a hypothesis with initial belief
    pub async fn propose_hypothesis(&self, description: impl Into<String>) -> Uuid {
        let mut state = self.state.write().await;
        let hyp = Hypothesis::new(description);
        let id = hyp.id;
        state.add_hypothesis(hyp);
        debug!(correlation_id = %state.correlation_id, hyp_id = %id, "Hypothesis proposed");

        crate::reasoning::observability::ReasoningObserver::log_event(
            &state,
            "hypothesis_proposed",
            Some(serde_json::json!({ "hyp_id": id.to_string() })),
        );

        id
    }

    /// Update belief for a hypothesis based on evidence
    pub async fn update_belief(&self, hyp_id: Uuid, evidence: impl Into<String>, is_supporting: bool) {
        let mut state = self.state.write().await;
        let _evidence_str = evidence.into();
        if let Some(hyp) = state.hypotheses.iter_mut().find(|h| h.id == hyp_id) {
            hyp.update_belief(1.0, is_supporting);
            let belief = hyp.belief;
            debug!(
                correlation_id = %state.correlation_id,
                hyp_id = %hyp_id,
                belief = belief,
                "Belief updated"
            );
        }
    }

    /// Create a plan from current hypotheses
    pub async fn create_plan(&self) -> Result<Uuid> {
        let mut state = self.state.write().await;
        state.transition(ReasoningPhase::CommitPlan).map_err(|e| anyhow::anyhow!(e))?;

        let mut plan = Plan::new(&state.goal);
        // Simple heuristic: pick highest-belief hypothesis and create steps
        if let Some(best) = state.hypotheses.iter().max_by(|a, b| a.belief.partial_cmp(&b.belief).unwrap()) {
            plan.confidence = best.belief;
            // Create a placeholder step for the hypothesis
            let step = PlanStep::new(
                format!("Investigate: {}", best.description),
                "llm",
            );
            plan.add_step(step);
        }

        let plan_id = plan.id;
        state.current_plan = Some(plan);
        info!(correlation_id = %state.correlation_id, plan_id = %plan_id, "Plan committed");

        crate::reasoning::observability::ReasoningObserver::log_event(
            &state,
            "plan_committed",
            Some(serde_json::json!({ "plan_id": plan_id.to_string() })),
        );

        Ok(plan_id)
    }

    /// Execute the next step of the current plan
    pub async fn execute_next_step(&self) -> Result<bool> {
        let mut state = self.state.write().await;

        if state.current_plan.is_none() {
            return Err(anyhow::anyhow!("No plan to execute"));
        }

        state.transition(ReasoningPhase::ExecuteStep).map_err(|e| anyhow::anyhow!(e))?;

        let is_complete = {
            let plan = state.current_plan.as_ref().unwrap();
            plan.is_complete()
        };

        if is_complete {
            state.transition(ReasoningPhase::Completed).map_err(|e| anyhow::anyhow!(e))?;
            info!(correlation_id = %state.correlation_id, "Plan completed");
            return Ok(false);
        }

        {
            let plan = state.current_plan.as_ref().unwrap();
            if let Some(step) = plan.current_step() {
                debug!(
                    correlation_id = %state.correlation_id,
                    step = %step.description,
                    "Executing plan step"
                );
            }
        }

        // Advance to next step
        if let Some(plan) = state.current_plan.as_mut() {
            plan.advance();
        }
        Ok(true)
    }

    /// Trigger self-correction when evidence contradicts assumptions
    pub async fn trigger_self_correction(&self, reason: impl Into<String>) -> Result<()> {
        let mut state = self.state.write().await;

        if state.correction_cycles >= self.max_correction_cycles {
            warn!(
                correlation_id = %state.correlation_id,
                cycles = state.correction_cycles,
                "Max correction cycles reached, failing"
            );
            state.transition(ReasoningPhase::Failed).map_err(|e| anyhow::anyhow!(e))?;
            return Err(anyhow::anyhow!("Max self-correction cycles exceeded"));
        }

        state.correction_cycles += 1;
        state.transition(ReasoningPhase::SelfCorrect).map_err(|e| anyhow::anyhow!(e))?;

        let reason_str = reason.into();
        info!(
            correlation_id = %state.correlation_id,
            reason = %reason_str,
            cycle = state.correction_cycles,
            "Self-correction triggered"
        );

        // Use the SelfCorrectionLoop logger (was unused)
        let correction_loop = crate::reasoning::self_correction::SelfCorrectionLoop::new(self.max_correction_cycles);
        correction_loop.log_correction(&state.correlation_id, state.correction_cycles, &reason_str);

        // Transition back to revision
        state.transition(ReasoningPhase::RevisePlan).map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }

    /// Record a memory reference for this reasoning session
    pub async fn link_memory(&self, key: impl Into<String>) {
        let mut state = self.state.write().await;
        state.add_memory_ref(key);
        debug!(correlation_id = %state.correlation_id, "Memory linked");
    }

    /// Get correlation ID for observability
    pub async fn correlation_id(&self) -> String {
        self.state.read().await.correlation_id.clone()
    }

    /// Visualize current beliefs as Mermaid diagram (uses MemoryManager::visualize_beliefs)
    pub async fn visualize_beliefs(&self) -> String {
        // Note: This is a placeholder - in a full integration we'd pass the actual MemoryManager
        "graph TD;\n    reasoning[\"Reasoning beliefs visualization\"];".to_string()
    }

    /// Get a redacted summary of current reasoning state
    pub async fn reasoning_summary(&self) -> serde_json::Value {
        let state = self.state.read().await;
        crate::reasoning::observability::ReasoningObserver::summary(&state)
    }

    /// Get a detailed trace (for debug mode)
    pub async fn reasoning_trace(&self) -> serde_json::Value {
        let state = self.state.read().await;
        crate::reasoning::observability::ReasoningObserver::detailed_trace(&state)
    }
}
