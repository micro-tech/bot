//! Unit tests for ToolSupervisorV2 (Task 158.5)

use crate::agents::tool_supervisor_v2::{ToolError, ToolResult, ToolSupervisorV2};
use serde_json::json;

#[test]
fn test_tool_result_success() {
    let result = ToolResult::Success(json!({"ok": true}));
    assert!(result.is_success());
    assert!(!result.is_retryable());
}

#[test]
fn test_tool_error_classification_not_found() {
    let err = ToolError::NotFound {
        message: "Tool 'foo' not found".into(),
    };
    assert!(!err.is_retryable());
}

#[test]
fn test_tool_error_classification_rate_limit() {
    let err = ToolError::RateLimit {
        message: "rate limit".into(),
        retry_after_ms: Some(1000),
    };
    assert!(err.is_retryable());
}

#[tokio::test]
async fn test_supervisor_unknown_tool_is_fatal() {
    let supervisor = ToolSupervisorV2::new(0);
    let result = supervisor.execute("nonexistent_tool_xyz", &json!({})).await;
    assert!(matches!(result, ToolResult::FatalError(_)));
}

#[tokio::test]
async fn test_supervisor_valid_tool_returns_success() {
    let supervisor = ToolSupervisorV2::new(0);
    let result = supervisor.execute("list_tools", &json!({})).await;
    assert!(result.is_success());
}
