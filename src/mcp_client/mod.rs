//! Generic MCP (Model Context Protocol) client for Helix.
//!
//! Spawns external MCP servers (e.g. the standalone `google-mcp`
//! Gmail/Calendar server) as subprocesses over stdio and exposes their tools
//! to the agent as `mcp__<server>__<tool>`.
//!
//! This module is intentionally named `mcp_client`: `src/mcp/` is already
//! taken by the "Master Control Program" (the internal bus router) and the
//! two are unrelated.
//!
//! When `[mcp] enabled = false` (the default) or no `config.toml` is found,
//! every entry point below is a silent no-op.

pub mod manager;
pub mod naming;

pub use naming::PREFIX;

use std::sync::OnceLock;

use log::warn;
use serde_json::Value;
use tokio::sync::Mutex as TokioMutex;

use crate::config::mcp::McpConfig;
use manager::McpManager;

/// Lazily parsed `[mcp]` config, read once from the config file Helix actually
/// loaded at startup (resolved by main.rs; see `set_config_path`). Falls back
/// to `./config.toml` when main.rs never set a path (unit tests, other entry
/// points) — which used to be the only place it ever looked, so `[mcp]` was
/// invisible whenever Helix ran off /etc/helix/config.toml.
static MCP_CONFIG: OnceLock<McpConfig> = OnceLock::new();

/// The config file Helix loaded, set once by main.rs right after config
/// resolution. The MCP client must read `[mcp]` from the LIVE file, for the
/// same reason the web editor must write to it.
static CONFIG_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Record the config file Helix loaded at startup. Call once from main.rs;
/// later calls are ignored (first one wins, like the OnceLock contract).
pub fn set_config_path(p: std::path::PathBuf) {
    let _ = CONFIG_PATH.set(p);
}

/// Dedicated multi-threaded runtime for ALL MCP I/O. Keeping every MCP
/// future (spawning, handshakes, stdio) on one runtime means child-process
/// pipes stay bound to a single reactor for the whole process lifetime.
static MCP_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Global connection manager, created on first use when MCP is enabled.
static MCP_MANAGER: OnceLock<std::sync::Arc<TokioMutex<McpManager>>> = OnceLock::new();

fn load_config_from(path: &std::path::Path) -> McpConfig {
    std::fs::read_to_string(path)
        .map(|s| McpConfig::load_from_toml(&s))
        .unwrap_or_default()
}

fn load_config() -> McpConfig {
    let path = CONFIG_PATH
        .get()
        .map(|p| p.as_path())
        .unwrap_or_else(|| std::path::Path::new("config.toml"));
    load_config_from(path)
}

/// The parsed `[mcp]` config (process-wide, loaded once).
pub fn config() -> &'static McpConfig {
    MCP_CONFIG.get_or_init(load_config)
}

/// True when the `[mcp]` section enables the client.
pub fn is_enabled() -> bool {
    config().is_enabled()
}

fn runtime() -> &'static tokio::runtime::Runtime {
    MCP_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("helix-mcp")
            .build()
            .expect("mcp_client: cannot build tokio runtime")
    })
}

/// Drive an MCP future to completion from synchronous code.
///
/// `tools::execute()` / `tools::tool_definitions()` are called from both
/// plain sync code and async contexts (axum handlers, the Ollama tool loop).
/// A fresh `tokio::runtime::Runtime::new().block_on(..)` — the pattern used
/// by `call_gemini_sync` — panics when called from inside an existing
/// runtime, which is exactly where `execute()` usually runs. Instead we keep
/// one dedicated runtime for MCP and enter it with `block_on` when there is
/// no ambient runtime, or `block_in_place` when there is one.
fn bridge<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let rt = runtime();
    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => tokio::task::block_in_place(|| rt.block_on(fut)),
        Err(_) => rt.block_on(fut),
    }
}

fn manager() -> Option<std::sync::Arc<TokioMutex<McpManager>>> {
    if !is_enabled() {
        return None;
    }
    Some(
        MCP_MANAGER
            .get_or_init(|| {
                std::sync::Arc::new(TokioMutex::new(McpManager::new(config().clone())))
            })
            .clone(),
    )
}

/// Ollama-style tool definitions for every MCP tool (empty when disabled).
/// Results are cached after the first fetch; servers are spawned lazily.
pub fn tool_definitions() -> Vec<Value> {
    let mgr = match manager() {
        Some(m) => m,
        None => return Vec::new(),
    };
    bridge(async move {
        let mut guard = mgr.lock().await;
        guard.tool_definitions().await
    })
}

/// Try to execute an MCP tool.
///
/// Returns `None` when `name` is not an MCP tool name or MCP is disabled
/// (the caller should then fall through to other tool sources). Otherwise
/// returns `Some(output)`, where the output may itself describe a failure.
pub fn try_execute(name: &str, args: &Value) -> Option<String> {
    let (server, tool) = naming::parse(name)?;
    let mgr = manager()?;
    let args = args.clone();
    Some(bridge(async move {
        let mut guard = mgr.lock().await;
        match guard.call_tool(&server, &tool, &args).await {
            Ok(out) => out,
            Err(e) => {
                warn!("{e}");
                e
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: these tests rely on the worktree's config.toml leaving the
    // example `[mcp]` section commented out, i.e. MCP disabled. No test in
    // this module enables the global client.

    #[test]
    fn disabled_by_default_from_worktree_config() {
        assert!(!is_enabled());
    }

    #[test]
    fn tool_definitions_empty_when_disabled() {
        assert!(tool_definitions().is_empty());
    }

    #[test]
    fn try_execute_returns_none_for_plain_tools() {
        assert_eq!(try_execute("read_log", &Value::Null), None);
    }

    #[test]
    fn try_execute_returns_none_for_mcp_names_when_disabled() {
        // Well-formed MCP name, but the client is disabled -> fall through.
        assert_eq!(
            try_execute("mcp__google__gmail_search", &Value::Null),
            None
        );
    }

    #[test]
    fn tools_mod_definitions_have_no_mcp_tools_when_disabled() {
        let defs = crate::tools::tool_definitions();
        let arr = defs.as_array().expect("tool_definitions must be an array");
        assert!(
            !arr.iter().any(|d| d
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n.starts_with(PREFIX))
                .unwrap_or(false)),
            "no mcp__ tools may be advertised while MCP is disabled"
        );
    }

    // NOTE: load_config_from (not load_config) is tested here on purpose —
    // load_config reads the process-global OnceLock path, which must stay
    // unset so the tests above keep seeing the worktree's disabled config.

    #[test]
    fn load_config_from_reads_mcp_section_at_given_path() {
        // GIVEN a config file with [mcp] enabled, at an arbitrary path:
        let dir = std::env::temp_dir().join("helix-mcp-path-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("custom-config.toml");
        std::fs::write(
            &path,
            "[mcp]\nenabled = true\n\n[[mcp.servers]]\nname = \"google\"\ncommand = \"/bin/true\"\n",
        )
        .unwrap();

        // WHEN loading from that path explicitly,
        // THEN the [mcp] section is honored:
        let cfg = load_config_from(&path);
        assert!(cfg.is_enabled());
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].name, "google");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_config_from_missing_file_yields_disabled_default() {
        // GIVEN a path that does not exist,
        // WHEN loading from it,
        // THEN we get the disabled default instead of a panic:
        let cfg = load_config_from(std::path::Path::new(
            "/nonexistent-dir-xyz/helix-test-config.toml",
        ));
        assert!(!cfg.is_enabled());
    }
}
