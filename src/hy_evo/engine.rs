use crate::hy_evo::genome::WorkflowGenome;
use crate::hy_evo::node::Node;
use crate::hy_evo::reflection::{ReflectionLlm, ReflectionRecord};
use crate::hy_evo::scoring::ExecutionMetrics;
use rand::prelude::*;

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

        let suggestions: Vec<String> = summary
            .lines()
            .filter(|l| l.trim().starts_with("- "))
            .map(|l| l.trim().trim_start_matches("- ").to_string())
            .collect();

        let record = ReflectionRecord {
            workflow_id: genome.id,
            summary: summary.clone(),
            suggestions: suggestions.clone(),
            metrics: metrics.clone(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
        };

        // Attach reflection to workflow metadata
        if let Some(g) = self.population.first_mut() {
            g.metadata.insert("last_reflection".to_string(), serde_json::Value::String(summary));
            g.metadata.insert("reflection_suggestions".to_string(), serde_json::json!(suggestions));
        }

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

    /// Calculate the average score across the population.
    pub fn average_score(&self) -> f64 {
        if self.population.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.population.iter().map(|g| g.score).sum();
        sum / self.population.len() as f64
    }

    /// Run a full evolution cycle on the population.
    ///
    /// Steps:
    /// 1. Select top N workflows
    /// 2. Mutate + crossover to produce offspring
    /// 3. Score new genomes (placeholder scoring for now)
    /// 4. Replace worst performers
    pub fn evolution_cycle(&mut self, top_n: usize, offspring_count: usize) {
        if self.population.len() < 2 {
            return;
        }

        // 1. Select top N
        let mut sorted: Vec<_> = self.population.iter().cloned().collect();
        sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        let parents: Vec<_> = sorted.into_iter().take(top_n).collect();

        // 2. Generate offspring via mutation + crossover
        let mut offspring = Vec::new();
        let mut rng = rand::rng();

        for _ in 0..offspring_count {
            if parents.len() >= 2 {
                let a_idx = rng.random_range(0..parents.len());
                let b_idx = rng.random_range(0..parents.len());
                let a = &parents[a_idx];
                let b = &parents[b_idx];

                // Crossover
                let mut child = crate::hy_evo::crossover::CrossoverEngine::new(Default::default())
                    .crossover(a, b);

                // Mutate
                crate::hy_evo::mutation::MutationEngine::new(Default::default()).mutate(&mut child);

                offspring.push(child);
            }
        }

        // 3. Simple scoring (can be replaced with real ExecutionMetrics later)
        for genome in &mut offspring {
            genome.score = genome.nodes.len() as f64 * 0.1; // placeholder
        }

        // 4. Replace worst performers
        self.population.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));

        let replace_count = offspring.len().min(self.population.len());
        for i in 0..replace_count {
            self.population[i] = offspring[i].clone();
        }
    }
}
