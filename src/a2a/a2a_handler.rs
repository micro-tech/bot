//! A2A v1.0 JSON-RPC handlers for Helix.
//!
//! Served from the axum web server:
//! - `GET /.well-known/agent-card.json` — public discovery (no secrets).
//! - `POST /a2a` — JSON-RPC endpoint (protected by the web auth token
//!   middleware, same as the other mutating endpoints).
//!
//! Task execution reuses [`HelixAcpAgent`](crate::acp::agent::HelixAcpAgent)
//! — one agent loop, multiple protocol surfaces (ACP on stdio, A2A on HTTP).
//!
//! NOTE: this file replaces the old echo stub (`handle_a2a_message`, which
//! just replied "A2A response to X: processed Y"). Nothing in the codebase
//! published bus messages to 'a2a' expecting the echo (the module was never
//! wired into `lib.rs`/`main.rs`), so no shim was needed.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Json,
    },
};
use futures_util::stream::{self, Stream};
use log::info;
use serde_json::Value;

use super::tasks::TaskStore;
use super::types::{
    normalize_method, now_rfc3339, param_task_id, AgentAuthentication, AgentCapabilities,
    AgentCard, AgentSkill, Artifact, JsonRpcError, JsonRpcErrorBody, JsonRpcRequest, JsonRpcSuccess,
    Message, Part, Task, TaskState, TextPart, A2A_PROTOCOL_VERSION,
};
use crate::acp::agent::HelixAcpAgent;

/// Shared A2A state: task registry + the agent that executes tasks.
#[derive(Clone)]
pub struct A2aState {
    pub store: TaskStore,
    pub agent: Arc<HelixAcpAgent>,
    /// Base URL clients use for the JSON-RPC endpoint (for the Agent Card).
    pub base_url: String,
}

impl A2aState {
    pub fn new(base_url: String, max_steps: u32) -> Self {
        Self {
            store: TaskStore::new(),
            agent: Arc::new(HelixAcpAgent::new(max_steps)),
            base_url,
        }
    }
}

/// `GET /.well-known/agent-card.json` — public discovery document.
pub async fn agent_card(State(state): State<A2aState>) -> impl IntoResponse {
    let card = AgentCard {
        name: "Helix".to_string(),
        description: "Helix personal agent: chat, tools, and routines over A2A v1.0."
            .to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: A2A_PROTOCOL_VERSION.to_string(),
        url: format!("{}/a2a", state.base_url.trim_end_matches('/')),
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
        },
        authentication: AgentAuthentication {
            // Bearer token = the web auth token ([web] auth_token).
            schemes: vec!["bearer".to_string()],
        },
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        skills: vec![
            AgentSkill {
                id: "chat".to_string(),
                name: "Chat".to_string(),
                description: "General conversation and task execution.".to_string(),
                tags: vec!["general".to_string()],
            },
            AgentSkill {
                id: "tools".to_string(),
                name: "Tools".to_string(),
                description: "Run Helix tools: logs, notes, email, system status."
                    .to_string(),
                tags: vec!["tools".to_string()],
            },
        ],
    };
    Json(card)
}

/// `POST /a2a` — JSON-RPC dispatcher. Accepts both v1.0 (`message/send`)
/// and v0.3-era (`message:send`) method spellings.
pub async fn a2a_jsonrpc(
    State(state): State<A2aState>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let id = req.id.clone();
    let method = normalize_method(&req.method);
    match method.as_str() {
        "message/send" => {
            let resp = handle_message_send(&state, id.clone(), &req.params).await;
            ok_or_err(id, resp)
        }
        "message/stream" => {
            // Streaming needs SSE, not a single JSON-RPC response.
            match start_stream(state.clone(), id.clone(), req.params).await {
                Ok(sse) => sse.into_response(),
                Err(e) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(JsonRpcError::invalid_params(id, e)).unwrap()),
                )
                    .into_response(),
            }
        }
        "tasks/get" => {
            let Some(task_id) = param_task_id(&req.params) else {
                return Json(serde_json::to_value(JsonRpcError::invalid_params(
                    id,
                    "tasks/get requires params.id",
                ))
                .unwrap())
                .into_response();
            };
            match state.store.get(&task_id) {
                Some(task) => jsonrpc_ok(id, &task),
                None => Json(
                    serde_json::to_value(JsonRpcError::task_not_found(id, &task_id)).unwrap(),
                )
                .into_response(),
            }
        }
        "tasks/cancel" => {
            let Some(task_id) = param_task_id(&req.params) else {
                return Json(serde_json::to_value(JsonRpcError::invalid_params(
                    id,
                    "tasks/cancel requires params.id",
                ))
                .unwrap())
                .into_response();
            };
            let Some(current) = state.store.get(&task_id) else {
                return Json(
                    serde_json::to_value(JsonRpcError::task_not_found(id, &task_id)).unwrap(),
                )
                .into_response();
            };
            if current.status.state.is_terminal() {
                return Json(serde_json::to_value(JsonRpcError::invalid_params(
                    id,
                    format!(
                        "task {} is already in terminal state {:?}",
                        task_id, current.status.state
                    ),
                ))
                .unwrap())
                .into_response();
            }
            if !state.store.cancel(&task_id) {
                return Json(serde_json::to_value(JsonRpcError::invalid_params(
                    id,
                    format!("could not cancel task {}", task_id),
                ))
                .unwrap())
                .into_response();
            }
            match state.store.get(&task_id) {
                Some(task) => jsonrpc_ok(id, &task),
                None => Json(
                    serde_json::to_value(JsonRpcError::task_not_found(id, &task_id)).unwrap(),
                )
                .into_response(),
            }
        }
        // Helix extension (not in the A2A v1.0 spec): list known tasks.
        "tasks/list" => jsonrpc_ok(id, &state.store.list()),
        _ => Json(
            serde_json::to_value(JsonRpcError::method_not_found(id, &req.method)).unwrap(),
        )
        .into_response(),
    }
}

