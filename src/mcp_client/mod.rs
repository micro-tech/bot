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

use std::sync::OnceLock;
use std::time::Duration;

use log::{debug, warn};
use serde_json::Value;

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
/// An `Arc<McpManager>` (not a mutex): the manager's maps are behind
/// short-held interior locks that are never held across network I/O
/// (task 397 / M7), so no global lock is ever held across a call and a
/// hung server cannot block other servers.
static MCP_MANAGER: OnceLock<std::sync::Arc<McpManager>> = OnceLock::new();

/// Bridge timeout for a whole `tool_definitions` fetch (all servers).
/// The per-server connect (30 s) + list (30 s) timeouts normally fire
/// first; this bounds the aggregate so a multi-server fetch cannot stall
/// a caller indefinitely.
const DEFS_TIMEOUT: Duration = Duration::from_secs(120);
/// Bridge timeout for a single `try_execute` call. The manager's own
/// CALL_TIMEOUT (120 s) normally fires first with the clean error; this
/// is the backstop that guarantees the worker thread is released.
const CALL_BRIDGE_TIMEOUT: Duration = Duration::from_secs(150);

fn load_config_from(path: &std::path::Path) -> McpConfig {
    match std::fs::read_to_string(path) {
        Ok(s) => McpConfig::load_from_toml(&s),
        Err(e) => {
            // Task 265: a missing/unreadable config file used to silently
            // disable MCP. Say so — an operator who wrote [mcp] enabled
            // must not wonder why no MCP tools appear.
            warn!(
                "mcp: cannot read config file {}, MCP disabled: {e}",
                path.display()
            );
            McpConfig::default()
        }
    }
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
    // Task 255: tests must not depend on the developer's ambient config.toml.
    // John's live config has [mcp] enabled, which broke the old assumption
    // that the worktree config leaves the section commented out.
    #[cfg(test)]
    if let Some(c) = *test_config_override().lock().unwrap() {
        return c;
    }
    MCP_CONFIG.get_or_init(load_config)
}

// ── Task 256 test seams ──────────────────────────────────────────────
// Hermetic overrides so integration tests can stub MCP servers without
// spawning processes or depending on ambient config. Guarded by
// `test_env_lock()` in tests that use them (they are process-global).

/// Test seam: hermetic override for the global config.
#[cfg(test)]
static TEST_CONFIG_OVERRIDE: std::sync::Mutex<Option<&'static McpConfig>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
fn test_config_override() -> &'static std::sync::Mutex<Option<&'static McpConfig>> {
    &TEST_CONFIG_OVERRIDE
}

#[cfg(test)]
pub fn override_config_for_tests(cfg: McpConfig) {
    // Box::leak: one tiny intentional leak per override install; test-only.
    let leaked: &'static McpConfig = Box::leak(Box::new(cfg));
    *test_config_override().lock().unwrap() = Some(leaked);
}

/// Test seam: canned `tool_definitions()` output.
#[cfg(test)]
static TEST_DEFS_OVERRIDE: std::sync::Mutex<Option<Vec<Value>>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub fn override_tool_definitions_for_tests(defs: Vec<Value>) {
    *TEST_DEFS_OVERRIDE.lock().unwrap() = Some(defs);
}

/// Test seam: canned `try_execute` outputs keyed by exact tool name.
/// Checked before any real manager/spawn work.
#[cfg(test)]
static TEST_TRY_EXECUTE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Test seam: ordered log of every `try_execute` call (tool name + args).
/// Lets integration tests assert a tool was (or was not) invoked, and with
/// what arguments. Drained via `take_try_execute_calls_for_tests`.
#[cfg(test)]
static TEST_TRY_EXECUTE_CALLS: std::sync::Mutex<Vec<(String, Value)>> =
    std::sync::Mutex::new(Vec::new());

/// Drain the recorded `try_execute` call log.
#[cfg(test)]
pub fn take_try_execute_calls_for_tests() -> Vec<(String, Value)> {
    std::mem::take(&mut *TEST_TRY_EXECUTE_CALLS.lock().unwrap())
}

#[cfg(test)]
pub fn stub_try_execute_for_tests(tool_name: &str, output: &str) {
    TEST_TRY_EXECUTE
        .lock()
        .unwrap()
        .insert(tool_name.to_string(), output.to_string());
}

/// Clear all test overrides. Tests that install overrides must call this
/// (holding `test_env_lock`) when done so later tests see a clean slate.
#[cfg(test)]
pub fn clear_test_overrides() {
    TEST_TRY_EXECUTE.lock().unwrap().clear();
    TEST_TRY_EXECUTE_CALLS.lock().unwrap().clear();
    *TEST_DEFS_OVERRIDE.lock().unwrap() = None;
    *test_config_override().lock().unwrap() = None;
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

/// Drive an MCP future to completion from synchronous code, bounded by
/// `timeout`.
///
/// `tools::execute()` / `tools::tool_definitions()` are called from both
/// plain sync code and async contexts (axum handlers, the Ollama tool loop).
/// A fresh `tokio::runtime::Runtime::new().block_on(..)` — the pattern used
/// by `call_gemini_sync` — panics when called from inside an existing
/// runtime, which is exactly where `execute()` usually runs. Instead we keep
/// one dedicated runtime for MCP and enter it with `block_on` when there is
/// no ambient runtime, or `block_in_place` when there is one.
///
/// The timeout (task 397 / M7) is the backstop that actually cancels a hung
/// call: `tokio::time::timeout` drops the future when it fires, so the
/// worker thread is released instead of leaking forever (the old bridge
/// had no timeout, so a hung call held the thread — and the global manager
/// mutex — indefinitely). The MCP child itself is NOT killed (see
/// mcp_handler docs — a real kill is a follow-up).
fn bridge<F, T>(fut: F, timeout: Duration) -> Result<T, String>
where
    F: std::future::Future<Output = T>,
{
    let rt = runtime();
    let timed = async move {
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| format!("mcp: operation timed out after {timeout:?}"))
    };
    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => tokio::task::block_in_place(|| rt.block_on(timed)),
        Err(_) => rt.block_on(timed),
    }
}

