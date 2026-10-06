//! Elicitation support for the Helix ACP server.
//!
//! Types for the ACP `session/elicit` flow (agent asks the user for
//! structured input mid-turn). Ported from grok-cli's
//! `src/acp/elicitation.rs` — the shape is protocol-level, not
//! agent-specific.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Elicitation request (matches the ACP schema shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElicitationRequest {
    pub session_id: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>, // JSON Schema for structured input
}

/// Elicitation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElicitationResponse {
    pub cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
}

impl ElicitationResponse {
    pub fn cancelled() -> Self {
        Self {
            cancelled: true,
            content: None,
        }
    }

    pub fn with_content(content: Value) -> Self {
        Self {
            cancelled: false,
            content: Some(content),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elicitation_response_cancelled_round_trips() {
        // GIVEN a cancelled response:
        let r = ElicitationResponse::cancelled();
        // WHEN serialized:
        let v = serde_json::to_value(&r).expect("must serialize");
        // THEN the shape matches the protocol:
        assert_eq!(v["cancelled"], true);
        assert!(v.get("content").is_none());
    }

    #[test]
    fn elicitation_response_with_content_round_trips() {
        // GIVEN a content response:
        let r = ElicitationResponse::with_content(serde_json::json!({"name": "helix"}));
        // WHEN serialized and parsed back:
        let v = serde_json::to_value(&r).expect("must serialize");
        let back: ElicitationResponse = serde_json::from_value(v).expect("must parse");
        // THEN content survives:
        assert!(!back.cancelled);
        assert_eq!(back.content.unwrap()["name"], "helix");
    }
}
