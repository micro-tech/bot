//! Async MCP (Model Context Protocol) connection manager.
//!
//! Spawns each configured MCP server as a subprocess over stdio, performs the
//! MCP initialize handshake, and exposes `tools/list` + `tools/call`.
//! Connections are established lazily on first use and tool schemas are
//! cached after the first fetch.

use std::collections::HashMap;
use std::time::Duration;

use log::{info, warn};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ContentBlock},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::Value;

use crate::config::mcp::{McpConfig, McpServerConfig};

use super::naming;

/// How long to wait for a server subprocess to spawn + complete the MCP
/// initialize handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a single `tools/call` may take before we give up.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// One live connection to an MCP server subprocess.
struct McpConnection {
    service: RunningService<RoleClient, ()>,
}

/// Async manager for all configured MCP servers.
///
/// Kept behind a `tokio::sync::Mutex` in a global; all public methods are
/// async and are driven from sync call sites through the bridge in
/// [`super::bridge`].
pub struct McpManager {
    config: McpConfig,
    connections: HashMap<String, McpConnection>,
    /// Cached Ollama-style tool definitions (namespaced), built once.
    defs_cache: Option<Vec<Value>>,
}

impl McpManager {
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            connections: HashMap::new(),
            defs_cache: None,
        }
    }

    /// Ensure we have a live connection to `server`, spawning the subprocess
    /// on first use. A failed server is skipped with a warning and never
    /// retried within this process (its tools simply stay unavailable).
    async fn ensure_connected(&mut self, server: &McpServerConfig) -> Result<(), String> {
        if self.connections.contains_key(&server.name) {
            return Ok(());
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

        self.connections
            .insert(server.name.clone(), McpConnection { service });
        Ok(())
    }

    /// Return Ollama-style tool definitions for every reachable server,
    /// namespaced as `mcp__<server>__<tool>`. Results are cached after the
    /// first successful fetch.
    pub async fn tool_definitions(&mut self) -> Vec<Value> {
        if let Some(cached) = &self.defs_cache {
            return cached.clone();
        }

        let mut defs = Vec::new();
        for server in self.config.usable_servers() {
            if let Err(e) = self.ensure_connected(&server).await {
                warn!("{e}");
                continue;
            }
            let conn = match self.connections.get(&server.name) {
                Some(c) => c,
                None => continue,
            };
            match conn.service.list_all_tools().await {
                Ok(tools) => {
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
                Err(e) => warn!("mcp: tools/list failed for server '{}': {e}", server.name),
            }
        }

        self.defs_cache = Some(defs.clone());
        defs
    }

    /// Call an upstream tool on a server. `server`/`tool` are the un-namespaced
    /// parts (see [`super::naming::parse`]).
    pub async fn call_tool(
        &mut self,
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

        self.ensure_connected(&server).await?;

        let conn = self
            .connections
            .get(server_name)
            .ok_or_else(|| format!("mcp: server '{server_name}' is not connected"))?;

        let mut params = CallToolRequestParams::new(tool_name.to_owned());
        if let Some(obj) = args.as_object() {
            params = params.with_arguments(obj.clone());
        }

        let result = tokio::time::timeout(CALL_TIMEOUT, conn.service.call_tool(params))
            .await
            .map_err(|_| {
                format!("mcp: tool '{tool_name}' on server '{server_name}' timed out after {CALL_TIMEOUT:?}")
            })?
            .map_err(|e| format!("mcp: tool call '{tool_name}' failed: {e}"))?;

        Ok(result_to_string(&result))
    }
}

/// Flatten a `tools/call` result into the plain String the tool dispatcher
/// expects. Text blocks are concatenated; non-text blocks get a placeholder.
/// When the server flagged the result as an error, the text is marked as such.
fn result_to_string(result: &CallToolResult) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(structured) = &result.structured_content {
        parts.push(
            serde_json::to_string_pretty(structured)
                .unwrap_or_else(|_| structured.to_string()),
        );
    }

    for block in &result.content {
        match block {
            ContentBlock::Text(t) => parts.push(t.text.clone()),
            ContentBlock::Image(_) => parts.push("[mcp: image content omitted]".to_string()),
            ContentBlock::Audio(_) => parts.push("[mcp: audio content omitted]".to_string()),
            ContentBlock::Resource(_) => {
                parts.push("[mcp: embedded resource omitted]".to_string())
            }
            ContentBlock::ResourceLink(r) => {
                parts.push(format!("[mcp: resource link: {}]", r.uri))
            }
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

    #[tokio::test]
    async fn definitions_empty_when_server_cannot_spawn() {
        let mut mgr = McpManager::new(bad_server_config());
        let defs = mgr.tool_definitions().await;
        assert!(defs.is_empty());
    }

    #[tokio::test]
    async fn call_reports_spawn_failure_as_error() {
        let mut mgr = McpManager::new(bad_server_config());
        let err = mgr
            .call_tool("ghost", "whatever", &Value::Null)
            .await
            .expect_err("must fail");
        assert!(err.contains("ghost"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn call_unknown_server_is_an_error() {
        let mut mgr = McpManager::new(McpConfig::default());
        let err = mgr
            .call_tool("nope", "whatever", &Value::Null)
            .await
            .expect_err("must fail");
        assert!(err.contains("nope"), "unexpected error: {err}");
    }

    #[test]
    fn result_to_string_extracts_text() {
        use rmcp::model::{CallToolResult, ContentBlock, TextContent};
        let result =
            CallToolResult::success(vec![ContentBlock::Text(TextContent::new("hello"))]);
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
        let mut mgr = McpManager::new(McpConfig {
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
