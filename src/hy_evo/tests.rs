//! Basic tests for the HyEvo evolution system.
#[allow(unused_imports)]
use crate::hy_evo::engine::HyEvoEngine;
use crate::hy_evo::genome::WorkflowGenome;
use crate::hy_evo::reflection::ReflectionLlm;
use crate::hy_evo::scoring::ExecutionMetrics;
use async_trait::async_trait;

struct DummyLlm;

#[async_trait]
impl ReflectionLlm for DummyLlm {
    async fn reflect(
        &self,
        _workflow: &WorkflowGenome,
        _metrics: &ExecutionMetrics,
    ) -> anyhow::Result<String> {
        Ok("Dummy reflection: workflow executed successfully.".to_string())
    }

    async fn evolve_code(&self, _feedback: &str, _genome: &str) -> anyhow::Result<String> {
        Ok("- improve node ordering\n- add error handling".to_string())
    }
}

#[tokio::test]
async fn test_evolution_cycle() {
    let mut engine = HyEvoEngine::new(DummyLlm);
    engine.seed(WorkflowGenome::new());
    engine.seed(WorkflowGenome::new());

    engine.evolution_cycle(2, 2);

    assert!(!engine.population.is_empty());
}

#[test]
fn test_best_workflow() {
    let mut engine = HyEvoEngine::new(DummyLlm);
    let mut g = WorkflowGenome::new();
    g.score = 0.9;
    engine.seed(g);

    assert!(engine.best_workflow().is_some());
}
