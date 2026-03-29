use crate::hy_evo::genome::WorkflowGenome;
use crate::hy_evo::node::Node;
use crate::hy_evo::reflection::{ReflectionLlm, ReflectionRecord};
use crate::hy_evo::scoring::ExecutionMetrics;

/// The HyEvo evolution engine.
/// Maintains a population of workflow genomes and evolves them using LLM feedback.
pub struct HyEvoEngine<L: ReflectionLlm + Send + Sync> {
    pub llm: L,
    pub population: Vec<WorkflowGenome>,
}

impl<L: ReflectionLlm + Send + Sync> HyEvoEngine<L> {
    /// Create a new engine with the given LLM.
    pub fn new(llm: L) -> Self {
        Self {
            llm,
            population: Vec::new(),
        }
    }

    /// Seed the population with an initial workflow genome.
    pub fn seed(&mut self, genome: WorkflowGenome) {
        self.population.push(genome);
    }

    /// Run a single evolution cycle.
    /// Uses the LLM to generate improvement feedback and records a reflection.
    pub async fn evolve_once(
        &mut self,
        metrics: &ExecutionMetrics,
    ) -> anyhow::Result<ReflectionRecord> {
        let genome = match self.population.first().cloned() {
            Some(g) => g,
            None => anyhow::bail!("No workflows in population to evolve"),
        };

        let feedback = format!(
            "Evolve workflow based on execution metrics:\n\
             - Latency: {}ms\n\
             - LLM calls: {}\n\
             - Errors: {}\n\
             - Success: {}",
            metrics.latency_ms, metrics.llm_calls, metrics.errors, metrics.success
        );
        let genome_repr = format!("Workflow ID: {} | Nodes: {}", genome.id, genome.nodes.len());

        let summary = self.llm.evolve_code(&feedback, &genome_repr).await?;

        let suggestions = summary
            .lines()
            .filter(|l| l.trim().starts_with("- "))
            .map(|l| l.trim().trim_start_matches("- ").to_string())
            .collect();

        let record = ReflectionRecord {
            workflow_id: genome.id,
            summary,
            suggestions,
            metrics: metrics.clone(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
        };

        Ok(record)
    }

    /// Return the highest-scoring genome in the population.
    pub fn best_workflow(&self) -> Option<&WorkflowGenome> {
        self.population.iter().max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}
