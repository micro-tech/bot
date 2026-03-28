use rand::prelude::*;
use uuid::Uuid;

use super::{
    crossover::{CrossoverConfig, CrossoverEngine},
    genome::WorkflowGenome,
    mutation::{MutationConfig, MutationEngine},
    reflection::{ReflectionEngine, ReflectionLlm, ReflectionRecord},
    scoring::{ExecutionMetrics, ScoringEngine, ScoringWeights},
};

/// Represents a population of workflows.
/// HyEvo evolves this population over time.
pub struct Population {
    pub workflows: Vec<WorkflowGenome>,
    pub max_size: usize,
}

impl Population {
    pub fn new(max_size: usize) -> Self {
        Self {
            workflows: Vec::new(),
            max_size,
        }
    }

    /// Add a workflow to the population.
    /// If full, replace the worst-scoring workflow.
    pub fn add(&mut self, genome: WorkflowGenome) {
        if self.workflows.len() < self.max_size {
            self.workflows.push(genome);
        } else {
            // Replace worst-scoring workflow
            if let Some((idx, _)) = self
                .workflows
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.score.total_cmp(&b.score))
            {
                self.workflows[idx] = genome;
            }
        }
    }

    /// Select a workflow using tournament selection.
    pub fn select_parent(&self) -> Option<&WorkflowGenome> {
        if self.workflows.is_empty() {
            return None;
        }

        let mut rng = thread_rng();
        let a = rng.gen_range(0..self.workflows.len());
        let b = rng.gen_range(0..self.workflows.len());

        Some(if self.workflows[a].score > self.workflows[b].score {
            &self.workflows[a]
        } else {
            &self.workflows[b]
        })
    }

    /// Get the best workflow in the population.
    pub fn best(&self) -> Option<&WorkflowGenome> {
        self.workflows
            .iter()
            .max_by(|a, b| a.score.total_cmp(&b.score))
    }
}

/// Main HyEvo engine
pub struct HyEvoEngine<L: ReflectionLlm + Send + Sync> {
    pub population: Population,
    pub mutation: MutationEngine,
    pub crossover: CrossoverEngine,
    pub scoring: ScoringEngine,
    pub reflection: ReflectionEngine<L>,
}

impl<L: ReflectionLlm + Send + Sync> HyEvoEngine<L> {
    pub fn new(
        population_size: usize,
        mutation_cfg: MutationConfig,
        crossover_cfg: CrossoverConfig,
        scoring_weights: ScoringWeights,
        llm: L,
    ) -> Self {
        Self {
            population: Population::new(population_size),
            mutation: MutationEngine::new(mutation_cfg),
            crossover: CrossoverEngine::new(crossover_cfg),
            scoring: ScoringEngine::new(scoring_weights),
            reflection: ReflectionEngine::new(llm),
        }
    }

    /// Add an initial workflow to the population.
    pub fn seed(&mut self, genome: WorkflowGenome) {
        self.population.add(genome);
    }

    /// Run a full evolution cycle:
    /// 1. Select parents
    /// 2. Crossover
    /// 3. Mutation
    /// 4. Score
    /// 5. Reflect
    /// 6. Insert into population
    pub async fn evolve_once(
        &mut self,
        metrics: &ExecutionMetrics,
    ) -> anyhow::Result<ReflectionRecord> {
        // Need at least 2 parents
        let parent_a = self.population.select_parent().cloned();
        let parent_b = self.population.select_parent().cloned();

        let (parent_a, parent_b) = match (parent_a, parent_b) {
            (Some(a), Some(b)) => (a, b),
            _ => anyhow::bail!("Not enough workflows in population"),
        };

        // 1. Crossover
        let mut child = self.crossover.crossover(&parent_a, &parent_b);

        // 2. Mutation
        self.mutation.mutate(&mut child);

        // 3. Score
        self.scoring.score(&mut child, metrics);

        // 4. Reflect
        let reflection = self.reflection.reflect(&child, metrics).await?;

        // 5. Insert into population
        self.population.add(child);

        Ok(reflection)
    }

    /// Get the best workflow currently available.
    pub fn best_workflow(&self) -> Option<&WorkflowGenome> {
        self.population.best()
    }
}
