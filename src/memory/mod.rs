//! Memory Module - Short Term
use serde_json::Value;
use std::collections::VecDeque;

use crate::cpu::interfaces::MemoryInterface;
use crate::hy_evo::node::NodeResult;

/// A simple in-memory short-term memory buffer.
/// Wraps your existing SessionData model.
pub struct MemoryHandle {
    pub context: VecDeque<String>,
    pub max_len: usize,
}

impl MemoryHandle {
    pub fn new(max_len: usize) -> Self {
        Self {
            context: VecDeque::new(),
            max_len,
        }
    }

    /// Update a belief value in memory (alias for write with belief semantics).
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

                    // enforce max length
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
