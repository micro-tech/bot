//! HyEvo: Hybrid Evolution Engine
//!
//! This module implements the full evolutionary workflow optimizer
//! for your agent OS. It includes:
//! - Workflow genome representation
//! - Node definitions
//! - Mutation engine
//! - Crossover engine
//! - Scoring system
//! - Reflection loop
//! - Evolution engine
//! - CPU integration helpers

pub mod ab_testing;
pub mod chaos;
pub mod crossover;
pub mod diff;
pub mod engine;
pub mod failure_injection;
pub mod genome;
pub mod integration;
pub mod metrics;
pub mod mutation;
pub mod node;
pub mod population;
pub mod reflection;
pub mod registry;
pub mod scoring;
pub mod selection;
pub mod storage;
pub mod templates;
pub mod tests;
pub mod triggers;
pub mod visualization;
pub mod workflow;

// Re‑exports for convenience
#[allow(unused_imports)]
pub use crossover::*;
#[allow(unused_imports)]
pub use engine::*;
#[allow(unused_imports)]
pub use genome::*;
#[allow(unused_imports)]
pub use integration::*;
#[allow(unused_imports)]
pub use mutation::*;
#[allow(unused_imports)]
pub use node::*;
#[allow(unused_imports)]
pub use reflection::*;
#[allow(unused_imports)]
pub use scoring::*;
#[allow(unused_imports)]
pub use workflow::*;

/// HyEvo version identifier
pub const HYEVO_VERSION: &str = "0.1.0";

/// Entry point for running a full evolution cycle.
/// The CPU will call this when:
/// - workflows need improvement
/// - a failure occurs
/// - periodic cron triggers fire
pub async fn evolve_once() -> anyhow::Result<()> {
    // Placeholder — your engine.rs will implement this.
    // This function stays here so the CPU has a stable API.
    Ok(())
}

/// Initialize HyEvo system (load workflows, configs, etc.)
pub async fn init() -> anyhow::Result<()> {
    // Placeholder — integration.rs will implement this.
    Ok(())
}
