// src/memory/mod.rs
pub mod episodic;
pub mod manager;
pub mod vector;

pub use manager::MemoryManager; // ← EXPORT IT

use serde_json::Value;
use std::collections::VecDeque;

use crate::cpu::interfaces::MemoryInterface;
use crate::hy_evo::node::NodeResult;
use crate::memory::episodic::EpisodicMemory;
use crate::memory::vector::VectorMemory;

/// Short-term working memory
pub struct MemoryHandle {
    pub context: VecDeque<String>,
    pub max_len: usize,
}

impl MemoryHandle {
    /// Drain the oldest N messages and return them as a single string.
    pub fn drain_oldest_chunk(&mut self, n: usize) -> Option<String> {
        if self.context.is_empty() {
            return None;
        }

        let count = n.min(self.context.len());
        let chunk: Vec<String> = self.context.drain(0..count).collect();
        Some(chunk.join(" "))
    }

    /// Insert a summary back into memory.
    pub fn push_summary(&mut self, summary: String) {
        self.context.push_front(format!("Summary: {}", summary));
    }

    pub fn update_belief(&mut self, key: &str, value: Value) -> NodeResult {
        self.write(key, value)
    }
}

impl MemoryInterface for MemoryHandle {
    fn read(&mut self, key: &str) -> NodeResult {
        match key {
            "context" => {
                let joined = self
                    .context
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                NodeResult::Value(Value::String(joined))
            }
            _ => NodeResult::Error(format!("Unknown memory key: {}", key)),
        }
    }

    fn write(&mut self, key: &str, value: Value) -> NodeResult {
        match key {
            "context" => {
                if let Some(s) = value.as_str() {
                    self.context.push_back(s.to_string());

                    while self.context.len() > self.max_len {
                        self.context.pop_front();
                    }

                    NodeResult::None
                } else {
                    NodeResult::Error("Expected string for context".into())
                }
            }
            _ => NodeResult::Error(format!("Unknown memory key: {}", key)),
        }
    }
}
