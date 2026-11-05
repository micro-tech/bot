//! Chat-command loop (tasks 295-300, 304-305, 309-310).
//!
//! A watcher-style routine (`chat-command-watch`, every 1m) polls a
//! dedicated chat-hub room via chat-mcp, executes read-only diagnostic
//! commands posted there by authorized senders, and posts output back
//! to the same room. Phase 3 adds task handoff: a `!` JSON envelope
//! (`how: llm`) hands a task string to one of Helix's LLM backends as
//! an async agent job, with the report posted back to the same room.
//! Plain commands stay fully in-process (no LLM on that path), like
//! `support-mail-watch`; LLM tasks run detached, never blocking the
//! tick.
//!
//! Safety shape (load-bearing order):
//! 1. Sender gate FIRST - only `allowed_senders` (expected
//!    `"cobble-jr"`, `"john"`) may issue commands. Everyone else,
//!    including `"helix"` (loop prevention: Helix's own replies can
//!    never re-trigger), is ignored silently and audit-logged.
//! 2. Trigger: trimmed body must start with `!`.
//! 3. Envelope (Phase 3): a `!` rest that parses as JSON with a valid
//!    `how` dispatches structured (command -> executor with `time`
//!    override, llm -> task handler); anything else falls back to the
//!    plain command path, so hand-typed `!uptime` keeps working.
//!    Malformed structured messages get a rejection naming the
//!    problem and execute nothing.
//! 4. Target: `asus: ...` routes to the ASUS (Phase 2); anything else
//!    runs locally. ASUS disabled/unconfigured -> not-enabled reply,
//!    zero ssh attempts.
//! 5. Parse + allowlist + metachar screen, THEN execution via
//!    `std::process::Command` directly. No shell anywhere on this path
//!    (`run_shell` is deliberately not used - it allows writes).

pub mod asus;
pub mod config;
pub mod envelope;
pub mod executor;
pub mod reply;
pub mod state;
pub mod task_handler;

pub use config::ChatCommandConfig;
use state::{CommandState, StateLoad};

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Tick budget: stop taking new commands after ~45 s (rest wait for
/// the next tick). The 60 s tick interval plus a 30 s command timeout
/// means a tick could otherwise overlap the next one.
pub const TICK_BUDGET: Duration = Duration::from_secs(45);
/// Messages per `chat_read` page (hub clamps limit to 1..=200).
const READ_LIMIT: i64 = 200;

/// Resolved `mcp__chat__*` tool names.
#[derive(Debug, Clone)]
pub struct ResolvedChatTools {
    pub read: String,
    pub send: String,
}

/// Find the chat read/send tools among advertised `mcp__chat__*`
/// tools. Returns None when the chat server isn't configured or has
/// no tools (the tick then no-ops gracefully). Exact names first,
/// then a defensive contains-fallback mirroring support_watch's
/// `resolve_mail_tools` (a differently-named surface still yields
/// something to probe with — but BOTH tools must resolve).
pub fn resolve_chat_tools() -> Option<ResolvedChatTools> {
    let prefix = "mcp__chat__";
    let names: Vec<String> = crate::mcp_client::tool_definitions()
        .iter()
        .filter_map(|d| {
            d.get("function")?
                .get("name")?
                .as_str()
                .map(str::to_string)
        })
        .filter(|n| n.starts_with(prefix))
        .collect();
    if names.is_empty() {
        return None;
    }
    let pick = |exact: &str, contains: &str| {
        names
            .iter()
            .find(|n| n.as_str() == exact)
            .or_else(|| {
                names
                    .iter()
                    .find(|n| n.to_lowercase().contains(contains))
            })
            .cloned()
    };
    let read = pick("mcp__chat__chat_read", "read")?;
    let send = pick("mcp__chat__chat_send", "send")?;
    Some(ResolvedChatTools { read, send })
}

/// One hub message (envelope fields we act on).
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: i64,
    pub sender: String,
    pub body: String,
}