fn manager() -> Option<std::sync::Arc<McpManager>> {
    if !is_enabled() {
        // debug!, not warn!: this fires on every tool_definitions() /
        // try_execute() call while MCP is off, and off is the normal
        // state for a config without [mcp] enabled = true. A broken
        // config that *should* have enabled MCP is caught louder, at
        // load time (load_config_from / load_section warn!).
        debug!("mcp: client disabled, no manager (no [mcp] enabled = true)");
        return None;
    }
    Some(
        MCP_MANAGER
            .get_or_init(|| std::sync::Arc::new(McpManager::new(config().clone())))
            .clone(),
    )
}

/// Ollama-style tool definitions for every MCP tool (empty when disabled).
/// Results are cached after the first fetch; servers are spawned lazily.
/// A bridge timeout returns an empty list (and a warning) rather than
/// stalling the caller — the manager's per-server timeouts normally fire
/// first, and an empty fetch is not cached, so the next call retries.
pub fn tool_definitions() -> Vec<Value> {
    #[cfg(test)]
    if let Some(d) = TEST_DEFS_OVERRIDE.lock().unwrap().clone() {
        return d;
    }
    let mgr = match manager() {
        Some(m) => m,
        None => return Vec::new(),
    };
    match bridge(async move { mgr.tool_definitions().await }, DEFS_TIMEOUT) {
        Ok(defs) => defs,
        Err(e) => {
            warn!("{e}");
            Vec::new()
        }
    }
}

/// Try to execute an MCP tool.
///
/// Returns `None` when `name` is not an MCP tool name or MCP is disabled
/// (the caller should then fall through to other tool sources). Otherwise
/// returns `Some(output)`, where the output may itself describe a failure.
pub fn try_execute(name: &str, args: &Value) -> Option<String> {
    #[cfg(test)]
    TEST_TRY_EXECUTE_CALLS
        .lock()
        .unwrap()
        .push((name.to_string(), args.clone()));
    #[cfg(test)]
    if let Some(out) = TEST_TRY_EXECUTE.lock().unwrap().get(name) {
        return Some(out.clone());
    }
    let (server, tool) = naming::parse(name)?;
    let mgr = manager()?;
    let args = args.clone();
    let name = name.to_string();
    match bridge(
        async move { mgr.call_tool(&server, &tool, &args).await },
        CALL_BRIDGE_TIMEOUT,
    ) {
        Ok(Ok(out)) => Some(out),
        Ok(Err(e)) => {
            warn!("{e}");
            Some(e)
        }
        Err(e) => {
            // Bridge timeout: the call was actually cancelled (task 397 /
            // M7) — report it as a failure, not a silent fall-through.
            warn!("{e}");
            Some(format!("mcp: call to '{name}' timed out"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_client::naming::PREFIX;

    // NOTE: these tests install a hermetic disabled config via
    // override_config_for_tests — they never depend on the developer's
    // ambient config.toml (task 255). No test in this module enables the
    // global client.

    /// Install the hermetic disabled config. Returns the test-env lock
    /// guard: overrides are process-global (task 256 seams), so tests that
    /// install them must serialize. Bind the return value (`let _lock =`)
    /// to hold it for the whole test body.
    fn disabled_test_config() -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::tools::subagent_tool::test_env_lock();
        clear_test_overrides();
        override_config_for_tests(McpConfig::default());
        guard
    }

    #[test]
    fn bridge_timeout_cancels_hung_future() {
        // Task 397 / M7: a future that never completes must be cancelled
        // at the timeout (the worker thread is released), not awaited
        // forever. The old bridge had no timeout, so a hung call leaked
        // the thread indefinitely.
        let start = std::time::Instant::now();
        let out: Result<(), String> = bridge(
            async { std::future::pending::<()>().await },
            Duration::from_millis(100),
        );
        assert!(out.is_err(), "hung future must time out");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "bridge must return promptly on timeout, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn bridge_returns_fast_future() {
        let out: Result<u32, String> = bridge(async { 42 }, Duration::from_secs(5));
        assert_eq!(out, Ok(42));
    }

    #[test]
    fn disabled_config_is_not_enabled() {
        let _lock = disabled_test_config();
        assert!(!is_enabled());
    }

    #[test]
    fn tool_definitions_empty_when_disabled() {
        let _lock = disabled_test_config();
        assert!(tool_definitions().is_empty());
    }

    #[test]
    fn try_execute_returns_none_for_plain_tools() {
        assert_eq!(try_execute("read_log", &Value::Null), None);
    }

    #[test]
    fn try_execute_returns_none_for_mcp_names_when_disabled() {
        let _lock = disabled_test_config();
        // Well-formed MCP name, but the client is disabled -> fall through.
        assert_eq!(try_execute("mcp__google__gmail_search", &Value::Null), None);
    }

    #[test]
    fn tools_mod_definitions_have_no_mcp_tools_when_disabled() {
        let _lock = disabled_test_config();
        // tools::tool_definitions() is cached (task 237) — rebuild so the
        // assertion sees the current (disabled) MCP state.
        crate::tools::invalidate_tool_definitions();
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
