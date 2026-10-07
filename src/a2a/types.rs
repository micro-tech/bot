//! A2A v1.0 JSON-RPC types for Helix.
//!
//! Shapes follow the Linux Foundation `lf.a2a.v1` spec (v1.0.0), JSON-RPC
//! binding. There is no official Rust SDK, so these are hand-rolled against
//! the spec. Where the spec evolved (v0.3 `sessionId` → v1.0 `contextId`,
//! `/`-separated method names), we accept both spellings on input and emit
//! v1.0 on output.

use serde::{Deserialize, Serialize};

/// Protocol version we implement.
pub const A2A_PROTOCOL_VERSION: &str = "1.0";

/// Task lifecycle states (A2A v1.0, kebab-case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Canceled,
    Failed,
    Rejected,
    AuthRequired,
    Unknown,
}

impl TaskState {
    /// Terminal states: no further transitions expected.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Completed
                | TaskState::Canceled
                | TaskState::Failed
                | TaskState::Rejected
        )
    }
}

/// A content part inside a message. v1.0 tags parts with `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Part {
    Text(TextPart),
    #[serde(rename = "file")]
    File(FilePart),
    #[serde(rename = "data")]
    Data(DataPart),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPart {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePart {
    pub name: String,
    #[serde(default)]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPart {
    pub data: serde_json::Value,
}

impl Part {
    /// Best-effort text extraction for prompt building.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Part::Text(t) => Some(&t.text),
            _ => None,
        }
    }
}

/// A message in a task's history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: String, // "user" | "agent"
    pub parts: Vec<Part>,
    pub message_id: String,
}

impl Message {
    /// Concatenate text parts for the agent prompt.
    pub fn text_content(&self) -> String {
        self.parts
            .iter()
            .filter_map(Part::as_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Status block of a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// An artifact produced by a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    pub name: String,
    pub parts: Vec<Part>,
}

/// A task: the unit of work in A2A.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    /// v1.0 field. We also accept v0.3 `sessionId` on input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub history: Vec<Message>,
}

/// A skill advertised on the Agent Card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// The Agent Card served at `/.well-known/agent-card.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub version: String,
    pub protocol_version: String,
    /// Base URL clients POST JSON-RPC to (the `/a2a` endpoint).
    pub url: String,
    pub capabilities: AgentCapabilities,
    pub authentication: AgentAuthentication,
    #[serde(default)]
    pub default_input_modes: Vec<String>,
    #[serde(default)]
    pub default_output_modes: Vec<String>,
    #[serde(default)]
    pub skills: Vec<AgentSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: bool,
    #[serde(default)]
    pub push_notifications: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuthentication {
    pub schemes: Vec<String>,
}

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// JSON-RPC 2.0 success response.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcSuccess {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub result: serde_json::Value,
}

/// JSON-RPC 2.0 error response.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub error: JsonRpcErrorBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcErrorBody {
    pub code: i64,
    pub message: String,
}

impl JsonRpcError {
    pub fn method_not_found(id: Option<serde_json::Value>, method: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            error: JsonRpcErrorBody {
                code: -32601,
                message: format!("method not found: {}", method),
            },
        }
    }

    pub fn invalid_params(id: Option<serde_json::Value>, msg: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            error: JsonRpcErrorBody {
                code: -32602,
                message: msg.into(),
            },
        }
    }

    pub fn task_not_found(id: Option<serde_json::Value>, task_id: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            error: JsonRpcErrorBody {
                code: -32001,
                message: format!("task not found: {}", task_id),
            },
        }
    }
}

/// Normalize a method name: accept both `message/send` (v1.0) and
/// `message:send` (v0.3-era) spellings.
pub fn normalize_method(method: &str) -> String {
    method.replace(':', "/").to_lowercase()
}

/// Extract a task id from params, accepting `id` (v1.0).
pub fn param_task_id(params: &serde_json::Value) -> Option<String> {
    params
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Current UTC timestamp in RFC 3339, for status blocks.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_state_terminal_classification() {
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Canceled.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(!TaskState::Working.is_terminal());
        assert!(!TaskState::Submitted.is_terminal());
        assert!(!TaskState::InputRequired.is_terminal());
    }

    #[test]
    fn task_state_wire_format_is_kebab_case() {
        let s = serde_json::to_value(TaskState::InputRequired).unwrap();
        assert_eq!(s, serde_json::json!("input-required"));
    }

    #[test]
    fn normalize_method_accepts_both_separators() {
        assert_eq!(normalize_method("message/send"), "message/send");
        assert_eq!(normalize_method("message:send"), "message/send");
        assert_eq!(normalize_method("tasks:GET"), "tasks/get");
    }

    #[test]
    fn message_text_content_joins_text_parts() {
        let m = Message {
            role: "user".to_string(),
            parts: vec![
                Part::Text(TextPart { text: "hello".into() }),
                Part::Text(TextPart { text: "world".into() }),
            ],
            message_id: "m1".into(),
        };
        assert_eq!(m.text_content(), "hello\nworld");
    }

    #[test]
    fn agent_card_serializes_required_fields() {
        let card = AgentCard {
            name: "Helix".into(),
            description: "d".into(),
            version: "1".into(),
            protocol_version: A2A_PROTOCOL_VERSION.into(),
            url: "http://127.0.0.1:8443/a2a".into(),
            capabilities: AgentCapabilities { streaming: true, push_notifications: false },
            authentication: AgentAuthentication { schemes: vec!["bearer".into()] },
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
            skills: vec![],
        };
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(v["protocolVersion"], "1.0");
        assert_eq!(v["capabilities"]["streaming"], true);
        assert_eq!(v["authentication"]["schemes"][0], "bearer");
    }
}
