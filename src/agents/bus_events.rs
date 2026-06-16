//! Bus event types for the Unified Agent Runtime.

use serde::{Deserialize, Serialize};

/// Event emitted when an agent run completes successfully.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFinished {
    pub goal: String,
    pub result: String,
    pub steps_taken: u32,
    pub correlation_id: u64,
}

/// Event emitted when an agent run fails or is halted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentError {
    pub goal: String,
    pub error: String,
    pub steps_taken: u32,
    pub correlation_id: u64,
}
