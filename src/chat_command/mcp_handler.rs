//! MCP tool-call path (task 319, spec `chat-command-mcp-spec.md`).
//!
//! `how: "mcp"` invokes one MCP tool on one configured server and posts
//! the result back to the `command` room. v1 is READ-ONLY:
//!
//! - The envelope requires `to` (parser-enforced — a broadcast MCP
//!   call would run the same tool twice), `server`, `tool`, and
//!   `scope: "read"`. Any `write:` scope is rejected at parse time.
//! - The server must be configured in `[mcp.servers]`; the tool must
//!   be offered by that server (checked against `tool_definitions()`);
//!   and the tool must be on that server's read-tool allowlist below.
//!   All three checks run BEFORE any call, and a failure posts a
//!   rejection naming the problem and executes nothing.
//! - The call runs in the tick (sync) within the `time` budget
//!   (default 60 s). The budget bounds how long the tick WAITS for
//!   the call — it does NOT kill the MCP server child. The executor
//!   (`mcp_client`) has no kill handle, so a runaway call keeps
//!   running in the background while the tick moves on (its result
//!   is dropped). A real kill is a follow-up, not this build.
//!
//! The read-tool allowlist is a static table of tool names confirmed
//! in-repo (task_handler's `GMAIL_READ_TOOLS`, the security allowlist).
//! A configured server whose tools are not listed here cannot be
//! called via `how: "mcp"` in v1 — that is fail-closed by design, and
//! adding a new read tool is a one-line edit to `read_tools_for`.

use super::reply;
use super::{ChatCommandConfig, ResolvedChatTools};
use std::sync::mpsc;
use std::time::Duration;

/// Default MCP time budget in seconds (spec §2: mcp 60).
pub const DEFAULT_MCP_TIME_SECS: u64 = 60;

/// Read-only tools per MCP server (upstream tool names, WITHOUT the
/// `mcp__<server>__` prefix). Only these may run via `how: "mcp"` in
/// v1. Anything else on a configured server is rejected as "not a
/// read tool" — the allowlist, not the tool name, decides.
///
/// - `google` (google-mcp, Gmail/Calendar): `gmail_search` and
///   `gmail_read` are the confirmed read tools (`GMAIL_READ_TOOLS` in
///   task_handler); `calendar_list` is a read tool per the security
///   allowlist. `gmail_send` / `gmail_modify` / `gmail_label` /
///   `gmail_archive` are writes and are NOT listed.
/// - `chat` (chat-mcp, the hub): `chat_read`, `chat_rooms`,
///   `chat_health` are read-only. `chat_send` posts a message (a
///   write) and `chat_watch` is a long-poll, not a single call —
///   neither is listed.
/// - Any other server: no read tools known in v1 (fail closed).
pub fn read_tools_for(server: &str) -> &'static [&'static str] {
    match server {
        "google" => &["gmail_search", "gmail_read", "calendar_list"],
        "chat" => &["chat_read", "chat_rooms", "chat_health"],
        _ => &[],
    }
}

/// True when `tool` is on `server`'s read-tool allowlist.
pub fn is_read_tool(server: &str, tool: &str) -> bool {
    read_tools_for(server).contains(&tool)
}

/// True when `server` is a usable entry in `[mcp.servers]` AND the
/// MCP client is enabled. A disabled client has no servers at all,
/// so every server is "unknown" (rejection, not a silent no-op).
fn server_configured(server: &str) -> bool {
    if !crate::mcp_client::is_enabled() {
        return false;
    }
    crate::mcp_client::config()
        .usable_servers()
        .iter()
        .any(|s| s.name == server)
}

/// Tool names the server currently offers, from `tool_definitions()`
/// (the MCP layer spawns servers lazily and caches the list, so this
/// is cheap after the first call — and `resolve_chat_tools` already
/// pays it every tick). Names are the upstream tool part (the
/// `mcp__<server>__` prefix is stripped).
fn offered_tools(server: &str) -> Vec<String> {
    let prefix = crate::mcp_client::naming::tool_name(server, "");
    crate::mcp_client::tool_definitions()
        .iter()
        .filter_map(|d| {
            d.get("function")?
                .get("name")?
                .as_str()
                .map(str::to_string)
        })
        .filter(|n| n.starts_with(&prefix))
        .filter_map(|n| crate::mcp_client::naming::parse(&n).map(|(_, tool)| tool))
        .collect()
}

