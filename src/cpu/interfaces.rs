use async_trait::async_trait;
use serde_json::Value;

use crate::hy_evo::node::NodeResult;

/// Memory subsystem interface
#[async_trait]
pub trait MemoryInterface: Send {
    fn read(&mut self, key: &str) -> NodeResult;
    fn write(&mut self, key: &str, value: Value) -> NodeResult;
}

/// Skills subsystem interface
#[async_trait]
pub trait SkillInterface: Sync {
    async fn call(&self, name: &str, params: &Value) -> NodeResult;
}

/// LLM subsystem interface (Ollama, Groq, etc.)
#[async_trait]
pub trait LlmInterface: Sync {
    async fn call(&self, model: &str, prompt: &str, params: &Value) -> NodeResult;
}

/// Bus subsystem interface
#[async_trait]
pub trait BusInterface: Sync {
    async fn publish(&self, to: &str, data: Value) -> NodeResult;
}
