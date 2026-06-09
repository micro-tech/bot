use async_trait::async_trait;
use serde_json::Value;

/// Minimal trait for Ollama-backed LLM implementations.
///
/// This trait focuses on the core capabilities needed for the agent:
/// - Simple text generation
/// - Chat-style completion (with message history)
/// - Tool-aware chat (for agentic loops)
#[async_trait]
pub trait OllamaLlm: Send + Sync {
    /// Generate a completion from a single prompt.
    async fn generate(&self, prompt: &str) -> anyhow::Result<String>;

    /// Chat completion with full message history.
    async fn chat(&self, messages: &[Value]) -> anyhow::Result<String>;

    /// Chat completion with tools enabled (agentic mode).
    async fn chat_with_tools(
        &self,
        messages: &[Value],
        tools: Value,
    ) -> anyhow::Result<String>;

    /// Return the model name this instance is configured with.
    fn model_name(&self) -> &str;
}
