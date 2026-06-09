//! Workflow Templates Library (Task 43)
//!
//! Pre-defined workflow templates for common agent patterns.

use crate::hy_evo::genome::WorkflowGenome;
use crate::hy_evo::node::{Node, NodeMetadata};
use std::collections::HashMap;

pub struct WorkflowTemplates;

impl WorkflowTemplates {
    /// Simple chat + memory workflow
    pub fn chat_with_memory() -> WorkflowGenome {
        let now = chrono::Utc::now().timestamp_millis() as u64;

        let mut genome = WorkflowGenome::new();

        let mem_read = NodeMetadata::new("Read context from memory");
        genome.add_node(mem_read, Node::MemoryRead { key: "context".to_string() });

        let llm_node = NodeMetadata::new("LLM response");
        genome.add_node(llm_node, Node::Llm {
            model: "ollama".to_string(),
            prompt: "Respond to user using memory context".to_string(),
            params: HashMap::new(),
        });

        let mem_write = NodeMetadata::new("Write context to memory");
        genome.add_node(mem_write, Node::MemoryWrite {
            key: "context".to_string(),
            value: serde_json::json!({"updated": true}),
        });

        genome.add_edge(0, 1);
        genome.add_edge(1, 2);

        genome
    }

    /// Skill execution workflow
    pub fn skill_execution(skill_name: &str) -> WorkflowGenome {
        let mut genome = WorkflowGenome::new();

        let skill_node = NodeMetadata::new(format!("Execute skill: {}", skill_name));
        genome.add_node(skill_node, Node::Skill {
            name: skill_name.to_string(),
            args: HashMap::new(),
        });

        genome
    }

    /// Reflection + improvement workflow
    pub fn reflection_cycle() -> WorkflowGenome {
        let mut genome = WorkflowGenome::new();

        let llm_node = NodeMetadata::new("LLM analysis");
        genome.add_node(llm_node, Node::Llm {
            model: "ollama".to_string(),
            prompt: "Analyze last execution and suggest improvements".to_string(),
            params: HashMap::new(),
        });

        let reflection_node = NodeMetadata::new("Reflection step");
        genome.add_node(reflection_node, Node::Reflection {
            target: "last_workflow".to_string(),
            prompt: "Reflect on the previous step".to_string(),
        });

        genome.add_edge(0, 1);

        genome
    }
}
