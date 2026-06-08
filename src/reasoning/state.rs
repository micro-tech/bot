//! Reasoning state model (Task 68)
//!
//! Defines the core state structures for the reasoning engine including goals,
//! hypotheses, plans, uncertainty metrics, and state transitions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// High-level phase of reasoning execution
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningPhase {
    /// Initial goal analysis
    AnalyzeGoal,
    /// Generate hypotheses/options
    ExpandOptions,
    /// Score and evaluate options
    EvaluateOptions,
    /// Commit to a plan
    CommitPlan,
    /// Execute a plan step
    ExecuteStep,
    /// Revise plan based on new evidence
    RevisePlan,
    /// Self-correction triggered
    SelfCorrect,
    /// Completed successfully
    Completed,
    /// Failed with error
    Failed,
}

/// A single hypothesis with Bayesian belief
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: Uuid,
    pub description: String,
    /// Bayesian belief score [0.0, 1.0]
    pub belief: f64,
    /// Evidence supporting this hypothesis
    pub supporting_evidence: Vec<String>,
    /// Evidence against this hypothesis
    pub contradicting_evidence: Vec<String>,
}

impl Hypothesis {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: description.into(),
            belief: 0.5,
            supporting_evidence: Vec::new(),
            contradicting_evidence: Vec::new(),
        }
    }

    /// Update belief based on new evidence (simple Bayesian update)
    pub fn update_belief(&mut self, evidence_weight: f64, is_supporting: bool) {
        // Simple odds-ratio update
        let likelihood = if is_supporting { 0.8 } else { 0.2 };
        let prior_odds = self.belief / (1.0 - self.belief).max(0.001);
        let likelihood_ratio = if is_supporting { likelihood / (1.0 - likelihood) } else { (1.0 - likelihood) / likelihood };
        let posterior_odds = prior_odds * likelihood_ratio;
        self.belief = posterior_odds / (1.0 + posterior_odds);

        if is_supporting {
            self.supporting_evidence.push(format!("weight:{:.2}", evidence_weight));
        } else {
            self.contradicting_evidence.push(format!("weight:{:.2}", evidence_weight));
        }
    }
}

/// A single step in a multi-step plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: Uuid,
    pub description: String,
    /// Tool/skill to invoke, or "llm", "memory_read", "memory_write"
    pub action_type: String,
    /// Parameters for the action
    pub params: serde_json::Value,
    /// Expected output description
    pub expected_output: Option<String>,
    /// Actual result after execution
    pub result: Option<serde_json::Value>,
    /// Whether this step succeeded
    pub success: Option<bool>,
}

impl PlanStep {
    pub fn new(description: impl Into<String>, action_type: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: description.into(),
            action_type: action_type.into(),
            params: serde_json::json!({}),
            expected_output: None,
            result: None,
            success: None,
        }
    }
}

/// A multi-step plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub goal: String,
    pub steps: Vec<PlanStep>,
    pub current_step_index: usize,
    /// Overall plan confidence [0.0, 1.0]
    pub confidence: f64,
}

impl Plan {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            goal: goal.into(),
            steps: Vec::new(),
            current_step_index: 0,
            confidence: 0.5,
        }
    }

    pub fn add_step(&mut self, step: PlanStep) {
        self.steps.push(step);
    }

    pub fn current_step(&self) -> Option<&PlanStep> {
        self.steps.get(self.current_step_index)
    }

    pub fn advance(&mut self) -> bool {
        if self.current_step_index + 1 < self.steps.len() {
            self.current_step_index += 1;
            true
        } else {
            false
        }
    }

    pub fn is_complete(&self) -> bool {
        self.current_step_index >= self.steps.len()
    }
}

/// Core reasoning engine state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningState {
    pub session_id: Uuid,
    pub phase: ReasoningPhase,
    /// The primary goal being pursued
    pub goal: String,
    /// Current hypotheses under consideration
    pub hypotheses: Vec<Hypothesis>,
    /// Current active plan
    pub current_plan: Option<Plan>,
    /// Memory references relevant to this reasoning session
    pub memory_refs: Vec<String>,
    /// Uncertainty metrics
    pub uncertainty: f64,
    /// Number of self-correction cycles used
    pub correction_cycles: u32,
    /// Correlation ID for tracing across logs
    pub correlation_id: String,
    /// Arbitrary metadata
    pub metadata: HashMap<String, serde_json::Value>,
}

impl ReasoningState {
    pub fn new(goal: impl Into<String>) -> Self {
        let goal = goal.into();
        Self {
            session_id: Uuid::new_v4(),
            phase: ReasoningPhase::AnalyzeGoal,
            goal: goal.clone(),
            hypotheses: Vec::new(),
            current_plan: None,
            memory_refs: Vec::new(),
            uncertainty: 0.8,
            correction_cycles: 0,
            correlation_id: Uuid::new_v4().to_string(),
            metadata: HashMap::new(),
        }
    }

    /// Transition to a new phase (validates allowed transitions)
    pub fn transition(&mut self, new_phase: ReasoningPhase) -> Result<(), String> {
        let allowed = match (&self.phase, &new_phase) {
            (ReasoningPhase::AnalyzeGoal, ReasoningPhase::ExpandOptions) => true,
            (ReasoningPhase::ExpandOptions, ReasoningPhase::EvaluateOptions) => true,
            (ReasoningPhase::EvaluateOptions, ReasoningPhase::CommitPlan) => true,
            (ReasoningPhase::CommitPlan, ReasoningPhase::ExecuteStep) => true,
            (ReasoningPhase::ExecuteStep, ReasoningPhase::ExecuteStep) => true, // stay in step
            (ReasoningPhase::ExecuteStep, ReasoningPhase::RevisePlan) => true,
            (ReasoningPhase::ExecuteStep, ReasoningPhase::Completed) => true,
            (ReasoningPhase::ExecuteStep, ReasoningPhase::Failed) => true,
            (ReasoningPhase::RevisePlan, ReasoningPhase::ExecuteStep) => true,
            (ReasoningPhase::RevisePlan, ReasoningPhase::SelfCorrect) => true,
            (ReasoningPhase::SelfCorrect, ReasoningPhase::RevisePlan) => true,
            (ReasoningPhase::SelfCorrect, ReasoningPhase::Failed) => true,
            _ => false,
        };

        if allowed {
            self.phase = new_phase;
            Ok(())
        } else {
            Err(format!("Invalid transition from {:?} to {:?}", self.phase, new_phase))
        }
    }

    /// Add a hypothesis and update uncertainty
    pub fn add_hypothesis(&mut self, hyp: Hypothesis) {
        self.hypotheses.push(hyp);
        // More hypotheses = more uncertainty initially
        self.uncertainty = (self.uncertainty * 0.9).max(0.1);
    }

    /// Record a memory reference
    pub fn add_memory_ref(&mut self, key: impl Into<String>) {
        let key = key.into();
        if !self.memory_refs.contains(&key) {
            self.memory_refs.push(key);
        }
    }
}