fn ok_or_err(id: Option<Value>, resp: Result<Value, (i64, String)>) -> axum::response::Response {
    match resp {
        Ok(v) => jsonrpc_ok(id, &v),
        Err((code, msg)) => Json(
            serde_json::to_value(JsonRpcError {
                jsonrpc: "2.0".to_string(),
                id,
                error: JsonRpcErrorBody { code, message: msg },
            })
            .unwrap(),
        )
        .into_response(),
    }
}

fn jsonrpc_ok(id: Option<Value>, result: &impl serde::Serialize) -> axum::response::Response {
    Json(
        serde_json::to_value(JsonRpcSuccess {
            jsonrpc: "2.0".to_string(),
            id,
            result: serde_json::to_value(result).unwrap_or(Value::Null),
        })
        .unwrap(),
    )
    .into_response()
}

/// Extract the user message + context id from `message/send` params.
/// Accepts v1.0 (`message`, `contextId`) and v0.3 (`sessionId`) shapes.
fn extract_send_params(params: &Value) -> Result<(Message, Option<String>), String> {
    let msg_val = params
        .get("message")
        .ok_or_else(|| "message/send requires params.message".to_string())?;
    let role = msg_val
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("user")
        .to_string();
    let parts: Vec<Part> = msg_val
        .get("parts")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if parts.is_empty() {
        // Also accept a bare string message for convenience.
        if let Some(text) = msg_val.get("text").and_then(|v| v.as_str()) {
            let message = Message {
                role,
                parts: vec![Part::Text(TextPart {
                    text: text.to_string(),
                })],
                message_id: format!("msg-{}", uuid::Uuid::new_v4().simple()),
            };
            let context_id = params
                .get("contextId")
                .or_else(|| params.get("sessionId"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            return Ok((message, context_id));
        }
        return Err("message/send requires params.message.parts (or .text)".to_string());
    }
    let message = Message {
        role,
        parts,
        message_id: msg_val
            .get("messageId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("msg-{}", uuid::Uuid::new_v4().simple())),
    };
    let context_id = params
        .get("contextId")
        .or_else(|| params.get("sessionId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok((message, context_id))
}

/// `message/send`: create the task, spawn execution, return the task.
async fn handle_message_send(
    state: &A2aState,
    _id: Option<Value>,
    params: &Value,
) -> Result<Value, (i64, String)> {
    let (message, context_id) = extract_send_params(params).map_err(|e| (-32602i64, e))?;
    let prompt = message.text_content();
    if prompt.trim().is_empty() {
        return Err((-32602, "message has no text content".to_string()));
    }

    let (task, _done_rx) = state.store.create(context_id, message);
    let task_id = task.id.clone();
    info!("A2A message/send -> task {}", task_id);

    // Spawn execution on the shared agent loop.
    let store = state.store.clone();
    let agent = Arc::clone(&state.agent);
    let worker_task_id = task_id.clone();
    let worker = tokio::spawn(async move {
        store.set_state(&worker_task_id, TaskState::Working, None);
        // The ACP agent wraps Helix's RuntimeLoop — one agent loop,
        // multiple protocol surfaces.
        let session_id = agent.new_session(std::path::PathBuf::from("."));
        let result = agent.run_prompt(&session_id, &prompt).await;

        let agent_message = Message {
            role: "agent".to_string(),
            parts: vec![Part::Text(TextPart {
                text: result.final_message.clone(),
            })],
            message_id: format!("msg-{}", uuid::Uuid::new_v4().simple()),
        };
        store.push_history(&worker_task_id, agent_message.clone());
        store.add_artifact(
            &worker_task_id,
            Artifact {
                artifact_id: format!("artifact-{}", uuid::Uuid::new_v4().simple()),
                name: "result".to_string(),
                parts: vec![Part::Text(TextPart {
                    text: result.final_message,
                })],
            },
        );
        let terminal = if result.had_final_answer {
            TaskState::Completed
        } else {
            TaskState::Failed
        };
        store.set_state(&worker_task_id, terminal, Some(agent_message));
        info!("A2A task {} finished: {:?}", worker_task_id, terminal);
    });
    state.store.set_worker(&task_id, worker);

    // Return the task as-created (spec: MAY be in working state already).
    let task = state.store.get(&task_id).unwrap_or(Task {
        id: task_id,
        context_id: None,
        status: crate::a2a::types::TaskStatus {
            state: TaskState::Submitted,
            message: None,
            timestamp: Some(now_rfc3339()),
        },
        artifacts: vec![],
        history: vec![],
    });
    serde_json::to_value(&task).map_err(|e| (-32603i64, e.to_string()))
}

/// `message/stream`: SSE stream of JSON-RPC task updates until terminal.
async fn start_stream(
    state: A2aState,
    id: Option<Value>,
    params: Value,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, String> {
    // Reuse message/send to create + spawn, then stream the task's progress.
    let task_value = handle_message_send(&state, id.clone(), &params)
        .await
        .map_err(|(_, msg)| msg)?;
    let task_id: String = task_value
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "internal: task has no id".to_string())?
        .to_string();

    let store = state.store.clone();
    let stream = stream::unfold(
        (store, task_id, id, None::<String>),
        |(store, task_id, rpc_id, last_seen)| async move {
            // Poll until the serialized task changes or it goes terminal.
            for _ in 0..40 {
                // ~10s max per gap; yields promptly on change.
                if let Some(task) = store.get(&task_id) {
                    let snapshot = serde_json::to_string(&task).unwrap_or_default();
                    let terminal = task.status.state.is_terminal();
                    if Some(&snapshot) != last_seen.as_ref() {
                        let event = Event::default().data(
                            serde_json::to_string(&serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": rpc_id,
                                "result": task,
                            }))
                            .unwrap_or_default(),
                        );
                        return Some((Ok(event), (store, task_id, rpc_id, Some(snapshot))));
                    }
                    if terminal {
                        return None;
                    }
                } else {
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            // Long-poll gap expired without change: keep the stream alive.
            let ping = Event::default().comment("keep-alive");
            Some((Ok(ping), (store, task_id, rpc_id, last_seen)))
        },
    );

    Ok(Sse::new(stream))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> A2aState {
        A2aState::new("http://127.0.0.1:8443".to_string(), 5)
    }

    #[test]
    fn extract_send_params_accepts_v1_shape() {
        let params = serde_json::json!({
            "message": {
                "role": "user",
                "parts": [{"kind": "text", "text": "hi"}],
                "messageId": "m1",
            },
            "contextId": "ctx-1",
        });
        let (msg, ctx) = extract_send_params(&params).expect("must parse");
        assert_eq!(msg.text_content(), "hi");
        assert_eq!(ctx.as_deref(), Some("ctx-1"));
    }

    #[test]
    fn extract_send_params_accepts_v03_session_id() {
        let params = serde_json::json!({
            "message": {
                "role": "user",
                "parts": [{"kind": "text", "text": "yo"}],
            },
            "sessionId": "sess-9",
        });
        let (msg, ctx) = extract_send_params(&params).expect("must parse");
        assert_eq!(msg.text_content(), "yo");
        assert_eq!(ctx.as_deref(), Some("sess-9"));
    }

    #[test]
    fn extract_send_params_rejects_empty() {
        let params = serde_json::json!({"message": {"role": "user", "parts": []}});
        assert!(extract_send_params(&params).is_err());
    }

    #[tokio::test]
    async fn message_send_creates_and_completes_task() {
        let state = test_state();
        let params = serde_json::json!({
            "message": {"role": "user", "parts": [{"kind": "text", "text": "hello a2a"}]},
        });
        let v = handle_message_send(&state, Some(serde_json::json!(1)), &params)
            .await
            .expect("send works");
        let task_id = v["id"].as_str().expect("has id").to_string();
        // Poll until terminal (the agent loop runs async).
        for _ in 0..100 {
            if let Some(t) = state.store.get(&task_id) {
                if t.status.state.is_terminal() {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let task = state.store.get(&task_id).expect("task exists");
        assert_eq!(task.status.state, TaskState::Completed);
        assert!(!task.artifacts.is_empty(), "expected a result artifact");
        assert_eq!(task.history.len(), 2, "user + agent messages");
    }

    #[tokio::test]
    async fn cancel_terminal_task_is_noop() {
        let state = test_state();
        let params = serde_json::json!({
            "message": {"role": "user", "parts": [{"kind": "text", "text": "x"}]},
        });
        let v = handle_message_send(&state, Some(serde_json::json!(1)), &params)
            .await
            .expect("send works");
        let task_id = v["id"].as_str().unwrap().to_string();
        // Wait for completion, then cancel must report false (already terminal).
        for _ in 0..100 {
            if let Some(t) = state.store.get(&task_id) {
                if t.status.state.is_terminal() {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(!state.store.cancel(&task_id));
    }
}
