//! ToolSupervisor trait — abstraction for tool execution.
//!
//! This allows swapping real tool execution with mocks for testing.

use crate::agents::agent_step::ToolInvocation;
use async_trait::async_trait;

#[async_trait]
pub trait ToolSupervisor: Send + Sync {
    /// Execute a tool and return the result or an error.
    async fn execute(&self, invocation: &ToolInvocation) -> Result<serde_json::Value, String>;

    /// Check if a tool name is allowed (security).
    fn is_allowed(&self, tool_name: &str) -> bool;
}

/// Simple in-memory supervisor for testing.
pub struct MockToolSupervisor {
    allowed: Vec<String>,
}

impl MockToolSupervisor {
    pub fn new(allowed: Vec<String>) -> Self {
        Self { allowed }
    }
}

#[async_trait]
impl ToolSupervisor for MockToolSupervisor {
    async fn execute(&self, invocation: &ToolInvocation) -> Result<serde_json::Value, String> {
        if !self.is_allowed(&invocation.name) {
            return Err(format!("Tool not allowed: {}", invocation.name));
        }
        Ok(serde_json::json!({"mock": true, "tool": invocation.name}))
    }

    fn is_allowed(&self, tool_name: &str) -> bool {
        self.allowed.contains(&tool_name.to_string())
    }
}
