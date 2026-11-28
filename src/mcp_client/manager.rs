//! Async MCP (Model Context Protocol) connection manager.
//!
//! Spawns each configured MCP server as a subprocess over stdio, performs the
//! MCP initialize handshake, and exposes `tools/list` + `tools/call`.
//! Connections are established lazily on first use and tool schemas are
//! cached after the first fetch.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use log::{info, warn};
use rmcp::{
    RoleClient, ServiceError, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ContentBlock},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::Value;
use tokio::sync::Mutex as TokioMutex;

use crate::config::mcp::{McpConfig, McpServerConfig};

use super::naming;

/// How long to wait for a server subprocess to spawn + complete the MCP
/// initialize handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a single `tools/call` may take before we give up.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a single `tools/list` may take before we give up (task 397 / M7:
/// the old code had no timeout here, so a hung server blocked the defs fetch
/// forever).
const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// True when a service error means the transport is dead (the child died,
/// the pipe broke, or a send failed) — the connection must be dropped and
/// re-established, not just reported. A `McpError` (the server answered
/// with an MCP-level error) is NOT a transport error: the connection is
/// fine, only the call failed.
fn is_transport_error(e: &ServiceError) -> bool {
    matches!(
        e,
        ServiceError::TransportSend(_) | ServiceError::TransportClosed
    )
}

/// One live connection to an MCP server subprocess.
///
/// `RunningService` derefs to `Peer`, whose `call_tool` / `list_all_tools`
/// take `&self`, so a connection is shared through `Arc` and needs no
/// per-connection lock for calls — that is what makes per-server isolation
/// work (task 397 / M7): a hung call on one server holds no lock that
/// could block another server.
struct McpConnection {
    service: RunningService<RoleClient, ()>,
}

/// Async manager for all configured MCP servers.
///
/// Spawns each configured MCP server as a subprocess over stdio, performs the
/// MCP initialize handshake, and exposes `tools/list` + `tools/call`.
/// Connections are established lazily on first use and tool schemas are
/// cached after the first fetch.
///
/// Locking (task 397 / M7): the old code held ONE global mutex across every
/// call (up to 120 s), so a hung server blocked all MCP calls and the
/// chat-command worker leaked while holding it. Now the maps are behind
/// short-held mutexes that are never held across network I/O: a call
/// borrows the connection `Arc` (brief lock), then awaits on the shared
/// service with no lock held. A hung server can therefore only stall its
/// own calls.
pub struct McpManager {
    config: McpConfig,
    connections: TokioMutex<HashMap<String, Arc<McpConnection>>>,
    /// Cached Ollama-style tool definitions (namespaced), built once.
    /// Only a NON-EMPTY fetch is cached (task 397 / M7): caching an empty
    /// result forever meant a server that was down at first fetch stayed
    /// invisible for the life of the process.
    defs_cache: TokioMutex<Option<Vec<Value>>>,
}

