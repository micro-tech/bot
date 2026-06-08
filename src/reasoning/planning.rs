//! Planning layer for multi-step execution (Task 70)

use crate::reasoning::state::PlanStep;
use serde::{Deserialize, Serialize};

/// Extended plan representation with execution tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub plan: crate::reasoning::state::Plan,
    pub executed_steps: Vec<PlanStep>,
    pub failed_steps: Vec<(PlanStep, String)>,
}

impl ExecutionPlan {
    pub fn from_plan(plan: crate::reasoning::state::Plan) -> Self {
        Self {
            plan,
            executed_steps: Vec::new(),
            failed_steps: Vec::new(),
        }
    }

    pub fn record_success(&mut self, step: PlanStep) {
        self.executed_steps.push(step);
    }

    pub fn record_failure(&mut self, step: PlanStep, error: impl Into<String>) {
        self.failed_steps.push((step, error.into()));
    }
}
