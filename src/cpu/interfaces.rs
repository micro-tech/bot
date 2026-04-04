use async_trait::async_trait;
use serde_json::Value;

use crate::hy_evo::node::NodeResult;

/// Memory subsystem interface
/// 
/// Suggestions for implementations:
/// - Add logging at the start and end of methods (e.g., using log::info! for entry/exit).
/// - Handle errors explicitly in implementations, logging any failures before returning Err.
#[async_trait]
pub trait MemoryInterface: Send + Sync {
    /// Read data from memory. Implementations should log the key being read and any errors.
    fn read(&mut self, key: &str) -> NodeResult;
    /// Write data to memory. Implementations should log the key and value, and log any write errors.
    fn write(&mut self, key: &str, value: Value) -> NodeResult;
}

/// Skills subsystem interface
/// 
/// Suggestions for implementations:
/// - Use logging to record skill calls, including parameters, for debugging.
/// - In async methods, log errors if the call fails.
#[async_trait]
pub trait SkillInterface: Sync + Send {
    /// Call a skill asynchronously. Implementations should log the skill name and parameters.
    async fn call(&self, name: &str, params: &Value) -> NodeResult;
}

/// LLM subsystem interface (Ollama, Groq, etc.)
/// 
/// Suggestions for implementations:
/// - Log prompts and responses to track LLM interactions.
/// - Add error logging for network or API failures.
#[async_trait]
pub trait LlmInterface: Sync + Send {
    /// Call an LLM model asynchronously. Implementations should log the model, prompt, and any errors.
    async fn call(&self, model: &str, prompt: &str, params: &Value) -> NodeResult;
    /// Summarize text asynchronously. Implementations should log the input and any errors.
    async fn summarize(&self, text: &str) -> NodeResult;
}

/// Bus subsystem interface
/// 
/// Suggestions for implementations:
/// - Log messages being published, including destinations and data.
/// - Handle and log any publishing errors, such as connection issues.
#[async_trait]
pub trait BusInterface: Sync + Send {
    /// Publish a message asynchronously. Implementations should log the 'to' field and payload.
    async fn publish(&self, to: &str, data: Value) -> NodeResult;
}
