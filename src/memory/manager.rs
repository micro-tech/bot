use crate::cpu::interfaces::MemoryInterface;
use crate::memory::MemoryHandle;
use crate::memory::NodeResult;
use crate::memory::episodic::EpisodicMemory;
use crate::memory::vector::VectorMemory;
use serde_json::Value;
use std::collections::HashMap;

pub struct MemoryManager {
    pub working: MemoryHandle,
    pub vector: VectorMemory,
    pub episodic: EpisodicMemory,
    pub beliefs: HashMap<String, Value>,
}

impl MemoryManager {
    pub fn new(working_max: usize, episodic_max: usize) -> Self {
        Self {
            working: MemoryHandle::new(working_max),
            vector: VectorMemory::new(),
            episodic: EpisodicMemory::new(episodic_max),
            beliefs: HashMap::new(),
        }
    }

    pub fn record_user_message(&mut self, text: &str) {
        let _ = self
            .working
            .write("context", Value::String(text.to_string()));

        self.episodic.record(format!("user_msg: {}", text));
    }

    pub fn search_facts(&self, query: &str, k: usize) -> Vec<String> {
        self.vector.search(query, k)
    }

    pub fn set_belief(&mut self, key: &str, value: Value) {
        self.beliefs.insert(key.to_string(), value);
    }

    pub fn get_belief(&self, key: &str) -> Option<&Value> {
        self.beliefs.get(key)
    }

    pub fn get_all_beliefs(&self) -> &HashMap<String, Value> {
        &self.beliefs
    }

    /// Generate a Mermaid graph visualization of beliefs
    pub fn visualize_beliefs(&self) -> String {
        let mut mermaid = "graph TD;\n".to_string();
        for (key, value) in &self.beliefs {
            let val_str = value.to_string().replace("\"", "'");
            mermaid.push_str(&format!("    {}[\"{}: {}\"];\n", key.replace(" ", "_").replace("-", "_"), key, val_str));
        }
        mermaid
    }
}

impl MemoryInterface for MemoryManager {
    fn read(&mut self, key: &str) -> NodeResult {
        self.working.read(key)
    }

    fn write(&mut self, key: &str, value: Value) -> NodeResult {
        self.working.write(key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_manager_new() {
        let manager = MemoryManager::new(10, 20);
        // Basic test to ensure creation
        assert_eq!(manager.working.max_len, 10);
        // You can add more assertions based on the actual structure
    }
}
