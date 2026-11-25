//! Helix mini-browser (tasks 350+): a headless Firefox driven over
//! WebDriver BiDi, shared by the LLM and John — the "like Cobble jr's
//! browser" model.
//!
//! Layout:
//! - `bidi` — the thin BiDi seam (ALL thirtyfour BiDi types live there).
//! - `session` — the session manager + driver trait (real / fake).
//! - `tool` — the `browser` tool facade.
//!
//! The browser is a long-lived service like `mcp_client`: one shared
//! session, started lazily on first use, driven on ONE dedicated tokio
//! runtime (`BROWSER_RT`) so the thirtyfour driver (geckodriver child
//! process, BiDi WebSocket) stays bound to a single reactor for the
//! whole process lifetime. Sync callers (the `browser` tool, the web
//! handlers) reach it through [`bridge`], the same pattern as
//! `mcp_client::bridge` (block_on with no ambient runtime, block_in_place
//! inside one).
//!
//! When `[browser] enabled = false` (the default), every entry point
//! below is a silent no-op / disabled message — no Firefox is spawned
//! and the `browser` tool is not usable.

pub mod bidi;
pub mod session;
pub mod tool;

use std::sync::OnceLock;

use log::debug;
use serde_json::Value;
use tokio::sync::Mutex as TokioMutex;

use crate::config::browser::BrowserConfig;
use session::BrowserSession;

/// The config file Helix loaded, set once by main.rs right after config
/// resolution. The browser reads `[browser]` from the LIVE file, for the
/// same reason the web editor must write to it.
static CONFIG_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Record the config file Helix loaded at startup. Call once from main.rs;
/// later calls are ignored (first one wins, like the OnceLock contract).
pub fn set_config_path(p: std::path::PathBuf) {
    let _ = CONFIG_PATH.set(p);
}

/// Lazily parsed `[browser]` config, read once from the config file Helix
/// actually loaded at startup (resolved by main.rs; see `set_config_path`).
/// Falls back to `./config.toml` when main.rs never set a path (unit
/// tests, other entry points).
static BROWSER_CONFIG: OnceLock<BrowserConfig> = OnceLock::new();

fn load_config_from(path: &std::path::Path) -> BrowserConfig {
    match std::fs::read_to_string(path) {
        Ok(s) => BrowserConfig::load_from_toml(&s),
        Err(e) => {
            debug!(
                "browser: cannot read config file {}, browser disabled: {e}",
                path.display()
            );
            BrowserConfig::default()
        }
    }
}

fn load_config() -> BrowserConfig {
    let path = CONFIG_PATH
        .get()
        .map(|p| p.as_path())
        .unwrap_or_else(|| std::path::Path::new("config.toml"));
    load_config_from(path)
}

/// The parsed `[browser]` config (process-wide, loaded once).
pub fn config() -> &'static BrowserConfig {
    #[cfg(test)]
    if let Some(c) = *test_config_override().lock().unwrap() {
        return c;
    }
    BROWSER_CONFIG.get_or_init(load_config)
}

/// Dedicated multi-threaded runtime for ALL browser I/O. Keeping every
/// browser future (session launch, BiDi, navigation) on one runtime means
/// the geckodriver child process and the BiDi WebSocket stay bound to a
/// single reactor for the whole process lifetime.
static BROWSER_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    BROWSER_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("helix-browser")
            .build()
            .expect("browser: cannot build tokio runtime")
    })
}

/// Drive a browser future to completion from synchronous code.
///
/// The `browser` tool and the web handlers are called from both plain
/// sync code and async contexts. A fresh `Runtime::new().block_on(..)`
/// panics when called from inside an existing runtime, which is exactly
/// where these usually run. Instead we keep one dedicated runtime and
/// enter it with `block_on` when there is no ambient runtime, or
/// `block_in_place` when there is one (same pattern as mcp_client).
pub(crate) fn bridge<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let rt = runtime();
    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => tokio::task::block_in_place(|| rt.block_on(fut)),
        Err(_) => rt.block_on(fut),
    }
}

