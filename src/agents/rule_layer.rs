//! Rule Layer — validates planner output for safety and correctness.
//!
//! Enforces:
//! - No hallucinated tool names
//! - No invalid JSON in tool args
//! - Max steps / retries
//! - No recursive tool calls (same tool > N times in a row)

use crate::agents::agent_step::{AgentStep, ToolInvocation};
use std::collections::HashSet;

pub struct RuleLayer {
    allowed_tools: HashSet<String>,
    max_consecutive_tool_calls: u32,
}

impl RuleLayer {
    pub fn new(allowed_tools: Vec<String>) -> Self {
        Self {
            allowed_tools: allowed_tools.into_iter().collect(),
            max_consecutive_tool_calls: 5,
        }
    }

    /// Returns Ok(()) if the step is valid, Err(reason) otherwise.
    pub fn validate(&self, step: &AgentStep, consecutive_tool_calls: u32) -> Result<(), String> {
        match step {
            AgentStep::ToolCall(ToolInvocation { name, args, .. }) => {
                if !self.allowed_tools.contains(name) {
                    return Err(format!("Hallucinated tool name: {}", name));
                }
                if consecutive_tool_calls >= self.max_consecutive_tool_calls {
                    return Err("Too many consecutive tool calls".into());
                }
                // Basic JSON sanity check
                if !args.is_object() && !args.is_null() {
                    return Err("Tool args must be a JSON object or null".into());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::agent_step::ToolInvocation;

    #[test]
    fn test_hallucinated_tool() {
        let rules = RuleLayer::new(vec!["search".into(), "calc".into()]);
        let bad = AgentStep::ToolCall(ToolInvocation {
            name: "delete_everything".into(),
            args: serde_json::json!({}),
            correlation_id: 1,
        });
        assert!(rules.validate(&bad, 0).is_err());
    }

    #[test]
    fn test_valid_tool() {
        let rules = RuleLayer::new(vec!["search".into()]);
        let good = AgentStep::ToolCall(ToolInvocation {
            name: "search".into(),
            args: serde_json::json!({"q": "rust"}),
            correlation_id: 2,
        });
        assert!(rules.validate(&good, 0).is_ok());
    }
}
