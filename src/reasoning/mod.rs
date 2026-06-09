//! Reasoning Engine — Core state model and transitions for goal-driven reasoning.
//!
//! Implements tasks 68-74: Core state, Bayesian integration, planning, memory/skill
//! integration, self-correction, and observability.

pub mod state;
pub mod engine;
pub mod planning;
pub mod self_correction;
pub mod observability;

#[allow(unused_imports)]
pub use state::{ReasoningState, ReasoningPhase, Hypothesis, PlanStep};
#[allow(unused_imports)]
pub use engine::ReasoningEngine;
#[allow(unused_imports)]
pub use planning::ExecutionPlan;
