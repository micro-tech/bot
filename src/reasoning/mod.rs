//! Reasoning Engine — Core state model and transitions for goal-driven reasoning.
//!
//! Implements tasks 68-74: Core state, Bayesian integration, planning, memory/skill
//! integration, self-correction, and observability.

#![allow(dead_code)]

pub mod engine;
pub mod observability;
pub mod planning;
pub mod self_correction;
pub mod state;

#[allow(unused_imports)]
pub use engine::ReasoningEngine;
#[allow(unused_imports)]
pub use planning::ExecutionPlan;
#[allow(unused_imports)]
pub use state::{Hypothesis, PlanStep, ReasoningPhase, ReasoningState};