fn message_from_value(item: &serde_json::Value) -> Option<ChatMessage> {
    let id = item
        .get("id")
        .and_then(|i| i.as_i64().or_else(|| i.as_str().and_then(|s| s.parse().ok())))?;
    Some(ChatMessage {
        id,
        sender: item
            .get("sender")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        body: item
            .get("body")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// Parse a `chat_read` output into messages. The MCP layer returns
/// the structured value first (`{"messages": [...]}`) followed by the
/// text body (a JSON array), so we stream JSON values and take the
/// first array — or the first object carrying a `messages` array.
/// Anything unparseable yields no messages (never crash the tick).
pub fn parse_messages(output: &str) -> Vec<ChatMessage> {
    let stream = serde_json::Deserializer::from_str(output).into_iter::<serde_json::Value>();
    for value in stream {
        let Ok(v) = value else { continue };
        if let Some(arr) = v.as_array() {
            return arr.iter().filter_map(message_from_value).collect();
        }
        if let Some(arr) = v.get("messages").and_then(|m| m.as_array()) {
            return arr.iter().filter_map(message_from_value).collect();
        }
    }
    Vec::new()
}

/// Parse a `chat_rooms` output for one room's latest id.
/// Returns None when unparseable; Some(None) when the room is absent
/// or has no messages yet; Some(Some(id)) otherwise.
pub fn parse_rooms_latest(output: &str, room: &str) -> Option<Option<i64>> {
    let stream = serde_json::Deserializer::from_str(output).into_iter::<serde_json::Value>();
    for value in stream {
        let Ok(v) = value else { continue };
        let arr = if let Some(a) = v.as_array() {
            Some(a)
        } else {
            v.get("rooms").and_then(|r| r.as_array())
        };
        if let Some(arr) = arr {
            for entry in arr {
                if entry.get("name").and_then(|n| n.as_str()) == Some(room) {
                    return Some(entry.get("last_id").and_then(|i| i.as_i64()));
                }
            }
            return Some(None);
        }
    }
    None
}

/// Sender gate: the message's sender must be in the allowed set.
/// Anything else is ignored silently (no reply) — the caller logs it.
pub fn is_allowed_sender(sender: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|a| a == sender)
}

/// Trigger parse result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// Body does not start with `!` — ignored silently.
    Ignored,
    /// `!` with nothing after it — error reply.
    Empty,
    /// Command text after `!` (trimmed).
    Command(String),
}

/// Trigger: trimmed body must start with `!`.
pub fn parse_trigger(body: &str) -> Trigger {
    let t = body.trim();
    if !t.starts_with('!') {
        return Trigger::Ignored;
    }
    let rest = t[1..].trim();
    if rest.is_empty() {
        Trigger::Empty
    } else {
        Trigger::Command(rest.to_string())
    }
}

/// Execution target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Local,
    Asus,
}

/// Split a command rest into (target, command). Exact-lowercase
/// `asus:` prefix routes to the ASUS; anything else runs locally.
pub fn split_target(rest: &str) -> (Target, String) {
    if let Some(after) = rest.strip_prefix("asus:") {
        (Target::Asus, after.trim().to_string())
    } else {
        (Target::Local, rest.to_string())
    }
}