impl McpManager {
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            connections: TokioMutex::new(HashMap::new()),
            defs_cache: TokioMutex::new(None),
        }
    }

    /// Drop the connection to `server` (if any) so the next use spawns a
    /// fresh one. Called on transport errors (reconnect) — never on a
    /// plain tool error, where the connection is fine.
    async fn evict(&self, server_name: &str) {
        self.connections.lock().await.remove(server_name);
    }

    /// Ensure we have a live connection to `server`, spawning the subprocess
    /// on first use. The map lock is held only to check/insert — never
    /// across the spawn + handshake, so a slow server cannot block other
    /// servers' connects. A failed server is skipped with a warning and is
    /// retried on the next use (its tools stay unavailable until then);
    /// nothing is cached for a failed connect.
    async fn ensure_connected(
        &self,
        server: &McpServerConfig,
    ) -> Result<Arc<McpConnection>, String> {
        {
            let guard = self.connections.lock().await;
            if let Some(conn) = guard.get(&server.name) {
                return Ok(Arc::clone(conn));
            }
        }

        info!(
            "mcp: spawning server '{}' ({} {:?})",
            server.name, server.command, server.args
        );

        let mut cmd = tokio::process::Command::new(&server.command);
        cmd.args(&server.args);
        for (k, v) in &server.env {
            cmd.env(k, v);
        }

        let transport = TokioChildProcess::new(cmd)
            .map_err(|e| format!("mcp: failed to spawn server '{}': {e}", server.name))?;

        let service = tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport))
            .await
            .map_err(|_| {
                format!(
                    "mcp: server '{}' did not complete the MCP handshake within {CONNECT_TIMEOUT:?}",
                    server.name
                )
            })?
            .map_err(|e| format!("mcp: handshake with server '{}' failed: {e}", server.name))?;

        let conn = Arc::new(McpConnection { service });
        // First-wins on a concurrent-connect race: if another task already
        // inserted this server while we handshook, use theirs and drop
        // ours (its stdio closes, so the child exits on its own).
        let mut guard = self.connections.lock().await;
        Ok(match guard.get(&server.name) {
            Some(existing) => Arc::clone(existing),
            None => {
                guard.insert(server.name.clone(), Arc::clone(&conn));
                conn
            }
        })
    }

    /// Return Ollama-style tool definitions for every reachable server,
    /// namespaced as `mcp__<server>__<tool>`. Results are cached after the
    /// first NON-EMPTY fetch; an empty fetch is retried on the next call
    /// (a server that was down at first fetch must not stay invisible).
    /// Each server's `list_all_tools` is bounded by LIST_TIMEOUT, and a
    /// transport failure evicts that server so the next fetch reconnects.
    pub async fn tool_definitions(&self) -> Vec<Value> {
        {
            let guard = self.defs_cache.lock().await;
            if let Some(cached) = &*guard
                && !cached.is_empty()
            {
                return cached.clone();
            }
        }

        let mut defs = Vec::new();
        for server in self.config.usable_servers() {
            let conn = match self.ensure_connected(&server).await {
                Ok(c) => c,
                Err(e) => {
                    warn!("{e}");
                    continue;
                }
            };
            match tokio::time::timeout(LIST_TIMEOUT, conn.service.list_all_tools()).await {
                Ok(Ok(tools)) => {
                    for tool in tools {
                        let description = tool
                            .description
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| format!("MCP tool '{}'", tool.name));
                        defs.push(serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": naming::tool_name(&server.name, &tool.name),
                                "description": format!("[MCP:{}] {description}", server.name),
                                "parameters": Value::Object((*tool.input_schema).clone()),
                            }
                        }));
                    }
                }
                Ok(Err(e)) => {
                    warn!("mcp: tools/list failed for server '{}': {e}", server.name);
                    if is_transport_error(&e) {
                        self.evict(&server.name).await;
                    }
                }
                Err(_) => {
                    warn!(
                        "mcp: tools/list for server '{}' timed out after {LIST_TIMEOUT:?}",
                        server.name
                    );
                }
            }
        }

        // Cache only a non-empty result (task 397 / M7): an empty fetch
        // means every server was down, and caching it would hide a server
        // that comes up later.
        if !defs.is_empty() {
            let mut guard = self.defs_cache.lock().await;
            *guard = Some(defs.clone());
        }
        defs
    }

    /// Call an upstream tool on a server. `server`/`tool` are the un-namespaced
    /// parts (see [`super::naming::parse`]). Bounded by CALL_TIMEOUT; on a
    /// transport error the connection is dropped and the call is retried
    /// once against a fresh connection.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        args: &Value,
    ) -> Result<String, String> {
        let server = self
            .config
            .usable_servers()
            .into_iter()
            .find(|s| s.name == server_name)
            .ok_or_else(|| format!("mcp: no server named '{server_name}' is configured"))?;

        let mut params = CallToolRequestParams::new(tool_name.to_owned());
        if let Some(obj) = args.as_object() {
            params = params.with_arguments(obj.clone());
        }

        match self.call_once(&server, &params).await {
            Ok(result) => Ok(result_to_string(&result)),
            Err(CallAttemptError::Transport(e)) => {
                // The transport died (child exited / pipe broke): drop the
                // dead connection and retry once on a fresh one. A plain
                // tool error is NOT retried — the connection is fine.
                warn!(
                    "mcp: transport error calling '{tool_name}' on '{server_name}' ({e}); reconnecting once"
                );
                self.evict(server_name).await;
                self.call_once(&server, &params)
                    .await
                    .map(|r| result_to_string(&r))
                    .map_err(|f| f.into_message(tool_name))
            }
            Err(other) => Err(other.into_message(tool_name)),
        }
    }

    /// One attempt at a tool call against a live connection, bounded by
    /// CALL_TIMEOUT. Distinguishes a dead transport (worth one reconnect
    /// + retry) from any other failure.
    async fn call_once(
        &self,
        server: &McpServerConfig,
        params: &CallToolRequestParams,
    ) -> Result<CallToolResult, CallAttemptError> {
        let conn = self
            .ensure_connected(server)
            .await
            .map_err(CallAttemptError::Other)?;
        match tokio::time::timeout(CALL_TIMEOUT, conn.service.call_tool(params.clone())).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(e)) if is_transport_error(&e) => Err(CallAttemptError::Transport(e)),
            Ok(Err(e)) => Err(CallAttemptError::Other(format!(
                "mcp: tool call failed: {e}"
            ))),
            Err(_) => Err(CallAttemptError::Other(format!(
                "mcp: tool call timed out after {CALL_TIMEOUT:?}"
            ))),
        }
    }
}

