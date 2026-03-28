use std::sync::{Arc, Mutex};

use super::{
    engine::HyEvoEngine,
    genome::WorkflowGenome,
    reflection::{ReflectionLlm, ReflectionRecord},
    scoring::ExecutionMetrics,
    workflow::Workflow,
};

/// Interface your CPU must implement to run workflows.
/// This keeps HyEvo decoupled from your agent OS internals.
#[async_trait::async_trait]
pub trait CpuExecutor {
    async fn execute_workflow(&self, workflow: &Workflow) -> anyhow::Result<ExecutionMetrics>;
}

/// HyEvo integration layer.
/// This is what your CPU will hold onto.
pub struct HyEvoIntegration<L: ReflectionLlm + Send + Sync> {
    pub engine: Arc<Mutex<HyEvoEngine<L>>>,
}

impl<L: ReflectionLlm + Send + Sync> HyEvoIntegration<L> {
    pub fn new(engine: HyEvoEngine<L>) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
        }
    }

    /// Get the best workflow currently in the population.
    pub fn get_best_workflow(&self) -> Option<WorkflowGenome> {
        let engine = self.engine.lock().unwrap();
        engine.best_workflow().cloned()
    }

    /// Seed the population with an initial workflow genome.
    pub fn seed(&self, genome: WorkflowGenome) {
        let mut engine = self.engine.lock().unwrap();
        engine.seed(genome);
    }

    /// Run a single evolution cycle given execution metrics.
    pub async fn evolve(&self, metrics: ExecutionMetrics) -> anyhow::Result<()> {
        let mut engine = self.engine.lock().unwrap();
        engine.evolve_once(&metrics).await?;
        Ok(())
    }

    /// Select the best genome from the current population.
    pub fn select_best(&self) -> Option<WorkflowGenome> {
        let engine = self.engine.lock().unwrap();
        engine.best_workflow().cloned()
    }

    /// Run a workflow through the CPU and evolve based on results.
    pub async fn run_and_evolve<E: CpuExecutor>(&self, cpu: &E) -> anyhow::Result<()> {
        // 1. Select best workflow
        let genome = {
            let engine = self.engine.lock().unwrap();
            engine.best_workflow().cloned()
        };

        let genome = match genome {
            Some(g) => g,
            None => anyhow::bail!("No workflows available in HyEvo population"),
        };

        // 2. Convert genome → executable workflow
        let workflow = Workflow::from_genome(genome.clone())?;

        // 3. Execute workflow via CPU
        let metrics = cpu.execute_workflow(&workflow).await?;

        // 4. Evolve based on execution results
        let reflection = {
            let mut engine = self.engine.lock().unwrap();
            engine.evolve_once(&metrics).await?
        };

        // 5. (Optional) Store reflection somewhere
        self.store_reflection(reflection)?;

        Ok(())
    }

    /// Store reflection records (you can extend this later)
    fn store_reflection(&self, reflection: ReflectionRecord) -> anyhow::Result<()> {
        println!("[HyEvo Reflection] {}", reflection.summary);
        Ok(())
    }
}
