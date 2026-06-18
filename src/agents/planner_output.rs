//! PlannerOutput enum — the canonical decision type returned by any Planner.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlannerOutput {
    LLMCall {
        prompt: String,
        reasoning: Option<String>,
    },
    ToolCall {
        tool_name: String,
        args_json: serde_json::Value,
        reasoning: Option<String>,
    },
    FinalAnswer {
        message: String,
        reasoning: Option<String>,
    },
    Error {
        message: String,
    },
}
