use rand::prelude::*;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use super::genome::WorkflowGenome;
use super::node::{Node, NodeMetadata};

/// Mutation configuration (can be loaded from config.toml later)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationConfig {
    pub add_node_prob: f32,
    pub remove_node_prob: f32,
    pub reorder_prob: f32,
    pub param_mutation_prob: f32,
    pub edge_mutation_prob: f32,
}

impl Default for MutationConfig {
    fn default() -> Self {
        Self {
            add_node_prob: 0.10,
            remove_node_prob: 0.05,
            reorder_prob: 0.20,
            param_mutation_prob: 0.30,
            edge_mutation_prob: 0.10,
        }
    }
}

/// Main mutation engine
pub struct MutationEngine {
    pub config: MutationConfig,
}

impl MutationEngine {
    pub fn new(config: MutationConfig) -> Self {
        Self { config }
    }

    /// Apply all mutation types to a genome
    pub fn mutate(&self, genome: &mut WorkflowGenome) {
        let mut rng = rand::rng();

        if rng.random::<f32>() < self.config.add_node_prob {
            mutate_add_node(genome);
        }

        if rng.random::<f32>() < self.config.remove_node_prob {
            mutate_remove_node(genome);
        }

        if rng.random::<f32>() < self.config.reorder_prob {
            mutate_reorder_nodes(genome);
        }

        if rng.random::<f32>() < self.config.param_mutation_prob {
            mutate_node_params(genome);
        }

        if rng.random::<f32>() < self.config.edge_mutation_prob {
            self.mutate_edges(genome);
        }

        genome.touch();
    }

    // -------------------------
    // EDGE MUTATIONS
    // -------------------------

    /// Randomly mutate edges (add/remove)
    fn mutate_edges(&self, genome: &mut WorkflowGenome) {
        let mut rng = rand::rng();

        if genome.nodes.len() < 2 {
            return;
        }

        // 50% chance: add edge
        if rng.random_bool(0.5) {
            let from = rng.random_range(0..genome.nodes.len());
            let to = rng.random_range(0..genome.nodes.len());

            if from != to {
                genome.edges.push((from, to));
            }
        } else {
            // 50% chance: remove edge
            if !genome.edges.is_empty() {
                let idx = rng.random_range(0..genome.edges.len());
                genome.edges.remove(idx);
            }
        }
    }
}

/// Add a random node to the workflow
pub fn mutate_add_node(genome: &mut WorkflowGenome) {
    let mut rng = rand::rng();

    let new_node = Node::Skill {
        name: "noop".to_string(),
        args: Default::default(),
    };

    let metadata = NodeMetadata::new("auto-added mutation node");

    let index = genome.add_node(metadata, new_node);

    // Add a random edge from an existing node to the new node
    if index > 0 {
        let from = rng.random_range(0..index);
        genome.add_edge(from, index);
    }
}

/// Remove a random node (if possible)
pub fn mutate_remove_node(genome: &mut WorkflowGenome) {
    if genome.nodes.len() <= 1 {
        return;
    }

    let mut rng = rand::rng();
    let idx = rng.random_range(0..genome.nodes.len());

    genome.nodes.remove(idx);

    // Remove edges referencing this node
    genome.edges.retain(|(from, to)| *from != idx && *to != idx);

    // Reindex edges
    for (_from, to) in &mut genome.edges {
        if *_from > idx {
            *_from -= 1;
        }
        if *to > idx {
            *to -= 1;
        }
    }
}

/// Randomly reorder nodes
pub fn mutate_reorder_nodes(genome: &mut WorkflowGenome) {
    let mut rng = rand::rng();
    genome.nodes.shuffle(&mut rng);
}

/// Mutate parameters of a random node
pub fn mutate_node_params(genome: &mut WorkflowGenome) {
    let mut rng = rand::rng();

    if genome.nodes.is_empty() {
        return;
    }

    let idx = rng.random_range(0..genome.nodes.len());
    let (metadata, node) = &mut genome.nodes[idx];

    metadata.last_modified = chrono::Utc::now().timestamp_millis() as u64;

    match node {
        Node::Skill { args, .. } | Node::Llm { params: args, .. } | Node::Code { params: args, .. } => {
            args.insert(
                "mutated".to_string(),
                serde_json::json!(rng.random_range(0..1000u32)),
            );
        }

        Node::MemoryWrite { value, .. } => {
            *value = serde_json::Value::Number(rng.random_range(0..1000u32).into());
        }

        _ => {}
    }
}