/// Truncate a string to at most `max` bytes on a char boundary.
/// Returns (truncated_string, was_truncated).
fn truncate_str(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

/// Format a command's reply body: echo line, then exit/timeout notes,
/// then output (stdout + `[stderr]` section), with a `[truncated]`
/// note when the output cap cut anything. Empty output becomes
/// `(no output)`.
fn format_reply(
    echo_prefix: &str,
    command_text: &str,
    outcome: &executor::ExecutionOutcome,
    timeout_secs: u64,
    max_output_bytes: usize,
) -> String {
    let mut body = format!("{echo_prefix} {command_text}\n");
    if outcome.timed_out {
        body.push_str(&format!("[TIMEOUT after {timeout_secs}s — process killed]\n"));
    } else if let Some(code) = outcome.exit_code {
        if code != 0 {
            body.push_str(&format!("[exit {code}]\n"));
        }
    } else if let Some(sig) = outcome.signal {
        body.push_str(&format!("[exit: killed by signal {sig}]\n"));
    }
    let mut output = outcome.stdout.clone();
    if !outcome.stderr.is_empty() {
        output.push_str("\n[stderr]\n");
        output.push_str(&outcome.stderr);
    }
    let mut truncated = outcome.stdout_truncated || outcome.stderr_truncated;
    let (capped, cap_truncated) = truncate_str(&output, max_output_bytes);
    truncated = truncated || cap_truncated;
    if output.is_empty() && !outcome.timed_out {
        body.push_str("(no output)\n");
    } else {
        body.push_str(&capped);
        if !capped.ends_with('\n') {
            body.push('\n');
        }
        if truncated {
            body.push_str("[truncated]\n");
        }
    }
    body
}

/// Whether processing this message consumed a command slot (replies
/// count; silent ignores do not).
enum Slot {
    Ignored,
    Command,
}

/// Process one message. Returns whether it consumed a command slot.
/// Replies are sent for executions, rejections, empty commands, and
/// not-enabled ASUS; unauthorized and non-`!` messages get no reply.
fn process_message(
    tools: &ResolvedChatTools,
    cfg: &ChatCommandConfig,
    runtime_dir: &Path,
    ssh_bin: &str,
    msg: &ChatMessage,
) -> Slot {
    // 1. Sender gate FIRST — before any parsing or execution.
    if !is_allowed_sender(&msg.sender, &cfg.allowed_senders) {
        log::info!(
            "chat-command: unauthorized sender '{}' ignored (room={})",
            msg.sender,
            cfg.room
        );
        return Slot::Ignored;
    }
    // 2. Trigger.
    let rest = match parse_trigger(&msg.body) {
        Trigger::Ignored => return Slot::Ignored,
        Trigger::Empty => {
            send_text(tools, &cfg, "empty command");
            return Slot::Command;
        }
        Trigger::Command(rest) => rest,
    };
    // 3. Structured envelope (Phase 3) or plain command fallback.
    // The sender gate already ran above, so a structured message
    // here is from an allowed sender. A rejection executes nothing.
    match envelope::parse(&rest) {
        envelope::Parsed::Structured(s) => {
            return dispatch_structured(tools, cfg, runtime_dir, ssh_bin, msg, s);
        }
        envelope::Parsed::Reject(reason) => {
            crate::utils::log_to_file(&format!(
                "chat-command: rejected structured sender={} reason={reason}",
                msg.sender
            ));
            send_text(tools, cfg, &format!("rejected: {reason}"));
            return Slot::Command;
        }
        envelope::Parsed::Plain(_) => {}
    }
    // 4. Target (plain path).
    let (target, command_text) = split_target(&rest);
    if command_text.is_empty() {
        send_text(tools, cfg, "empty command");
        return Slot::Command;
    }
    // 4a. ASUS target.
    if target == Target::Asus {
        if !cfg.asus_configured() {
            send_text(tools, cfg, &format!("asus$ {command_text}\nASUS relay not enabled/configured"));
            return Slot::Command;
        }
        let (host, user, key) = (
            cfg.asus_host.clone().unwrap_or_default(),
            cfg.asus_user.clone().unwrap_or_default(),
            cfg.asus_key_path.clone().unwrap_or_default(),
        );
        return run_and_reply(
            tools,
            cfg,
            &msg.sender,
            Target::Asus,
            &command_text,
            cfg.command_timeout_secs,
            || {
                let argv = executor::parse_command(&command_text)?;
                asus::check_asus_command(&argv)?;
                Ok(argv)
            },
            |words| {
                asus::run_asus_command(
                    ssh_bin,
                    &key,
                    &user,
                    &host,
                    words,
                    Duration::from_secs(cfg.command_timeout_secs),
                    cfg.max_output_bytes,
                )
            },
        );
    }
    // 4b. Local target.
    run_and_reply(
        tools,
        cfg,
        &msg.sender,
        Target::Local,
        &command_text,
        cfg.command_timeout_secs,
        || {
            let argv = executor::parse_command(&command_text)?;
            executor::check_local_command(&argv)?;
            Ok(argv)
        },
        |words| {
            executor::run_command_with_timeout(
                words,
                Duration::from_secs(cfg.command_timeout_secs),
                cfg.max_output_bytes,
            )
        },
    )
}

/// Shared execute→reply path for local and ASUS targets. `validate`
/// parses + allowlists (rejection → rejection reply, no execution);
/// `run` executes the validated argv.
/// Dispatch a validated structured envelope (Phase 3). Commands go
/// to the existing executor with the envelope's `time` override
/// (routing by `which`, NOT by an `asus:` prefix inside the command
/// string — the JSON command is verbatim, so a `which: local`
/// command starting with `asus:` fails the local allowlist rather
/// than routing). LLM tasks go to the handler. Scope was validated
/// as present + well-formed by the parser; for commands it does not
/// widen the read-only executor (a `write:` scope on a command still
/// runs the read-only allowlist).
fn dispatch_structured(
    tools: &ResolvedChatTools,
    cfg: &ChatCommandConfig,
    runtime_dir: &Path,
    ssh_bin: &str,
    msg: &ChatMessage,
    s: envelope::Structured,
) -> Slot {
    match s {
        envelope::Structured::Command {
            target,
            time_secs,
            scope: _,
            command,
        } => {
            let timeout_secs = time_secs.unwrap_or(cfg.command_timeout_secs);
            if target == Target::Asus {
                if !cfg.asus_configured() {
                    send_text(
                        tools,
                        cfg,
                        &format!("asus$ {command}\nASUS relay not enabled/configured"),
                    );
                    return Slot::Command;
                }
                let (host, user, key) = (
                    cfg.asus_host.clone().unwrap_or_default(),
                    cfg.asus_user.clone().unwrap_or_default(),
                    cfg.asus_key_path.clone().unwrap_or_default(),
                );
                return run_and_reply(
                    tools,
                    cfg,
                    &msg.sender,
                    Target::Asus,
                    &command,
                    timeout_secs,
                    || {
                        let argv = executor::parse_command(&command)?;
                        asus::check_asus_command(&argv)?;
                        Ok(argv)
                    },
                    |words| {
                        asus::run_asus_command(
                            ssh_bin,
                            &key,
                            &user,
                            &host,
                            words,
                            Duration::from_secs(timeout_secs),
                            cfg.max_output_bytes,
                        )
                    },
                );
            }
            run_and_reply(
                tools,
                cfg,
                &msg.sender,
                Target::Local,
                &command,
                timeout_secs,
                || {
                    let argv = executor::parse_command(&command)?;
                    executor::check_local_command(&argv)?;
                    Ok(argv)
                },
                |words| {
                    executor::run_command_with_timeout(
                        words,
                        Duration::from_secs(timeout_secs),
                        cfg.max_output_bytes,
                    )
                },
            )
        }
        envelope::Structured::Llm {
            backend,
            time_secs,
            scope,
            task,
        } => {
            task_handler::submit_llm_task(
                tools,
                cfg,
                runtime_dir,
                &msg.sender,
                msg.id,
                backend,
                time_secs.unwrap_or(task_handler::DEFAULT_LLM_TIME_SECS),
                scope,
                &task,
            );
            Slot::Command
        }
    }
}

fn run_and_reply(
    tools: &ResolvedChatTools,
    cfg: &ChatCommandConfig,
    sender: &str,
    target: Target,
    command_text: &str,
    timeout_secs: u64,
    validate: impl FnOnce() -> Result<Vec<String>, String>,
    run: impl FnOnce(&[String]) -> std::io::Result<executor::ExecutionOutcome>,
) -> Slot {
    let target_name = match target {
        Target::Local => "local",
        Target::Asus => "asus",
    };
    let echo_prefix = match target {
        Target::Local => "$",
        Target::Asus => "asus$",
    };
    let argv = match validate() {
        Ok(argv) => argv,
        Err(reason) => {
            crate::utils::log_to_file(&format!(
                "chat-command: rejected sender={sender} target={target_name} cmd={command_text} reason={reason}"
            ));
            send_text(tools, cfg, &format!("{echo_prefix} {command_text}\n{reason}"));
            return Slot::Command;
        }
    };
    let outcome = match run(&argv) {
        Ok(outcome) => outcome,
        Err(e) => {
            crate::utils::log_to_file(&format!(
                "chat-command: spawn failed sender={sender} target={target_name} cmd={command_text} err={e}"
            ));
            send_text(tools, cfg, &format!("{echo_prefix} {command_text}\nspawn failed: {e}"));
            return Slot::Command;
        }
    };
    crate::utils::log_to_file(&format!(
        "chat-command: sender={sender} target={target_name} cmd={command_text} exit={} ms={} bytes={}",
        outcome
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| if outcome.timed_out { "timeout".to_string() } else { "signal".to_string() }),
        outcome.duration_ms,
        outcome.stdout.len() + outcome.stderr.len()
    ));
    let body = format_reply(
        echo_prefix,
        command_text,
        &outcome,
        timeout_secs,
        cfg.max_output_bytes,
    );
    send_text(tools, cfg, &body);
    Slot::Command
}