/// Outcome of a bounded MCP call.
#[derive(Debug)]
pub enum McpCallOutcome {
    /// The tool call returned. `None` means `try_execute` had nothing
    /// to run (MCP disabled / bad name) — the caller treats it as a
    /// failure report, not a result. The `String` may itself be an
    /// MCP error (classified by the caller via `is_tool_error`).
    Completed(Option<String>),
    /// The wait exceeded the budget. The worker thread keeps running
    /// (the child is NOT killed — see module docs); the tick moves on.
    TimedOut,
}

/// Run `f` on a worker thread and wait at most `budget` for it.
/// Returns `Completed` with `f`'s output, or `TimedOut` when the
/// budget expires first. The worker is NOT killed on timeout — it
/// finishes in the background and its result is dropped (the channel
/// send fails silently once the receiver is gone).
fn call_with_timeout<F>(f: F, budget: Duration) -> McpCallOutcome
where
    F: FnOnce() -> Option<String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = f();
        let _ = tx.send(out);
    });
    match rx.recv_timeout(budget) {
        Ok(out) => McpCallOutcome::Completed(out),
        Err(_) => McpCallOutcome::TimedOut,
    }
}

/// True when a tool-call output is an MCP-layer error rather than a
/// result. Mirrors `reply::is_send_error`: the MCP layer prefixes
/// tool errors with "MCP tool reported an error:" and the manager
/// prefixes its own failures with "mcp:". A genuine tool result is
/// never prefixed that way, so the classification is safe.
fn is_tool_error(output: &str) -> bool {
    let t = output.trim_start();
    t.starts_with("MCP tool reported an error:") || t.starts_with("mcp:")
}

