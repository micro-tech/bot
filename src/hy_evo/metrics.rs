//! Evolution Metrics (Task 38)
use crate::hy_evo::engine::HyEvoEngine;
use crate::hy_evo::reflection::ReflectionLlm;

pub fn collect_metrics<L: ReflectionLlm + Send + Sync>(engine: &HyEvoEngine<L>) -> String {
    format!(
        "Population: {}, Best Score: {:.2}, Avg Score: {:.2}",
        engine.population.len(),
        engine.best_workflow().map_or(0.0, |g| g.score),
        engine.average_score()
    )
}