/// Global browser session, created on first use when the browser is
/// enabled. The session itself is behind a tokio mutex so concurrent
/// tool calls serialize (one session, one action at a time).
static BROWSER_SESSION: OnceLock<std::sync::Arc<TokioMutex<BrowserSession>>> = OnceLock::new();

pub(crate) fn session_manager() -> Option<std::sync::Arc<TokioMutex<BrowserSession>>> {
    #[cfg(test)]
    if let Some(s) = test_session_override().lock().unwrap().clone() {
        return Some(s);
    }
    if !config().is_enabled() {
        debug!("browser: disabled, no session (no [browser] enabled = true)");
        return None;
    }
    Some(
        BROWSER_SESSION
            .get_or_init(|| {
                std::sync::Arc::new(TokioMutex::new(BrowserSession::new(config().clone())))
            })
            .clone(),
    )
}

/// Run one `browser` tool action against the shared session (sync entry
/// point for the tool registry). Returns the disabled message when the
/// browser is off.
pub fn execute_action(action: &str, args: &Value) -> String {
    let Some(mgr) = session_manager() else {
        return session::disabled_message();
    };
    let args = args.clone();
    let action = action.to_string();
    bridge(async move {
        let mut session = mgr.lock().await;
        match session.act(&action, &args).await {
            Ok(out) => out,
            Err(e) => e,
        }
    })
}

/// One-shot screenshot as PNG bytes (the 2FA affordance, task 355).
/// Returns the disabled message as an error when the browser is off.
pub fn screenshot_png() -> Result<Vec<u8>, String> {
    let Some(mgr) = session_manager() else {
        return Err(session::disabled_message());
    };
    bridge(async move {
        let mut session = mgr.lock().await;
        session.screenshot().await
    })
}

/// One 2FA input action (task 355). `kind` is "click" (x, y), "type"
/// (text), or "key" (key). Returns the disabled message as an error
/// when the browser is off.
pub fn browser_input(
    kind: &str,
    x: Option<i64>,
    y: Option<i64>,
    text: Option<String>,
    key: Option<String>,
) -> Result<String, String> {
    let Some(mgr) = session_manager() else {
        return Err(session::disabled_message());
    };
    let kind = kind.to_string();
    bridge(async move {
        let mut session = mgr.lock().await;
        session
            .input(&kind, x, y, text.as_deref(), key.as_deref())
            .await
    })
}

/// One-line browser status (never launches a browser).
pub fn browser_status() -> String {
    let Some(mgr) = session_manager() else {
        return session::disabled_message();
    };
    bridge(async move {
        let session = mgr.lock().await;
        session.status_text().await
    })
}

// ── Test seams ───────────────────────────────────────────────────────
// Hermetic overrides so tests can drive a fake session without launching
// Firefox or depending on ambient config. Guarded by `test_env_lock()`
// in tests that use them (they are process-global).

#[cfg(test)]
static TEST_CONFIG_OVERRIDE: std::sync::Mutex<Option<&'static BrowserConfig>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
fn test_config_override() -> &'static std::sync::Mutex<Option<&'static BrowserConfig>> {
    &TEST_CONFIG_OVERRIDE
}

#[cfg(test)]
pub fn override_config_for_tests(cfg: BrowserConfig) {
    // Box::leak: one tiny intentional leak per override install; test-only.
    let leaked: &'static BrowserConfig = Box::leak(Box::new(cfg));
    *test_config_override().lock().unwrap() = Some(leaked);
}

#[cfg(test)]
static TEST_SESSION_OVERRIDE: std::sync::Mutex<Option<std::sync::Arc<TokioMutex<BrowserSession>>>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
fn test_session_override()
-> &'static std::sync::Mutex<Option<std::sync::Arc<TokioMutex<BrowserSession>>>> {
    &TEST_SESSION_OVERRIDE
}

#[cfg(test)]
pub fn override_session_for_tests(session: BrowserSession) {
    *test_session_override().lock().unwrap() = Some(std::sync::Arc::new(TokioMutex::new(session)));
}

/// Clear all test overrides. Tests that install overrides must call this
/// (holding `test_env_lock`) when done so later tests see a clean slate.
#[cfg(test)]
pub fn clear_test_overrides() {
    *test_session_override().lock().unwrap() = None;
    *test_config_override().lock().unwrap() = None;
}
