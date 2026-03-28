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

pub mod crossover;
pub mod engine;
pub mod genome;
pub mod integration;
pub mod mutation;
pub mod node;
pub mod reflection;
pub mod scoring;
pub mod workflow;

// Re‑exports for convenience
pub use crossover::*;
pub use engine::*;
pub use genome::*;
pub use integration::*;
pub use mutation::*;
pub use node::*;
pub use reflection::*;
pub use scoring::*;
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
