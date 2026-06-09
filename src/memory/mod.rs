// src/memory/mod.rs
pub mod episodic;
pub mod manager;
pub mod vector;

pub use manager::MemoryManager; // ← EXPORT IT

use serde_json::Value;
use std::collections::VecDeque;

use crate::cpu::interfaces::MemoryInterface;
use crate::hy_evo::node::NodeResult;

/// Short-term working memory
pub struct MemoryHandle {
    pub context: VecDeque<String>,
    pub max_len: usize,
}

impl MemoryHandle {
    /// Create a new working-memory handle with the given capacity.
    pub fn new(max_len: usize) -> Self {
        Self {
            context: VecDeque::new(),
            max_len,
        }
    }

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

    /// Get recent entries (most recent first)
    pub fn get_recent_entries(&self, n: usize) -> Option<Vec<String>> {
        if self.context.is_empty() {
            return None;
        }
        let recent: Vec<String> = self.context.iter().rev().take(n).cloned().collect();
        Some(recent)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let handle = MemoryHandle::new(10);
        assert_eq!(handle.max_len, 10);
        assert!(handle.context.is_empty());
    }

    #[test]
    fn test_write_and_read_context() {
        let mut handle = MemoryHandle::new(5);
        let result = handle.write("context", Value::String("hello".to_string()));
        assert!(matches!(result, NodeResult::None));
        assert_eq!(handle.context.len(), 1);
        assert_eq!(handle.context[0], "hello");

        let read_result = handle.read("context");
        if let NodeResult::Value(Value::String(s)) = read_result {
            assert_eq!(s, "hello");
        } else {
            panic!("Expected string value");
        }
    }

    #[test]
    fn test_write_max_len() {
        let mut handle = MemoryHandle::new(2);
        handle.write("context", Value::String("msg1".to_string()));
        handle.write("context", Value::String("msg2".to_string()));
        handle.write("context", Value::String("msg3".to_string()));
        assert_eq!(handle.context.len(), 2);
        assert_eq!(handle.context[0], "msg2");
        assert_eq!(handle.context[1], "msg3");
    }

    #[test]
    fn test_drain_oldest_chunk() {
        let mut handle = MemoryHandle::new(5);
        handle.write("context", Value::String("msg1".to_string()));
        handle.write("context", Value::String("msg2".to_string()));
        let chunk = handle.drain_oldest_chunk(1).unwrap();
        assert_eq!(chunk, "msg1");
        assert_eq!(handle.context.len(), 1);
        assert_eq!(handle.context[0], "msg2");
    }

    #[test]
    fn test_push_summary() {
        let mut handle = MemoryHandle::new(5);
        handle.push_summary("test summary".to_string());
        assert_eq!(handle.context.len(), 1);
        assert_eq!(handle.context[0], "Summary: test summary");
    }

    #[test]
    fn test_read_unknown_key() {
        let mut handle = MemoryHandle::new(5);
        let result = handle.read("unknown");
        assert!(matches!(result, NodeResult::Error(_)));
    }

    #[test]
    fn test_write_unknown_key() {
        let mut handle = MemoryHandle::new(5);
        let result = handle.write("unknown", Value::String("test".to_string()));
        assert!(matches!(result, NodeResult::Error(_)));
    }

    #[test]
    fn test_write_non_string_context() {
        let mut handle = MemoryHandle::new(5);
        let result = handle.write("context", Value::Number(serde_json::Number::from(42)));
        assert!(matches!(result, NodeResult::Error(_)));
    }
}