/// Cap `output` at `max_bytes` on a char boundary, like the command
/// path's `format_reply`. Returns (capped, was_truncated).
fn cap_output(output: &str, max_bytes: usize) -> (String, bool) {
    if output.len() <= max_bytes {
        return (output.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    (output[..end].to_string(), true)
}

/// Report header for an MCP call, citing the task's hub message id
/// (so the sender can correlate call → result). Never starts with
/// `!` (so a report can never retrigger the routine).
fn report_header(hub_id: i64, server: &str, tool: &str, kind: &str) -> String {
    format!("mcp call #{hub_id} {kind} (server {server} tool {tool})")
}

/// Full report body for a completed call: header line, then the
/// tool's returned content (capped at `max_output_bytes`, with a
/// `[truncated]` note when the cap cut anything). Empty output
/// becomes `(no output)`. A tool-error output gets a `failed`
/// header; a real result gets a `result` header.
fn format_result(
    hub_id: i64,
    server: &str,
    tool: &str,
    output: &str,
    max_output_bytes: usize,
) -> String {
    let kind = if is_tool_error(output) { "failed" } else { "result" };
    let mut body = format!("{}\n", report_header(hub_id, server, tool, kind));
    if output.is_empty() {
        body.push_str("(no output)\n");
        return body;
    }
    let (capped, truncated) = cap_output(output, max_output_bytes);
    body.push_str(&capped);
    if !capped.ends_with('\n') {
        body.push('\n');
    }
    if truncated {
        body.push_str("[truncated]\n");
    }
    body
}

/// Timeout report body: header line only (no partial results — the
/// call's output is unavailable when the wait expires).
fn format_timeout(hub_id: i64, server: &str, tool: &str, time_secs: u64) -> String {
    format!(
        "mcp call #{hub_id} timed out after {time_secs}s (server {server} tool {tool})\n"
    )
}

/// Post one reply body (chunked, same room). Failures are logged by
/// the caller (a failed reply never re-executes anything).
fn post_body(tools: &ResolvedChatTools, cfg: &ChatCommandConfig, body: &str) {
    let chunks = reply::chunk_reply(body, cfg.chunk_bytes);
    if !reply::send_reply(tools, &cfg.room, &chunks) {
        log::warn!("chat-command: mcp report send abandoned (room={})", cfg.room);
    }
}

/// Audit-log one line for an MCP call, rejection, timeout, or report
/// (same log as commands).
fn audit(line: &str) {
    crate::utils::log_to_file(&format!("chat-command: mcp {line}"));
}

/// Run one validated `how: "mcp"` call and post exactly one reply
/// (result / failure / timeout / rejection). Always consumes a
/// command slot (`Slot::Command`) — rejections post a reply, so they
/// count, exactly like the parser's rejections.
///
/// Checks run in this order, each BEFORE any call:
/// 1. server configured in `[mcp.servers]` (else "unknown mcp
///    server '<name>'");
/// 2. tool offered by that server (else "unknown tool '<tool>' on
///    server '<server>'");
/// 3. tool on the server's read-tool allowlist (else "tool '<tool>'
///    is not a read tool").
/// Then the call runs on a worker thread within the `time` budget.
///
/// `pub(super)` — only `mod.rs` (`dispatch_structured`) calls it.
/// That also keeps the return type (`super::Slot`, private to the
/// parent) at least as visible as this function, so the
/// `private_interfaces` lint stays quiet.
pub(super) fn run_mcp_call(
    tools: &ResolvedChatTools,
    cfg: &ChatCommandConfig,
    hub_id: i64,
    server: &str,
    tool: &str,
    args: &serde_json::Value,
    time_secs: u64,
) -> super::Slot {
    if !server_configured(server) {
        let reason = format!("unknown mcp server '{server}'");
        audit(&format!("rejected hub_id={hub_id} reason={reason}"));
        super::send_text(tools, cfg, &format!("rejected: {reason}"));
        return super::Slot::Command;
    }
    if !offered_tools(server).iter().any(|t| t == tool) {
        let reason = format!("unknown tool '{tool}' on server '{server}'");
        audit(&format!("rejected hub_id={hub_id} reason={reason}"));
        super::send_text(tools, cfg, &format!("rejected: {reason}"));
        return super::Slot::Command;
    }
    if !is_read_tool(server, tool) {
        let reason = format!("tool '{tool}' is not a read tool");
        audit(&format!("rejected hub_id={hub_id} reason={reason}"));
        super::send_text(tools, cfg, &format!("rejected: {reason}"));
        return super::Slot::Command;
    }
    audit(&format!(
        "call hub_id={hub_id} server={server} tool={tool} budget={time_secs}s"
    ));
    let full_name = crate::mcp_client::naming::tool_name(server, tool);
    let args = args.clone();
    let outcome = call_with_timeout(
        move || crate::mcp_client::try_execute(&full_name, &args),
        Duration::from_secs(time_secs),
    );
    match outcome {
        McpCallOutcome::Completed(Some(output)) => {
            let body = format_result(hub_id, server, tool, &output, cfg.max_output_bytes);
            audit(&format!(
                "report hub_id={hub_id} server={server} tool={tool} bytes={}",
                output.len()
            ));
            post_body(tools, cfg, &body);
        }
        McpCallOutcome::Completed(None) => {
            // The checks above guarantee a runnable call, so None
            // means the client vanished mid-call — fail closed as a
            // failure report, never as an empty result.
            let body = format_result(
                hub_id,
                server,
                tool,
                "mcp: tool call unavailable (MCP client returned nothing)",
                cfg.max_output_bytes,
            );
            audit(&format!(
                "report hub_id={hub_id} server={server} tool={tool} unavailable"
            ));
            post_body(tools, cfg, &body);
        }
        McpCallOutcome::TimedOut => {
            audit(&format!(
                "timeout hub_id={hub_id} server={server} tool={tool} after={time_secs}s"
            ));
            post_body(
                tools,
                cfg,
                &format_timeout(hub_id, server, tool, time_secs),
            );
        }
    }
    super::Slot::Command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_allowlist_google_reads_only() {
        assert!(is_read_tool("google", "gmail_search"));
        assert!(is_read_tool("google", "gmail_read"));
        assert!(is_read_tool("google", "calendar_list"));
        // Writes are NOT read tools.
        for w in ["gmail_send", "gmail_modify", "gmail_label", "gmail_archive"] {
            assert!(!is_read_tool("google", w), "{w} must not be a read tool");
        }
        assert!(!is_read_tool("google", "gmail_delete"));
        assert!(!is_read_tool("google", "calendar_create"));
    }

    #[test]
    fn read_allowlist_chat_reads_only() {
        assert!(is_read_tool("chat", "chat_read"));
        assert!(is_read_tool("chat", "chat_rooms"));
        assert!(is_read_tool("chat", "chat_health"));
        // chat_send posts (a write); chat_watch is a long-poll.
        assert!(!is_read_tool("chat", "chat_send"));
        assert!(!is_read_tool("chat", "chat_watch"));
    }

    #[test]
    fn read_allowlist_unknown_server_is_empty() {
        assert!(!is_read_tool("mail", "anything"));
        assert!(!is_read_tool("", "gmail_search"));
        assert!(read_tools_for("nope").is_empty());
    }

    #[test]
    fn tool_error_classification_matches_send_errors() {
        assert!(is_tool_error("MCP tool reported an error:\nhub error: 429"));
        assert!(is_tool_error("mcp: tool call failed: timeout"));
        assert!(is_tool_error("  mcp: leading whitespace is trimmed"));
        assert!(!is_tool_error("{\"messages\": []}"));
        assert!(!is_tool_error("Search results:\n- a\n- b"));
        assert!(!is_tool_error(""));
    }

    #[test]
    fn result_report_header_and_capping() {
        let body = format_result(42, "google", "gmail_search", "hello\n", 7500);
        assert!(body.starts_with("mcp call #42 result (server google tool gmail_search)\n"), "{body:?}");
        assert!(body.contains("hello\n"), "{body:?}");
        assert!(!body.starts_with('!'), "reports must never start with '!'");
        // Empty output.
        let body = format_result(7, "google", "gmail_search", "", 7500);
        assert!(body.contains("(no output)\n"), "{body:?}");
        // Tool-error output gets a failed header.
        let body = format_result(7, "google", "gmail_search", "mcp: boom", 7500);
        assert!(body.starts_with("mcp call #7 failed (server google tool gmail_search)\n"), "{body:?}");
        // Cap: long output is truncated with a note, on a char boundary.
        let long = "x".repeat(100);
        let body = format_result(7, "google", "gmail_search", &long, 10);
        assert!(body.contains("[truncated]\n"), "{body:?}");
        assert!(!body.contains(&"x".repeat(100)), "capped body must not contain the full output");
    }

    #[test]
    fn timeout_report_names_budget_and_call() {
        let body = format_timeout(9, "google", "gmail_search", 60);
        assert_eq!(
            body,
            "mcp call #9 timed out after 60s (server google tool gmail_search)\n"
        );
        assert!(!body.starts_with('!'));
    }

    #[test]
    fn call_with_timeout_fast_call_completes() {
        let outcome = call_with_timeout(
            || Some("done".to_string()),
            Duration::from_millis(500),
        );
        assert!(matches!(outcome, McpCallOutcome::Completed(Some(o)) if o == "done"));
        let outcome = call_with_timeout(|| None, Duration::from_millis(500));
        assert!(matches!(outcome, McpCallOutcome::Completed(None)));
    }

    #[test]
    fn call_with_timeout_slow_call_times_out() {
        // A closure that sleeps past the budget must return TimedOut
        // promptly (the worker keeps sleeping in the background; the
        // test only asserts the waiter moves on).
        let start = std::time::Instant::now();
        let outcome = call_with_timeout(
            || {
                std::thread::sleep(Duration::from_secs(5));
                Some("too late".to_string())
            },
            Duration::from_millis(100),
        );
        assert!(matches!(outcome, McpCallOutcome::TimedOut));
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "timeout must bound the wait, not the worker: {:?}",
            start.elapsed()
        );
    }
}
