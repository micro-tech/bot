//! AgentStep — the instruction set ("bytecode") executed by the Unified Agent Runtime Loop.
//!
//! Security & Safety Notes:
//! - All tool names are validated at runtime (no hallucinated tools).
//! - LLMCall and ToolCall carry correlation IDs for tracing.
//! - Error variant is used for controlled failure paths only.

use serde::{Deserialize, Serialize};

/// Specification for an LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMCallSpec {
    pub backend: String,          // "ollama", "gemini", "grok", etc.
    pub prompt: String,
    pub correlation_id: u64,
}

/// Tool invocation specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvocation {
    pub name: String,
    pub args: serde_json::Value,
    pub correlation_id: u64,
}

/// The core instruction set executed by the runtime loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentStep {
    /// Call an LLM backend.
    LLMCall(LLMCallSpec),

    /// Execute a tool.
    ToolCall(ToolInvocation),

    /// Final answer — agent is done.
    FinalAnswer(String),

    /// Controlled error / failure path.
    Error(String),
}

impl AgentStep {
    /// Returns true if this step represents a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentStep::FinalAnswer(_) | AgentStep::Error(_))
    }

    /// Returns a short human-readable label for logging.
    pub fn label(&self) -> &'static str {
        match self {
            AgentStep::LLMCall(_) => "llm_call",
            AgentStep::ToolCall(_) => "tool_call",
            AgentStep::FinalAnswer(_) => "final_answer",
            AgentStep::Error(_) => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_states() {
        assert!(AgentStep::FinalAnswer("done".into()).is_terminal());
        assert!(AgentStep::Error("fail".into()).is_terminal());
        assert!(!AgentStep::LLMCall(LLMCallSpec {
            backend: "ollama".into(),
            prompt: "hi".into(),
            correlation_id: 1,
        }).is_terminal());
    }

    #[test]
    fn test_labels() {
        assert_eq!(AgentStep::ToolCall(ToolInvocation {
            name: "search".into(),
            args: serde_json::json!({}),
            correlation_id: 42,
        }).label(), "tool_call");
    }
}