/// One call attempt's failure, so `call_tool` can tell a dead transport
/// (retry once on a fresh connection) from any other failure.
enum CallAttemptError {
    /// The transport died mid-call — reconnect and retry once.
    Transport(ServiceError),
    /// Anything else (spawn/handshake failure, MCP-level error, timeout)
    /// — report it, do not retry.
    Other(String),
}

impl CallAttemptError {
    fn into_message(self, tool_name: &str) -> String {
        match self {
            CallAttemptError::Transport(e) => {
                format!("mcp: tool call '{tool_name}' failed: {e}")
            }
            CallAttemptError::Other(msg) => msg,
        }
    }
}

/// Flatten a `tools/call` result into the plain String the tool dispatcher
/// expects. Text blocks are concatenated; non-text blocks get a placeholder.
/// When the server flagged the result as an error, the text is marked as such.
fn result_to_string(result: &CallToolResult) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(structured) = &result.structured_content {
        parts.push(
            serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string()),
        );
    }

    for block in &result.content {
        match block {
            ContentBlock::Text(t) => parts.push(t.text.clone()),
            ContentBlock::Image(_) => parts.push("[mcp: image content omitted]".to_string()),
            ContentBlock::Audio(_) => parts.push("[mcp: audio content omitted]".to_string()),
            ContentBlock::Resource(_) => parts.push("[mcp: embedded resource omitted]".to_string()),
            ContentBlock::ResourceLink(r) => parts.push(format!("[mcp: resource link: {}]", r.uri)),
            // ContentBlock is #[non_exhaustive]: future block kinds degrade
            // to a placeholder instead of failing to compile.
            _ => parts.push("[mcp: non-text content omitted]".to_string()),
        }
    }

    if parts.is_empty() {
        parts.push("[mcp: tool returned no content]".to_string());
    }

    let joined = parts.join("\n");
    if result.is_error.unwrap_or(false) {
        format!("MCP tool reported an error:\n{joined}")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::mcp::McpServerConfig;

    fn bad_server_config() -> McpConfig {
        McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "ghost".into(),
                command: "/nonexistent/helix-mcp-test-binary".into(),
                ..Default::default()
            }],
        }
    }

    #[test]
    fn is_transport_error_matches_only_dead_transports() {
        // Task 397 / M7: only a dead transport triggers evict + reconnect.
        // An MCP-level error (the server answered) or a timeout is NOT a
        // transport error — the connection is fine, only the call failed.
        assert!(is_transport_error(&ServiceError::TransportClosed));
        assert!(!is_transport_error(&ServiceError::Timeout {
            timeout: Duration::from_secs(1)
        }));
        let mcp_err = ServiceError::McpError(rmcp::model::ErrorData {
            code: rmcp::model::ErrorCode::INTERNAL_ERROR,
            message: "boom".into(),
            data: None,
        });
        assert!(!is_transport_error(&mcp_err));
    }

    #[tokio::test]
    async fn empty_defs_fetch_does_not_poison_manager() {
        // Task 397 / M7: an empty defs fetch (all servers down) must not
        // be cached forever and must not poison later calls — a second
        // fetch retries (returns empty again, no panic), and call_tool
        // still reports the spawn failure.
        let mgr = McpManager::new(bad_server_config());
        assert!(mgr.tool_definitions().await.is_empty());
        assert!(mgr.tool_definitions().await.is_empty());
        let err = mgr
            .call_tool("ghost", "whatever", &Value::Null)
            .await
            .expect_err("must fail");
        assert!(err.contains("ghost"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn definitions_empty_when_server_cannot_spawn() {
        let mgr = McpManager::new(bad_server_config());
        let defs = mgr.tool_definitions().await;
        assert!(defs.is_empty());
    }

    #[tokio::test]
    async fn mail_template_without_binary_advertises_no_tools() {
        // Task 255: the support@ watcher's mail-mcp entry degrades gracefully
        // when the binary is not installed (the sandbox never has it).
        // No mcp__mail__* tool may be advertised, and nothing may panic.
        let mgr = McpManager::new(McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "mail".into(),
                command: "/home/cobble/.local/bin/mail-mcp".into(),
                ..Default::default()
            }],
        });
        let defs = mgr.tool_definitions().await;
        assert!(
            defs.is_empty(),
            "missing mail-mcp binary must advertise no tools"
        );
        for d in &defs {
            let name = d
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            assert!(!name.starts_with("mcp__mail__"), "leaked tool: {name}");
        }
    }

    #[tokio::test]
    async fn call_reports_spawn_failure_as_error() {
        let mgr = McpManager::new(bad_server_config());
        let err = mgr
            .call_tool("ghost", "whatever", &Value::Null)
            .await
            .expect_err("must fail");
        assert!(err.contains("ghost"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn call_unknown_server_is_an_error() {
        let mgr = McpManager::new(McpConfig::default());
        let err = mgr
            .call_tool("nope", "whatever", &Value::Null)
            .await
            .expect_err("must fail");
        assert!(err.contains("nope"), "unexpected error: {err}");
    }

    #[test]
    fn result_to_string_extracts_text() {
        use rmcp::model::{CallToolResult, ContentBlock, TextContent};
        let result = CallToolResult::success(vec![ContentBlock::Text(TextContent::new("hello"))]);
        assert_eq!(result_to_string(&result), "hello");
    }

    #[test]
    fn result_to_string_marks_errors() {
        use rmcp::model::{CallToolResult, ContentBlock, TextContent};
        let mut result =
            CallToolResult::success(vec![ContentBlock::Text(TextContent::new("boom"))]);
        result.is_error = Some(true);
        let s = result_to_string(&result);
        assert!(s.contains("error"), "got: {s}");
        assert!(s.contains("boom"), "got: {s}");
    }

    // Live end-to-end test against the real google-mcp server binary.
    // Requires the binary on PATH (or MCP_TEST_SERVER set) AND Google OAuth
    // credentials; tools/list itself needs no auth, but the binary must run.
    #[tokio::test]
    #[ignore]
    async fn live_list_tools_against_google_mcp() {
        let command = std::env::var("MCP_TEST_SERVER").unwrap_or_else(|_| "google-mcp".into());
        let mgr = McpManager::new(McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "google".into(),
                command,
                ..Default::default()
            }],
        });
        let defs = mgr.tool_definitions().await;
        assert!(!defs.is_empty(), "expected tools from google-mcp");
        let names: Vec<_> = defs
            .iter()
            .filter_map(|d| d.get("function")?.get("name")?.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("gmail_search")),
            "missing gmail_search in {names:?}"
        );
    }
}
