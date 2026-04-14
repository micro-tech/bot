use rand::prelude::*;
use uuid::Uuid;

use super::genome::WorkflowGenome;
use super::node::{Node, NodeMetadata};

/// Crossover configuration (can be extended later)
#[derive(Debug, Clone)]
pub struct CrossoverConfig {
    pub single_point_prob: f32,
    pub two_point_prob: f32,
    pub uniform_prob: f32,
}

impl Default for CrossoverConfig {
    fn default() -> Self {
        Self {
            single_point_prob: 0.5,
            two_point_prob: 0.3,
            uniform_prob: 0.2,
        }
    }
}

/// Main crossover engine
pub struct CrossoverEngine {
    pub config: CrossoverConfig,
}

impl CrossoverEngine {
    pub fn new(config: CrossoverConfig) -> Self {
        Self { config }
    }

    /// Perform crossover between two parent genomes.
    /// Returns a new child genome.
    pub fn crossover(
        &self,
        parent_a: &WorkflowGenome,
        parent_b: &WorkflowGenome,
    ) -> WorkflowGenome {
        let mut rng = thread_rng();
        let roll = rng.r#gen::<f32>();

        if roll < self.config.single_point_prob {
            return self.single_point(parent_a, parent_b);
        }

        if roll < self.config.single_point_prob + self.config.two_point_prob {
            return self.two_point(parent_a, parent_b);
        }

        self.uniform(parent_a, parent_b)
    }

    // ---------------------------------------------------------
    // SINGLE-POINT CROSSOVER
    // ---------------------------------------------------------

    fn single_point(&self, a: &WorkflowGenome, b: &WorkflowGenome) -> WorkflowGenome {
        let mut rng = thread_rng();

        let split_a = rng.gen_range(0..a.nodes.len().max(1));
        let split_b = rng.gen_range(0..b.nodes.len().max(1));

        let mut child = WorkflowGenome::new();

        // Take prefix from A
        for i in 0..split_a {
            child.nodes.push(a.nodes[i].clone());
        }

        // Take suffix from B
        for i in split_b..b.nodes.len() {
            child.nodes.push(b.nodes[i].clone());
        }

        self.rebuild_edges(&mut child);

        child
    }

    // ---------------------------------------------------------
    // TWO-POINT CROSSOVER
    // ---------------------------------------------------------

    fn two_point(&self, a: &WorkflowGenome, b: &WorkflowGenome) -> WorkflowGenome {
        let mut rng = thread_rng();

        if a.nodes.is_empty() || b.nodes.is_empty() {
            return a.clone();
        }

        let (start_a, end_a) = random_range_pair(a.nodes.len(), &mut rng);
        let (start_b, end_b) = random_range_pair(b.nodes.len(), &mut rng);

        let mut child = WorkflowGenome::new();

        // Prefix from A
        for i in 0..start_a {
            child.nodes.push(a.nodes[i].clone());
        }

        // Middle from B
        for i in start_b..end_b {
            child.nodes.push(b.nodes[i].clone());
        }

        // Suffix from A
        for i in end_a..a.nodes.len() {
            child.nodes.push(a.nodes[i].clone());
        }

        self.rebuild_edges(&mut child);

        child
    }

    // ---------------------------------------------------------
    // UNIFORM CROSSOVER
    // ---------------------------------------------------------

    fn uniform(&self, a: &WorkflowGenome, b: &WorkflowGenome) -> WorkflowGenome {
        let mut rng = thread_rng();
        let mut child = WorkflowGenome::new();

        let max_len = a.nodes.len().max(b.nodes.len());

        for i in 0..max_len {
            if rng.gen_bool(0.5) {
                if let Some(node) = a.nodes.get(i) {
                    child.nodes.push(node.clone());
                }
            } else {
                if let Some(node) = b.nodes.get(i) {
                    child.nodes.push(node.clone());
                }
            }
        }

        self.rebuild_edges(&mut child);

        child
    }

    // ---------------------------------------------------------
    // EDGE RECONSTRUCTION
    // ---------------------------------------------------------

    /// Rebuild edges after crossover.
    /// Ensures edges reference valid node indices.
    fn rebuild_edges(&self, genome: &mut WorkflowGenome) {
        genome.edges.clear();

        // Simple heuristic: connect nodes linearly
        for i in 0..genome.nodes.len().saturating_sub(1) {
            genome.edges.push((i, i + 1));
        }

        genome.touch();
    }
}

/// Generate a random (start, end) pair where start < end
fn random_range_pair(len: usize, rng: &mut rand::rngs::ThreadRng) -> (usize, usize) {
    if len < 2 {
        return (0, len);
    }

    let a = rng.gen_range(0..len);
    let b = rng.gen_range(0..len);

    if a < b { (a, b) } else { (b, a) }
}
