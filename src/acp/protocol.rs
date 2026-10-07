//! ACP (Agent Client Protocol) wire types for Helix.
//!
//! The wire format comes from the official `agent-client-protocol` crate
//! (schema v1) — we re-export the types the server needs and add only
//! Helix-specific conveniences. This keeps Zed interop on the standard
//! schema instead of a hand-rolled dialect.
//!
//! Ported from grok-cli's `src/acp/protocol.rs`, minus grok-cli's
//! migration-era extensions (those were grok-cli-specific; the wire format
//! here is the plain crate schema).

use agent_client_protocol::schema::v1::{SessionId, SessionNotification};

/// Agent name reported to ACP clients (Zed shows this in the agent picker).
pub const HELIX_ACP_AGENT_NAME: &str = "helix";

/// Convenience: build an agent-message-chunk notification.
///
/// The schema types are `#[non_exhaustive]`, so we build via JSON
/// round-trip (the same approach grok-cli uses for such types).
pub fn agent_message_chunk(session_id: SessionId, text: impl Into<String>) -> SessionNotification {
    let sid = session_id.0.to_string();
    serde_json::from_value(serde_json::json!({
        "sessionId": sid,
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text.into() },
        }
    }))
    .expect("agent_message_chunk JSON must match the ACP schema")
}

/// Convenience: build a tool-call notification for a Helix tool invocation.
///
/// Built via JSON round-trip like [`agent_message_chunk`] (non-exhaustive types).
pub fn tool_call_notification(
    session_id: SessionId,
    tool_call_id: impl Into<String>,
    title: impl Into<String>,
    tool_name: impl Into<String>,
) -> SessionNotification {
    let sid = session_id.0.to_string();
    serde_json::from_value(serde_json::json!({
        "sessionId": sid,
        "update": {
            "sessionUpdate": "tool_call",
            "toolCallId": tool_call_id.into(),
            "title": title.into(),
            "name": tool_name.into(),
        }
    }))
    .expect("tool_call_notification JSON must match the ACP schema")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        InitializeRequest, NewSessionRequest, PromptRequest,
    };

    #[test]
    fn protocol_initialize_request_round_trips() {
        // GIVEN a minimal initialize request as Zed would send it:
        let raw = serde_json::json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
        });
        // WHEN we deserialize it into the crate type:
        let req: InitializeRequest = serde_json::from_value(raw).expect("must parse");
        // THEN it round-trips back through JSON unchanged in shape:
        let back = serde_json::to_value(&req).expect("must serialize");
        assert_eq!(back["protocolVersion"], 1);
    }

    #[test]
    fn protocol_session_new_round_trips() {
        // GIVEN a session/new request (mcpServers is required by this schema):
        let raw = serde_json::json!({ "cwd": "/home/cobble/helix", "mcpServers": [] });
        // WHEN parsed:
        let req: NewSessionRequest = serde_json::from_value(raw).expect("must parse");
        // THEN the cwd survives:
        assert_eq!(req.cwd.to_string_lossy(), "/home/cobble/helix");
    }

    #[test]
    fn protocol_prompt_request_round_trips() {
        // GIVEN a session/prompt request:
        let raw = serde_json::json!({
            "sessionId": "sess-1",
            "prompt": [{ "type": "text", "text": "hello helix" }],
        });
        // WHEN parsed:
        let req: PromptRequest = serde_json::from_value(raw).expect("must parse");
        // THEN the session id survives:
        let back = serde_json::to_value(&req).expect("must serialize");
        assert_eq!(back["sessionId"], "sess-1");
    }

    #[test]
    fn protocol_agent_message_chunk_builds() {
        // GIVEN a session id and text:
        let sid = SessionId::new("s1");
        // WHEN we build the notification:
        let notif = agent_message_chunk(sid, "hi");
        // THEN it serializes with the session id:
        let v = serde_json::to_value(&notif).expect("must serialize");
        assert_eq!(v["sessionId"], "s1");
    }
}