/// Chunk one reply body and send it. Failures are logged by
/// `reply::send_reply` (abandoned chunks); the cursor still advances
/// (a failed reply must never cause a duplicate execution).
fn send_text(tools: &ResolvedChatTools, cfg: &ChatCommandConfig, body: &str) {
    let chunks = reply::chunk_reply(body, cfg.chunk_bytes);
    if !reply::send_reply(tools, &cfg.room, &chunks) {
        log::warn!("chat-command: reply send abandoned (room={})", cfg.room);
    }
}

/// Read one page via `chat_read`. Failures yield an empty page (the
/// tick ends; the next minute retries). Never propagates.
fn read_page(tools: &ResolvedChatTools, room: &str, since: i64) -> Vec<ChatMessage> {
    match crate::mcp_client::try_execute(
        &tools.read,
        &serde_json::json!({"room": room, "since": since, "limit": READ_LIMIT}),
    ) {
        Some(out) => parse_messages(&out),
        None => {
            log::warn!("chat-command: chat_read returned nothing (room={room})");
            Vec::new()
        }
    }
}

/// Seed the cursor to the room's latest id (no replay). The rooms
/// tool is resolved fresh (it is not part of the read/send pair).
fn seed_cursor(room: &str) -> Option<i64> {
    let rooms_tool = crate::mcp_client::tool_definitions()
        .iter()
        .filter_map(|d| {
            d.get("function")?
                .get("name")?
                .as_str()
                .map(str::to_string)
        })
        .find(|n| n == "mcp__chat__chat_rooms")?;
    let out = crate::mcp_client::try_execute(&rooms_tool, &serde_json::json!({}))?;
    match parse_rooms_latest(&out, room) {
        Some(latest) => Some(latest.unwrap_or(0)),
        None => {
            log::warn!("chat-command: chat_rooms output unparseable, cannot seed cursor");
            None
        }
    }
}

/// Tick in-progress guard. A tick that overlaps a still-running tick
/// skips (the budget makes overlap rare, but a 30 s command timeout
/// plus sends can push a tick past the 60 s interval).
static TICK_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

