//! Evolution Metrics (Task 38)
use crate::hy_evo::engine::HyEvoEngine;

pub fn collect_metrics(engine: &HyEvoEngine) -> String {
    format!(
        "Population: {}, Best Score: {:.2}, Avg Score: {:.2}",
        engine.population.len(),
        engine.best_workflow().map_or(0.0, |g| g.score),
        engine.average_score()
    )
}
