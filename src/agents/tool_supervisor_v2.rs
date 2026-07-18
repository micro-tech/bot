//! Tool Supervisor v2 — Validation, Error Classification, and Retry
//!
//! Provides structured tool execution with error classification and retry logic.

use serde_json::Value;
use std::time::Duration;

/// Classified tool execution errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    InvalidArgs {
        message: String,
        context: Option<String>,
    },
    ExecutionError {
        message: String,
        context: Option<String>,
    },
    NotFound {
        message: String,
    },
    RateLimit {
        message: String,
        retry_after_ms: Option<u64>,
    },
    Timeout {
        message: String,
    },
    Unknown {
        message: String,
    },
}

impl ToolError {
    pub fn message(&self) -> &str {
        match self {
            ToolError::InvalidArgs { message, .. } => message,
            ToolError::ExecutionError { message, .. } => message,
            ToolError::NotFound { message } => message,
            ToolError::RateLimit { message, .. } => message,
            ToolError::Timeout { message } => message,
            ToolError::Unknown { message } => message,
        }
    }

    /// Returns true if this error type is considered retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ToolError::RateLimit { .. } | ToolError::Timeout { .. } | ToolError::ExecutionError { .. }
        )
    }
}

/// Canonical result of a tool call.
#[derive(Debug, Clone)]
pub enum ToolResult {
    Success(Value),
    RetryableError(ToolError),
    FatalError(ToolError),
}

impl ToolResult {
    pub fn is_success(&self) -> bool {
        matches!(self, ToolResult::Success(_))
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, ToolResult::RetryableError(_))
    }
}

/// Tool Supervisor V2 — wraps tool execution with validation + retry.
pub struct ToolSupervisorV2 {
    pub max_retries: u32,
    pub retry_delay: Duration,
}

impl Default for ToolSupervisorV2 {
    fn default() -> Self {
        Self {
            max_retries: 2,
            retry_delay: Duration::from_millis(300),
        }
    }
}

impl ToolSupervisorV2 {
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            retry_delay: Duration::from_millis(300),
        }
    }

    /// Execute a tool with validation, error classification, and retry.
    pub async fn execute(
        &self,
        tool_name: &str,
        args: &Value,
    ) -> ToolResult {
        // Basic argument validation (placeholder)
        if args.is_null() && tool_name != "noop" {
            return ToolResult::FatalError(ToolError::InvalidArgs {
                message: "Arguments cannot be null for this tool".to_string(),
                context: Some(tool_name.to_string()),
            });
        }

        // Note: Current tool execution (via crate::tools::execute) returns a plain String.
        // Only "Unknown tool..." is treated as a fatal error. All other output is success.
        //
        // Real retry logic (using self.max_retries + self.retry_delay + classify_error + is_retryable)
        // can be added once tools return structured errors or we parse transient failures from the raw string.
        // For now we execute once; the loop was removed because it never iterated (clippy::never_loop).

        let raw = crate::tools::execute(tool_name, args);

        if raw.starts_with("Unknown tool") {
            let classified = self.classify_error(tool_name, &raw);
            return ToolResult::FatalError(classified);
        }

        // Treat any other output as success
        ToolResult::Success(serde_json::json!(raw))
    }

    fn classify_error(&self, tool_name: &str, msg: &str) -> ToolError {
        let lower = msg.to_lowercase();
        if lower.contains("not found") || lower.contains("unknown tool") {
            ToolError::NotFound {
                message: format!("Tool '{}' not found", tool_name),
            }
        } else if lower.contains("rate limit") || lower.contains("too many requests") {
            ToolError::RateLimit {
                message: msg.to_string(),
                retry_after_ms: Some(1000),
            }
        } else if lower.contains("timeout") {
            ToolError::Timeout {
                message: msg.to_string(),
            }
        } else if lower.contains("invalid") || lower.contains("argument") {
            ToolError::InvalidArgs {
                message: msg.to_string(),
                context: Some(tool_name.to_string()),
            }
        } else {
            ToolError::ExecutionError {
                message: msg.to_string(),
                context: Some(tool_name.to_string()),
            }
        }
    }
}
