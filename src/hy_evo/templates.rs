//! Workflow Templates Library (Task 43)
//!
//! Pre-defined workflow templates for common agent patterns.

use crate::hy_evo::genome::WorkflowGenome;
use crate::hy_evo::node::Node;
use uuid::Uuid;

pub struct WorkflowTemplates;

impl WorkflowTemplates {
    /// Simple chat + memory workflow
    pub fn chat_with_memory() -> WorkflowGenome {
        WorkflowGenome {
            id: Uuid::new_v4(),
            nodes: vec![
                Node::Memory { key: "context".into(), op: "read".into() },
                Node::Llm { prompt: "Respond to user using memory context".into() },
                Node::Memory { key: "context".into(), op: "write".into() },
            ],
            edges: vec![(0, 1), (1, 2)],
            score: 0.0,
            metadata: Default::default(),
        }
    }

    /// Skill execution workflow
    pub fn skill_execution(skill_name: &str) -> WorkflowGenome {
        WorkflowGenome {
            id: Uuid::new_v4(),
            nodes: vec![
                Node::Skill { name: skill_name.into(), args: serde_json::Value::Null },
            ],
            edges: vec![],
            score: 0.0,
            metadata: Default::default(),
        }
    }

    /// Reflection + improvement workflow
    pub fn reflection_cycle() -> WorkflowGenome {
        WorkflowGenome {
            id: Uuid::new_v4(),
            nodes: vec![
                Node::Llm { prompt: "Analyze last execution and suggest improvements".into() },
                Node::Reflection { target: "last_workflow".into() },
            ],
            edges: vec![(0, 1)],
            score: 0.0,
            metadata: Default::default(),
        }
    }
}
