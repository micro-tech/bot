//! ACP stdio server for Helix.
//!
//! Speaks the Agent Client Protocol over stdin/stdout using the official
//! `agent-client-protocol` crate's `Agent::builder()` pattern (the same
//! approach grok-cli uses): the crate owns transport framing and method
//! dispatch, and we register one typed handler per ACP method.
//!
//! Wire format on stdio is newline-delimited JSON-RPC (the crate's
//! `ByteStreams` transport). Logging goes to **stderr** — never stdout,
//! or the JSON-RPC stream corrupts.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use agent_client_protocol::schema::v1::{
    ClientNotification, ClientRequest, ContentBlock, InitializeRequest, InitializeResponse,
    ListSessionsRequest, ListSessionsResponse, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, SessionId, SessionNotification,
};
use agent_client_protocol::{
    Agent, ByteStreams, Client, ConnectionTo, Responder, on_receive_dispatch, on_receive_notification,
    on_receive_request,
};
use log::{debug, info, warn};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::agent::{HelixAcpAgent, ToolCallRecord};
use super::protocol::{agent_message_chunk, tool_call_notification, HELIX_ACP_AGENT_NAME};

/// Extract plain text from a prompt's content blocks.
fn prompt_text(req: &PromptRequest) -> String {
    req.prompt
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Send a `session/update` notification, logging (not failing) on error.
fn send_update(cx: &ConnectionTo<Client>, notif: SessionNotification) {
    if let Err(e) = cx.send_notification(notif) {
        warn!("ACP session/update send failed: {}", e);
    }
}

/// Run the ACP server on the given stdio handles until the client disconnects.
pub async fn run_acp_stdio(
    agent: HelixAcpAgent,
) -> anyhow::Result<()> {
    let agent = Arc::new(agent);
    let initialized = Arc::new(AtomicBool::new(false));

    let a_init = Arc::clone(&agent);
    let a_new = Arc::clone(&agent);
    let a_prompt = Arc::clone(&agent);
    let a_list = Arc::clone(&agent);
    let init_flag = Arc::clone(&initialized);

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let transport = ByteStreams::new(stdout.compat_write(), stdin.compat());

    let r = Agent
        .builder()
        .name(HELIX_ACP_AGENT_NAME)
        // ── initialize ──────────────────────────────────────────────────
        .on_receive_request(
            move |req: InitializeRequest,
                  responder: Responder<InitializeResponse>,
                  _cx: ConnectionTo<Client>| {
                let _agent = Arc::clone(&a_init);
                async move {
                    info!(
                        "ACP initialize from client (protocol v{})",
                        req.protocol_version
                    );
                    // Schema types are #[non_exhaustive]; build the response
                    // via JSON round-trip (same approach as grok-cli).
                    let resp: InitializeResponse = serde_json::from_value(serde_json::json!({
                        "protocolVersion": req.protocol_version,
                        "agentCapabilities": {},
                        "authMethods": [],
                    }))
                    .map_err(agent_client_protocol::util::internal_error)?;
                    responder.respond(resp)
                }
            },
            on_receive_request!(),
        )
        // ── session/new ─────────────────────────────────────────────────
        .on_receive_request(
            move |req: NewSessionRequest,
                  responder: Responder<NewSessionResponse>,
                  _cx: ConnectionTo<Client>| {
                let agent = Arc::clone(&a_new);
                let init = Arc::clone(&init_flag);
                async move {
                    // Tolerate clients that skip initialize.
                    init.store(true, Ordering::Release);
                    let sid = agent.new_session(req.cwd.clone());
                    info!("ACP session/new -> {}", sid);
                    let resp: NewSessionResponse =
                        serde_json::from_value(serde_json::json!({ "sessionId": sid }))
                            .map_err(agent_client_protocol::util::internal_error)?;
                    responder.respond(resp)
                }
            },
            on_receive_request!(),
        )
        // ── session/prompt (spawned so the loop stays responsive) ────────
        .on_receive_request(
            move |req: PromptRequest,
                  responder: Responder<PromptResponse>,
                  cx: ConnectionTo<Client>| {
                let agent = Arc::clone(&a_prompt);
                async move {
                    let cx2 = cx.clone();
                    cx.spawn(
                        async move { handle_prompt(req, responder, cx2, agent).await },
                    )?;
                    Ok(())
                }
            },
            on_receive_request!(),
        )
        // ── session/list ────────────────────────────────────────────────
        .on_receive_request(
            move |_req: ListSessionsRequest,
                  responder: Responder<ListSessionsResponse>,
                  _cx: ConnectionTo<Client>| {
                let agent = Arc::clone(&a_list);
                async move {
                    let sessions: Vec<SessionId> = agent
                        .list_sessions()
                        .into_iter()
                        .map(SessionId::new)
                        .collect();
                    // ListSessionsResponse wraps session ids; build via JSON
                    // round-trip to stay robust to schema shape drift.
                    let resp: ListSessionsResponse = serde_json::from_value(
                        serde_json::json!({ "sessions": sessions }),
                    )
                    .unwrap_or_else(|_| {
                        serde_json::from_value(serde_json::json!({})).expect("empty list resp")
                    });
                    responder.respond(resp)
                }
            },
            on_receive_request!(),
        )
        // ── client notifications (ignored, logged) ──────────────────────
        .on_receive_notification(
            move |_notif: ClientNotification, _cx: ConnectionTo<Client>| async move {
                debug!("ACP client notification received (no-op)");
                Ok(())
            },
            on_receive_notification!(),
        )
        // ── fallthrough: unknown methods get a clean error ──────────────
        .on_receive_dispatch(
            move |msg: agent_client_protocol::Dispatch<ClientRequest, ClientNotification>,
                  _cx: ConnectionTo<Client>| async move {
                warn!("ACP unhandled message: {:?}", msg);
                Ok(())
            },
            on_receive_dispatch!(),
        )
        .connect_to(transport)
        .await;

    if let Err(ref e) = r {
        info!("ACP session closed: {}", e);
    }
    Ok(())
}

/// Handle one `session/prompt`: run the Helix agent, stream updates.
///
/// Returns the crate's `Error` (not anyhow) because this future is handed
/// to `cx.spawn`, which requires `Output = Result<(), agent_client_protocol::Error>`.
async fn handle_prompt(
    req: PromptRequest,
    responder: Responder<PromptResponse>,
    cx: ConnectionTo<Client>,
    agent: Arc<HelixAcpAgent>,
) -> Result<(), agent_client_protocol::Error> {
    let err = |msg: String| agent_client_protocol::util::internal_error(msg);
    let session_id = req.session_id.0.to_string();
    let text = prompt_text(&req);
    info!(
        "ACP session/prompt: session={} text_len={}",
        session_id,
        text.len()
    );

    // Announce the turn so Zed shows activity immediately.
    send_update(
        &cx,
        agent_message_chunk(SessionId::new(session_id.clone()), "…"),
    );

    let result = agent.run_prompt(&session_id, &text).await;

    // Stream the tool calls that happened (post-hoc in v1: the runtime loop
    // doesn't expose live step hooks, so we replay the trace in order).
    for (i, tc) in result.tool_calls.iter().enumerate() {
        let notif = tool_call_notification(
            SessionId::new(session_id.clone()),
            format!("tc-{}-{}", session_id, i),
            tool_call_title(tc),
            tc.name.clone(),
        );
        send_update(&cx, notif);
    }

    // Stream the final answer as a message chunk.
    send_update(
        &cx,
        agent_message_chunk(
            SessionId::new(session_id.clone()),
            result.final_message.clone(),
        ),
    );

    let resp: PromptResponse = serde_json::from_value(serde_json::json!({
        "stopReason": "end_turn",
    }))
    .map_err(|e| err(format!("prompt response shape: {e}")))?;
    responder.respond(resp)?;
    Ok(())
}

/// Human-readable title for a tool call notification.
fn tool_call_title(tc: &ToolCallRecord) -> String {
    let args = tc.args.to_string();
    let short = if args.len() > 80 {
        format!("{}…", &args[..80])
    } else {
        args
    };
    format!("{} {}", tc.name, short)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_text_extracts_text_blocks() {
        // GIVEN a prompt request with text blocks:
        let req: PromptRequest = serde_json::from_value(serde_json::json!({
            "sessionId": "s1",
            "prompt": [
                { "type": "text", "text": "hello" },
                { "type": "text", "text": "world" },
            ],
        }))
        .expect("must parse");
        // WHEN we extract:
        let text = prompt_text(&req);
        // THEN both blocks join:
        assert_eq!(text, "hello\nworld");
    }

    #[test]
    fn prompt_text_ignores_non_text_blocks() {
        // GIVEN a prompt with a resource link mixed in:
        let req: PromptRequest = serde_json::from_value(serde_json::json!({
            "sessionId": "s1",
            "prompt": [
                { "type": "text", "text": "hi" },
                { "type": "resource_link", "uri": "file:///x", "name": "x" },
            ],
        }))
        .expect("must parse");
        // WHEN we extract:
        let text = prompt_text(&req);
        // THEN only the text survives:
        assert_eq!(text, "hi");
    }

    #[test]
    fn tool_call_title_truncates_long_args() {
        // GIVEN a tool call with huge args:
        let tc = ToolCallRecord {
            name: "repo_grep".to_string(),
            args: serde_json::json!({ "pattern": "x".repeat(200) }),
            result_summary: String::new(),
        };
        // WHEN we title it:
        let title = tool_call_title(&tc);
        // THEN it names the tool and truncates:
        assert!(title.starts_with("repo_grep "));
        assert!(title.len() < 120);
    }
}
