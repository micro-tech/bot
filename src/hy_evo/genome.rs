use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::node::{Node, NodeMetadata};
use super::mutation::{mutate_add_node, mutate_remove_node, mutate_reorder_nodes, mutate_node_params};

/// A WorkflowGenome represents the *evolvable* structure of a workflow.
/// It is the core unit that HyEvo mutates, crosses, scores, and executes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGenome {
    /// Unique ID for this workflow
    pub id: Uuid,

    /// Ordered list of nodes (skills, LLM calls, memory ops, etc.)
    pub nodes: Vec<(NodeMetadata, Node)>,

    /// Directed edges between nodes (node index → node index)
    pub edges: Vec<(usize, usize)>,

    /// Arbitrary metadata (e.g., tags, categories, notes)
    pub metadata: HashMap<String, serde_json::Value>,

    /// Fitness score assigned by the scoring engine
    pub score: f64,

    /// Timestamp (ms) when this workflow was created
    pub created_at: u64,

    /// Timestamp (ms) when this workflow was last modified
    pub last_modified: u64,
}

impl WorkflowGenome {
    /// Create a new empty genome
    pub fn new() -> Self {
        let now = chrono::Utc::now().timestamp_millis() as u64;

        Self {
            id: Uuid::new_v4(),
            nodes: Vec::new(),
            edges: Vec::new(),
            metadata: HashMap::new(),
            score: 0.0,
            created_at: now,
            last_modified: now,
        }
    }

    /// Add a node to the genome and return its index
    pub fn add_node(&mut self, metadata: NodeMetadata, node: Node) -> usize {
        let index = self.nodes.len();
        self.nodes.push((metadata, node));
        self.touch();
        index
    }

    /// Add a directed edge between two nodes
    pub fn add_edge(&mut self, from: usize, to: usize) {
        self.edges.push((from, to));
        self.touch();
    }

    /// Update the last_modified timestamp
    pub fn touch(&mut self) {
        self.last_modified = chrono::Utc::now().timestamp_millis() as u64;
    }

    /// Set a metadata field
    pub fn set_meta(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.metadata.insert(key.into(), value);
        self.touch();
    }

    /// Get a metadata field
    pub fn get_meta(&self, key: &str) -> Option<&serde_json::Value> {
        self.metadata.get(key)
    }

    /// Clear score before re-evaluation
    pub fn reset_score(&mut self) {
        self.score = 0.0;
    }

    /// Apply a random mutation
    pub fn mutate(&mut self) {
        let mutation_type = (chrono::Utc::now().timestamp_nanos() % 4) as u32; // simple random
        match mutation_type {
            0 => mutate_add_node(self),
            1 => mutate_remove_node(self),
            2 => mutate_reorder_nodes(self),
            3 => mutate_node_params(self),
            _ => {}
        }
        self.touch();
    }
}