struct TickGuard;
impl Drop for TickGuard {
    fn drop(&mut self) {
        TICK_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

fn try_acquire_tick_guard() -> Option<TickGuard> {
    if TICK_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        Some(TickGuard)
    } else {
        None
    }
}

/// One routine tick. Production entry point (ssh binary `"ssh"`).
pub fn tick(runtime_dir: &Path) {
    tick_with(runtime_dir, "ssh");
}

/// Tick with an injectable ssh binary (tests point it at a stub
/// script; production always passes `"ssh"`).
pub fn tick_with(runtime_dir: &Path, ssh_bin: &str) {
    let Some(_guard) = try_acquire_tick_guard() else {
        log::debug!("chat-command: tick already in progress, skipping");
        return;
    };
    let started = Instant::now();
    let cfg = ChatCommandConfig::live_config_at(runtime_dir);
    let tools = match resolve_chat_tools() {
        Some(t) => t,
        None => {
            log::debug!("chat-command: no mcp__chat__* tools advertised; skipping tick");
            return;
        }
    };
    // Cursor: fresh/corrupt/room-changed states seed to the room's
    // latest id (never replay). A failed seed ends the tick; the
    // next minute retries.
    let (mut state, load_kind) = CommandState::load(runtime_dir);
    let mut cursor: i64 = match (&load_kind, &state.room, state.last_seen_id) {
        (StateLoad::Loaded, Some(room), Some(id)) if room == &cfg.room => id,
        _ => {
            match load_kind {
                StateLoad::Corrupt => {
                    log::warn!("chat-command: state corrupt, resetting cursor to latest id");
                }
                StateLoad::Missing => {
                    log::debug!("chat-command: no state yet, seeding cursor to latest id");
                }
                StateLoad::Loaded => {
                    log::info!("chat-command: room changed or cursor missing, re-seeding cursor");
                }
            }
            let Some(latest) = seed_cursor(&cfg.room) else {
                log::warn!("chat-command: could not seed cursor, skipping tick");
                return;
            };
            state = CommandState {
                room: Some(cfg.room.clone()),
                last_seen_id: Some(latest),
            };
            if let Err(e) = state.save(runtime_dir) {
                log::warn!("chat-command: failed to persist seeded state: {e}");
            }
            latest
        }
    };
    // Page through new messages in id order.
    let mut processed_cmds = 0usize;
    loop {
        let page = read_page(&tools, &cfg.room, cursor);
        let raw_len = page.len();
        let mut messages: Vec<ChatMessage> =
            page.into_iter().filter(|m| m.id > cursor).collect();
        messages.sort_by_key(|m| m.id);
        if messages.is_empty() {
            break;
        }
        let mut stop = false;
        for msg in messages {
            if started.elapsed() >= TICK_BUDGET {
                log::debug!("chat-command: tick budget reached, rest waits for next tick");
                stop = true;
                break;
            }
            // Cap pre-check (no side effects): a message that would
            // consume a command slot waits when the cap is reached —
            // its cursor does NOT advance, so the next tick retries
            // it. Ignored messages never consume slots and always
            // advance. (This also makes max_commands_per_tick = 0
            // mean "execute nothing", not "execute one".)
            let would_consume = is_allowed_sender(&msg.sender, &cfg.allowed_senders)
                && matches!(
                    parse_trigger(&msg.body),
                    Trigger::Empty | Trigger::Command(_)
                );
            if would_consume && processed_cmds >= cfg.max_commands_per_tick {
                stop = true;
                break;
            }
            let slot = process_message(&tools, &cfg, runtime_dir, ssh_bin, &msg);
            if matches!(slot, Slot::Command) {
                processed_cmds += 1;
            }
            state.last_seen_id = Some(msg.id);
            if let Err(e) = state.save(runtime_dir) {
                log::warn!("chat-command: failed to persist cursor: {e}");
                stop = true;
                break;
            }
            cursor = msg.id;
        }
        if stop || raw_len < READ_LIMIT as usize {
            break;
        }
    }
    // Retry any LLM spawns deferred for lack of a runtime (normally
    // a no-op: the submit already spawned).
    task_handler::drain_pending(&tools, &cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_defs() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"type":"function","function":{"name":"mcp__chat__chat_read"}}),
            serde_json::json!({"type":"function","function":{"name":"mcp__chat__chat_send"}}),
            serde_json::json!({"type":"function","function":{"name":"mcp__chat__chat_rooms"}}),
        ]
    }

    fn install_stubs(read_output: &str, rooms_output: &str) {
        crate::mcp_client::override_tool_definitions_for_tests(test_defs());
        crate::mcp_client::stub_try_execute_for_tests("mcp__chat__chat_read", read_output);
        crate::mcp_client::stub_try_execute_for_tests("mcp__chat__chat_send", "{\"id\": 99}");
        crate::mcp_client::stub_try_execute_for_tests("mcp__chat__chat_rooms", rooms_output);
    }

    fn send_bodies() -> Vec<String> {
        crate::mcp_client::take_try_execute_calls_for_tests()
            .into_iter()
            .filter(|(name, _)| name == "mcp__chat__chat_send")
            .filter_map(|(_, args)| args.get("body").and_then(|b| b.as_str()).map(str::to_string))
            .collect()
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "helix-chat-command-tick-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_config(dir: &std::path::Path, body: &str) {
        std::fs::write(dir.join("config.toml"), body).unwrap();
    }

    const BASE_CONFIG: &str = "[chat_command]\nallowed_senders = [\"cobble-jr\", \"john\"]\n";

    #[test]
    fn tool_resolution_needs_both_tools() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        crate::mcp_client::override_tool_definitions_for_tests(test_defs());
        assert!(resolve_chat_tools().is_some());
        crate::mcp_client::override_tool_definitions_for_tests(vec![
            serde_json::json!({"type":"function","function":{"name":"mcp__chat__chat_read"}}),
        ]);
        assert!(resolve_chat_tools().is_none(), "send missing must fail closed");
        crate::mcp_client::override_tool_definitions_for_tests(vec![]);
        assert!(resolve_chat_tools().is_none());
        crate::mcp_client::clear_test_overrides();
    }

    #[test]
    fn parse_messages_array_and_structured_forms() {
        let arr = r#"[{"id": 5, "room": "command", "sender": "cobble-jr", "body": "!hostname"}]"#;
        let msgs = parse_messages(arr);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, 5);
        assert_eq!(msgs[0].sender, "cobble-jr");
        // Structured-first form (what the MCP layer actually returns:
        // pretty {"messages": [...]} followed by the text array).
        let both = "{\"messages\": [{\"id\": 7, \"room\": \"command\", \"sender\": \"john\", \"body\": \"hi\"}]}\n[]";
        let msgs = parse_messages(both);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, 7);
        assert!(parse_messages("hub error: boom").is_empty());
        assert!(parse_messages("").is_empty());
    }

    #[test]
    fn parse_rooms_latest_forms() {
        let arr = r#"[{"name": "general", "last_id": 3}, {"name": "command", "last_id": 41}]"#;
        assert_eq!(parse_rooms_latest(arr, "command"), Some(Some(41)));
        assert_eq!(parse_rooms_latest(arr, "nope"), Some(None));
        let obj = r#"{"rooms": [{"name": "command", "last_id": null}]}"#;
        assert_eq!(parse_rooms_latest(obj, "command"), Some(None));
        assert_eq!(parse_rooms_latest("garbage", "command"), None);
    }

    #[test]
    fn trigger_and_target_parsing() {
        assert_eq!(parse_trigger("hello"), Trigger::Ignored);
        assert_eq!(parse_trigger("  !hostname  "), Trigger::Command("hostname".to_string()));
        assert_eq!(parse_trigger("!"), Trigger::Empty);
        assert_eq!(parse_trigger("!   "), Trigger::Empty);
        assert_eq!(split_target("hostname"), (Target::Local, "hostname".to_string()));
        assert_eq!(split_target("asus: hostname"), (Target::Asus, "hostname".to_string()));
        assert_eq!(split_target("Asus: hostname"), (Target::Local, "Asus: hostname".to_string()));
    }

    #[test]
    fn tick_tools_absent_quiet_skips() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        crate::mcp_client::override_tool_definitions_for_tests(vec![]);
        let dir = tmpdir("absent");
        write_config(&dir, BASE_CONFIG);
        tick(&dir);
        assert!(
            !dir.join("chat_command").exists(),
            "absent tools must not create state"
        );
        assert!(crate::mcp_client::take_try_execute_calls_for_tests().is_empty());
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_authorized_executes_and_advances_cursor() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("auth");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!whoami"}, {"id": 2, "room": "command", "sender": "cobble-jr", "body": "!hostname"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 2, "two commands → two replies: {bodies:?}");
        assert!(bodies[0].contains("$ whoami"), "echo first: {}", bodies[0]);
        assert!(bodies[1].contains("$ hostname"), "echo second: {}", bodies[1]);
        let (state, _) = CommandState::load(&dir);
        assert_eq!(state.last_seen_id, Some(2));
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_unauthorized_and_non_bang_ignored_no_reply() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("unauth");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "helix", "body": "!hostname"}, {"id": 2, "room": "command", "sender": "grok-cli", "body": "!whoami"}, {"id": 3, "room": "command", "sender": "cobble-jr", "body": "hello"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        assert!(send_bodies().is_empty(), "ignored messages must not reply");
        let (state, _) = CommandState::load(&dir);
        assert_eq!(state.last_seen_id, Some(3), "cursor advances past ignored messages");
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_empty_and_asus_not_enabled_reply() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("empty-asus");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "john", "body": "!   "}, {"id": 2, "room": "command", "sender": "john", "body": "!asus: hostname"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("empty command"), "{}", bodies[0]);
        assert!(bodies[1].contains("not enabled"), "{}", bodies[1]);
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_corrupt_state_resets_without_replay() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("corrupt");
        write_config(&dir, BASE_CONFIG);
        let path = CommandState::state_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{broken").unwrap();
        install_stubs(
            r#"[{"id": 5, "room": "command", "sender": "cobble-jr", "body": "!hostname"}]"#,
            r#"[{"name": "command", "last_id": 10}]"#,
        );
        tick(&dir);
        assert!(
            send_bodies().is_empty(),
            "old command below the reset cursor must never execute"
        );
        let (state, _) = CommandState::load(&dir);
        assert_eq!(state.last_seen_id, Some(10));
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_max_commands_cap_leaves_rest_for_next_tick() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("cap");
        write_config(
            &dir,
            "[chat_command]\nallowed_senders = [\"cobble-jr\"]\nmax_commands_per_tick = 2\n",
        );
        let msgs: Vec<String> = (1..=5)
            .map(|i| format!("{{\"id\": {i}, \"room\": \"command\", \"sender\": \"cobble-jr\", \"body\": \"!whoami\"}}"))
            .collect();
        install_stubs(
            &format!("[{}]", msgs.join(",")),
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        assert_eq!(send_bodies().len(), 2, "cap of 2 must stop after 2 replies");
        let (state, _) = CommandState::load(&dir);
        assert_eq!(state.last_seen_id, Some(2));
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_guard_skips_overlapping_tick() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("guard");
        write_config(&dir, BASE_CONFIG);
        let guard = try_acquire_tick_guard().expect("guard must acquire");
        tick(&dir);
        assert!(
            crate::mcp_client::take_try_execute_calls_for_tests().is_empty(),
            "guarded tick must not touch tools"
        );
        drop(guard);
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_chunks_long_reply_in_order() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("chunk");
        write_config(
            &dir,
            "[chat_command]\nallowed_senders = [\"cobble-jr\"]\nchunk_bytes = 10\n",
        );
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!whoami"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert!(bodies.len() >= 2, "10-byte budget must chunk a whoami reply: {bodies:?}");
        for (i, b) in bodies.iter().enumerate() {
            assert!(b.starts_with(&format!("[{}/{}] ", i + 1, bodies.len())), "{b:?}");
        }
        let joined: String = bodies
            .iter()
            .map(|b| b.split_once("] ").map(|(_, rest)| rest).unwrap_or(b.as_str()))
            .collect();
        assert!(joined.contains("$ whoami"), "reassembled reply keeps the echo: {joined:?}");
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Phase 2 (tasks 304/305): ASUS relay via stub ssh ─────────────

    /// A stub `ssh` script: appends its argv to $ASUS_STUB_LOG and
    /// prints a canned hostname. Lets tests assert the exact ssh
    /// invocation (or zero invocations) without touching a real host.
    fn stub_ssh(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("ssh-stub");
        let log = dir.join("ssh-calls.log");
        std::fs::write(
            &script,
            "#!/bin/sh\necho \"$@\" >> \"$ASUS_STUB_LOG\"\necho \"ASUS-HOSTNAME\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, log)
    }

    fn asus_config() -> String {
        "[chat_command]\nallowed_senders = [\"cobble-jr\"]\nasus_enabled = true\nasus_host = \"100.112.110.14\"\nasus_user = \"someone\"\nasus_key_path = \"/home/cobble/.ssh/id_helix_asus\"\n"
            .to_string()
    }

    #[test]
    fn tick_asus_routes_to_stub_ssh_with_expected_argv() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("asus-ok");
        write_config(&dir, &asus_config());
        let (script, log) = stub_ssh(&dir);
        unsafe { std::env::set_var("ASUS_STUB_LOG", &log) };
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!asus: hostname"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick_with(&dir, script.to_str().unwrap());
        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            calls.contains("BatchMode=yes")
                && calls.contains("ConnectTimeout=15")
                && calls.contains("/home/cobble/.ssh/id_helix_asus")
                && calls.contains("someone@100.112.110.14")
                && calls.contains("hostname"),
            "ssh argv must carry BatchMode + timeout + key + user@host + words: {calls:?}"
        );
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("asus$ hostname"), "{}", bodies[0]);
        assert!(bodies[0].contains("ASUS-HOSTNAME"), "{}", bodies[0]);
        unsafe { std::env::remove_var("ASUS_STUB_LOG") };
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_asus_rejections_make_zero_ssh_calls() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("asus-reject");
        write_config(&dir, &asus_config());
        let (script, log) = stub_ssh(&dir);
        unsafe { std::env::set_var("ASUS_STUB_LOG", &log) };
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!asus: Remove-Item x"}, {"id": 2, "room": "command", "sender": "cobble-jr", "body": "!asus: Get-Process | Sort-Object CPU"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick_with(&dir, script.to_str().unwrap());
        assert!(
            !log.exists() || std::fs::read_to_string(&log).unwrap().is_empty(),
            "disallowed + pipeline ASUS commands must never reach ssh"
        );
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 2);
        assert!(bodies.iter().all(|b| b.contains("rejected")), "{bodies:?}");
        unsafe { std::env::remove_var("ASUS_STUB_LOG") };
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_asus_disabled_makes_zero_ssh_calls() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("asus-off");
        write_config(&dir, BASE_CONFIG);
        let (script, log) = stub_ssh(&dir);
        unsafe { std::env::set_var("ASUS_STUB_LOG", &log) };
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!asus: hostname"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick_with(&dir, script.to_str().unwrap());
        assert!(
            !log.exists(),
            "ASUS disabled must attempt zero ssh calls"
        );
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("not enabled"), "{}", bodies[0]);
        unsafe { std::env::remove_var("ASUS_STUB_LOG") };
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Phase 3 (tasks 309/310): envelope + task handoff ──────────

    #[test]
    fn tick_json_command_executes() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("json-cmd");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!{\"how\": \"command\", \"which\": \"local\", \"time\": 30, \"scope\": \"read\", \"command\": \"uptime\"}"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
        assert!(bodies[0].contains("$ uptime"), "{}", bodies[0]);
        let (state, _) = CommandState::load(&dir);
        assert_eq!(state.last_seen_id, Some(1));
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_json_command_asus_not_enabled() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("json-asus-off");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!{\"how\": \"command\", \"which\": \"asus\", \"scope\": \"read\", \"command\": \"hostname\"}"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
        assert!(bodies[0].contains("asus$ hostname"), "{}", bodies[0]);
        assert!(bodies[0].contains("not enabled"), "{}", bodies[0]);
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_json_llm_posts_ack_and_writes_role_file() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        task_handler::reset_for_tests();
        let dir = tmpdir("json-llm-ack");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!{\"how\": \"llm\", \"which\": \"server\", \"time\": 300, \"scope\": \"read\", \"task\": \"Check disk usage\"}"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
        assert!(
            bodies[0].contains("task #1 accepted, running on server, budget 300s"),
            "{}",
            bodies[0]
        );
        // Role file lands under the tick's runtime dir (the tmpdir),
        // never the worktree — and carries the read allowlist.
        let role = std::fs::read_to_string(dir.join("agents").join("chat-task-read.md"))
            .expect("role file must exist");
        assert!(role.contains("tools: ["), "{role}");
        assert!(role.contains("read_note"), "{role}");
        assert!(!role.contains("write_note"), "{role}");
        let (state, _) = CommandState::load(&dir);
        assert_eq!(state.last_seen_id, Some(1));
        task_handler::reset_for_tests();
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_json_llm_second_task_for_busy_backend_queues() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        task_handler::reset_for_tests();
        let dir = tmpdir("json-llm-queue");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!{\"how\": \"llm\", \"which\": \"server\", \"scope\": \"read\", \"task\": \"First task\"}"}, {"id": 2, "room": "command", "sender": "john", "body": "!{\"how\": \"llm\", \"which\": \"server\", \"scope\": \"read\", \"task\": \"Second task\"}"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 2, "{bodies:?}");
        assert!(bodies[0].contains("task #1 accepted"), "{}", bodies[0]);
        assert!(bodies[1].contains("task #2 queued"), "{}", bodies[1]);
        assert!(bodies[1].contains("server is busy running task #1"), "{}", bodies[1]);
        assert!(bodies[1].contains("position 1"), "{}", bodies[1]);
        task_handler::reset_for_tests();
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_json_rejections_and_helix_structured_ignored() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        task_handler::reset_for_tests();
        let dir = tmpdir("json-reject");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!{\"how\": \"llm\", \"which\": \"server\", \"scope\": \"read\"}"}, {"id": 2, "room": "command", "sender": "cobble-jr", "body": "!{\"how\": \"llm\", \"which\": \"mars\", \"scope\": \"read\", \"task\": \"x\"}"}, {"id": 3, "room": "command", "sender": "helix", "body": "!{\"how\": \"llm\", \"which\": \"server\", \"scope\": \"read\", \"task\": \"Loop bait\"}"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 2, "{bodies:?}");
        assert!(bodies[0].contains("rejected: llm task needs a task string"), "{}", bodies[0]);
        assert!(bodies[1].contains("rejected: unknown backend 'mars'"), "{}", bodies[1]);
        let (state, _) = CommandState::load(&dir);
        assert_eq!(state.last_seen_id, Some(3), "cursor advances past ignored helix message");
        task_handler::reset_for_tests();
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_invalid_how_falls_back_to_plain_command() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = tmpdir("json-fallback");
        write_config(&dir, BASE_CONFIG);
        install_stubs(
            r#"[{"id": 1, "room": "command", "sender": "cobble-jr", "body": "!{\"how\": \"bogus\"}"}, {"id": 2, "room": "command", "sender": "cobble-jr", "body": "!uptime"}]"#,
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 2, "{bodies:?}");
        // Plain-path rejection: echo + metachar reason (a structured
        // rejection would have no "$ " echo).
        assert!(bodies[0].contains("$ "), "{}", bodies[0]);
        assert!(bodies[0].contains("forbidden character"), "{}", bodies[0]);
        assert!(bodies[1].contains("$ uptime"), "{}", bodies[1]);
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tick_json_oversize_rejects() {
        let _lock = crate::tools::subagent_tool::test_env_lock();
        task_handler::reset_for_tests();
        let dir = tmpdir("json-oversize");
        write_config(&dir, BASE_CONFIG);
        let big_task = "x".repeat(9000);
        let msg = format!(
            "{{\"id\": 1, \"room\": \"command\", \"sender\": \"cobble-jr\", \"body\": \"!{{\\\"how\\\": \\\"llm\\\", \\\"which\\\": \\\"server\\\", \\\"scope\\\": \\\"read\\\", \\\"task\\\": \\\"{big_task}\\\"}}\"}}"
        );
        install_stubs(
            &format!("[{msg}]"),
            r#"[{"name": "command", "last_id": 0}]"#,
        );
        tick(&dir);
        let bodies = send_bodies();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
        assert!(bodies[0].contains("rejected: message too large"), "{}", bodies[0]);
        task_handler::reset_for_tests();
        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
