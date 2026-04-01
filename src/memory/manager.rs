use crate::cpu::interfaces::MemoryInterface;
use crate::memory::MemoryHandle;
use crate::memory::episodic::EpisodicMemory;
use crate::memory::vector::VectorMemory;
use serde_json::Value;

pub struct MemoryManager {
    pub working: MemoryHandle,
    pub vector: VectorMemory,
    pub episodic: EpisodicMemory,
}

impl MemoryManager {
    pub fn new(working_max: usize, episodic_max: usize) -> Self {
        Self {
            working: MemoryHandle::new(working_max),
            vector: VectorMemory::new(),
            episodic: EpisodicMemory::new(episodic_max),
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
}
