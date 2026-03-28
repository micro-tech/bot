use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Represents a single executable unit in a workflow.
/// HyEvo evolves sequences of these nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Node {
    /// Call a skill by name (your skills/ folder)
    Skill {
        name: String,
        params: HashMap<String, serde_json::Value>,
    },

    /// Call an LLM (Ollama, Groq, etc.)
    Llm {
        model: String,
        prompt_template: String,
        params: HashMap<String, serde_json::Value>,
    },

    /// Execute a deterministic Rust function
    Code {
        function: String, // name of registered function
        params: HashMap<String, serde_json::Value>,
    },

    /// Read from memory
    MemoryRead { key: String },

    /// Write to memory
    MemoryWrite {
        key: String,
        value: serde_json::Value,
    },

    /// Publish a message to the bus
    BusPublish { to: String, data: serde_json::Value },

    /// Conditional branching
    Conditional {
        condition: ConditionNode,
        then_branch: Vec<Uuid>, // node IDs
        else_branch: Vec<Uuid>,
    },
}

/// Represents a condition used in Conditional nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionNode {
    /// Compare a memory value
    MemoryEquals {
        key: String,
        value: serde_json::Value,
    },

    /// Check if last LLM output contains text
    OutputContains { text: String },

    /// Custom condition (future‑proof)
    Custom {
        name: String,
        params: HashMap<String, serde_json::Value>,
    },
}

/// Result of executing a node.
/// The CPU will use this to pass data between nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeResult {
    /// Node produced a JSON value
    Value(serde_json::Value),

    /// Node produced a string (LLM output, logs, etc.)
    Text(String),

    /// Node produced no meaningful output
    None,

    /// Node failed (HyEvo uses this for scoring)
    Error(String),
}

/// Metadata for each node instance inside a workflow.
/// This allows nodes to be uniquely identified and evolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetadata {
    pub id: Uuid,
    pub description: String,
    pub created_at: u64,
    pub last_modified: u64,
}

impl NodeMetadata {
    pub fn new(description: impl Into<String>) -> Self {
        let now = chrono::Utc::now().timestamp_millis() as u64;
        Self {
            id: Uuid::new_v4(),
            description: description.into(),
            created_at: now,
            last_modified: now,
        }
    }
}
