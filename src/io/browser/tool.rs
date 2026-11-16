//! The `browser` tool (task 352): one tool, one `action` parameter.
//!
//! Thin facade over the browser session manager (`super::execute_action`).
//! One tool, not separate tools per action — one session, one schema,
//! keeps the tool count from ballooning and matches the one-session-
//! one-browser model. The weak desktop LLM gets a small surface to aim at.
//!
//! Actions: navigate, click, type, key, screenshot, get_text, current_url,
//! back, evaluate, gmail_oauth, gmail_oauth_check, restart, status.
//! click/type take a CSS "selector" OR x/y coordinates (the coordinate
//! form clicks to focus first, then types). See
//! [`super::session::BrowserSession::act`] for what each does.
//!
//! Security: navigation is confined to the `[browser] allowed_hosts`
//! allowlist (default Google + localhost); the tool never returns
//! password field contents (get_text strips them, evaluate refuses
//! password-touching scripts). The tool is a no-op with a disabled
//! message when `[browser] enabled = false`.

use serde_json::Value;

/// Execute the `browser` tool. Returns short text results.
pub fn browser_tool(args: &Value) -> String {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
    if action.is_empty() {
        return "browser needs an 'action' (navigate, click, type, key, screenshot, get_text, current_url, back, evaluate, gmail_oauth, gmail_oauth_check, restart, status)."
            .to_string();
    }
    super::execute_action(action, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::browser::BrowserConfig;
    use crate::io::browser::session::{BrowserSession, FakeDriver};
    use std::sync::Arc;

    /// Install an enabled config + a fake-driver session, returning the
    /// test-env lock guard. The overrides are process-global, so tests
    /// that install them must serialize (same pattern as mcp_client).
    fn fake_browser() -> (std::sync::MutexGuard<'static, ()>, Arc<FakeDriver>) {
        let guard = crate::tools::subagent_tool::test_env_lock();
        crate::io::browser::clear_test_overrides();
        crate::io::browser::override_config_for_tests(BrowserConfig {
            enabled: true,
            ..BrowserConfig::default()
        });
        let fake = Arc::new(FakeDriver::new());
        let session = BrowserSession::with_driver(
            BrowserConfig {
                enabled: true,
                ..BrowserConfig::default()
            },
            Arc::clone(&fake) as Arc<dyn crate::io::browser::session::BrowserDriver>,
        );
        crate::io::browser::override_session_for_tests(session);
        (guard, fake)
    }

    #[test]
    fn tool_navigate_returns_url_and_title() {
        let (_lock, _fake) = fake_browser();
        let out = browser_tool(&serde_json::json!({
            "action": "navigate",
            "url": "https://accounts.google.com/"
        }));
        assert!(out.contains("accounts.google.com"), "got: {out}");
        crate::io::browser::clear_test_overrides();
    }

    #[test]
    fn tool_navigate_refuses_non_allowlisted_host() {
        let (_lock, fake) = fake_browser();
        let out = browser_tool(&serde_json::json!({
            "action": "navigate",
            "url": "https://evil.example.com/"
        }));
        assert!(out.contains("allowlist"), "got: {out}");
        assert_eq!(
            fake.current_url_sync(),
            "about:blank",
            "refused navigation must not reach the driver"
        );
        crate::io::browser::clear_test_overrides();
    }

    #[test]
    fn tool_missing_action_lists_actions() {
        let (_lock, _fake) = fake_browser();
        let out = browser_tool(&serde_json::json!({}));
        assert!(out.contains("navigate"), "got: {out}");
        crate::io::browser::clear_test_overrides();
    }

    #[test]
    fn tool_disabled_config_reports_disabled() {
        let guard = crate::tools::subagent_tool::test_env_lock();
        crate::io::browser::clear_test_overrides();
        crate::io::browser::override_config_for_tests(BrowserConfig::default());
        let out = browser_tool(&serde_json::json!({
            "action": "navigate",
            "url": "https://accounts.google.com/"
        }));
        assert!(out.contains("disabled"), "got: {out}");
        crate::io::browser::clear_test_overrides();
        drop(guard);
    }
}
