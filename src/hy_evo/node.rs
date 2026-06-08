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
        args: HashMap<String, serde_json::Value>,
    },

    /// Call an LLM (Ollama, Groq, etc.)
    Llm {
        model: String,
        prompt: String,
        params: HashMap<String, serde_json::Value>,
    },

    /// Memory operations
    Memory {
        operation: String, // "read" or "write"
        key: String,
        value: Option<serde_json::Value>,
    },

    /// Reflection step
    Reflection {
        target: String,
        prompt: String,
    },

    /// Execute a deterministic Rust function
    Code {
        function: String,
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
        then_branch: Vec<Uuid>,
        else_branch: Vec<Uuid>,
    },
}

/// Represents a condition used in Conditional nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionNode {
    MemoryEquals {
        key: String,
        value: serde_json::Value,
    },
    OutputContains { text: String },
    Custom {
        name: String,
        params: HashMap<String, serde_json::Value>,
    },
}

/// Result of executing a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeResult {
    Value(serde_json::Value),
    Text(String),
    None,
    Error(String),
    Success, // Added for compatibility
}

/// Metadata for each node instance inside a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetadata {
    pub id: Uuid,
    pub description: String,
    pub created_at: u64,
    pub last_modified: u64,
    pub version: u32,
}

impl NodeMetadata {
    pub fn new(description: impl Into<String>) -> Self {
        let now = chrono::Utc::now().timestamp_millis() as u64;
        Self {
            id: Uuid::new_v4(),
            description: description.into(),
            created_at: now,
            last_modified: now,
            version: 1,
        }
    }
}
