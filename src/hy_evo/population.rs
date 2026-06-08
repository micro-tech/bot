//! Population Manager — maintains a pool of workflows for evolution.
use crate::hy_evo::genome::WorkflowGenome;

pub struct PopulationManager {
    pub population: Vec<WorkflowGenome>,
    pub max_size: usize,
}

impl PopulationManager {
    pub fn new(max_size: usize) -> Self {
        Self {
            population: Vec::new(),
            max_size,
        }
    }

    /// Add a genome, culling worst performers if over capacity.
    pub fn add(&mut self, genome: WorkflowGenome) {
        self.population.push(genome);
        if self.population.len() > self.max_size {
            self.cull_worst(1);
        }
    }

    /// Keep the best K genomes, remove the rest.
    pub fn keep_best(&mut self, k: usize) {
        self.population.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        self.population.truncate(k);
    }

    /// Remove the worst K genomes.
    pub fn cull_worst(&mut self, k: usize) {
        self.population.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
        let remove = k.min(self.population.len());
        self.population.drain(0..remove);
    }
}
