//! SYSADMIN-01 support-mail watcher (tasks 255–260).
//!
//! Adaptive poll loop for `support@micro-tech.xyz`:
//! - 15 min during business hours (08:00–18:00 America/New_York, adjustable
//!   via `[support]` in config.toml),
//! - 1-min burst while unhandled mail exists (exits after 2 clean checks),
//! - 30 min overnight.
//!
//! Driven by the `support-mail-watch` routine (`every 1m`) in the cron
//! registry — one routine plus this internal state machine, never three
//! routines toggled on/off (that races). Polling and orchestration run
//! IN-PROCESS on the R630: no LLM is spawned just to poll (John's call).
//! Reply drafting delegates to a capable backend per his commander pattern
//! (task 258); this module only detects new mail and hands it to a
//! [`MailSink`].
//!
//! Inbound mail is untrusted: it flows through `tools::execute`, which wraps
//! MCP outputs with the task-235 injection tagging (`security::maybe_tag`),
//! so the model layer treats message bodies as DATA, never instructions.

pub mod config;
pub mod state;
pub mod drafts;

use std::path::PathBuf;
use std::sync::OnceLock;

use chrono_tz::Tz;
use log::{debug, info, warn};

pub use config::SupportConfig;
pub use state::{WatchMode, WatchState};

/// Backend pinned for support reply drafting (task 258).
///
/// John's architectural call, recorded here so it can't drift: the R630's
/// local models are not capable enough to write customer-facing replies.
/// Polling/orchestration runs locally (no LLM needed); DRAFTING delegates
/// to a capable backend per his commander pattern. Never "auto", never local.
pub const SUPPORT_DRAFT_BACKEND: &str = "gemini";

/// The MCP server name for the support mailbox (mirrors the config.toml
/// template from task 255).
pub const MAIL_SERVER_NAME: &str = "mail";

const BURST_CADENCE_MS: u64 = 60_000;
const DAY_CADENCE_MS: u64 = 15 * 60_000;
const NIGHT_CADENCE_MS: u64 = 30 * 60_000;
/// Clean checks required to leave BURST (task 256 normative).
const BURST_EXIT_STREAK: u32 = 2;

static RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Set at startup from main.rs (next to `persona::set_persona_dir`).
/// First call wins (production); falls back to the process CWD.
pub fn set_runtime_dir(dir: PathBuf) {
    let _ = RUNTIME_DIR.set(dir);
}

/// Test seam: override the runtime dir. Tests that use it must hold
/// `test_env_lock()` — the override is process-global.
#[cfg(test)]
static TEST_RUNTIME_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub fn override_runtime_dir_for_tests(dir: PathBuf) {
    *TEST_RUNTIME_DIR.lock().unwrap() = Some(dir);
}

fn runtime_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(d) = TEST_RUNTIME_DIR.lock().unwrap().clone() {
        return d;
    }
    RUNTIME_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// A new inbound message, parsed defensively from the mail tool's output
/// (mail-mcp's exact schema is unknown in this sandbox — see task 255.3).
#[derive(Debug, Clone)]
pub struct IncomingMail {
    pub id: String,
    pub from: String,
    pub subject: String,
    pub snippet: String,
}

/// Receives new mail discovered by the poll loop. Task 256 ships a
/// [`LogOnlySink`]; task 257/258 wire the real draft-staging sink. The trait
/// keeps the poll loop buildable and testable without the draft flow.
pub trait MailSink {
    fn on_new_mail(&self, messages: &[IncomingMail]);
}

/// Default sink: logs and moves on. Used until the draft flow (257/258)
/// is wired, and as a safe fallback.
pub struct LogOnlySink;

impl MailSink for LogOnlySink {
    fn on_new_mail(&self, messages: &[IncomingMail]) {
        for m in messages {
            info!(
                "support_watch: new mail id={} from={} subject={} (no draft flow wired yet)",
                m.id, m.from, m.subject
            );
        }
    }
}

/// Resolved `mcp__mail__*` tool names. mail-mcp's exact surface is unknown
/// in this sandbox (task 255.3), so names are resolved defensively from the
/// advertised tool list at runtime — never hardcoded.
#[derive(Debug, Clone)]
pub struct ResolvedMailTools {
    pub check: String,
    pub send: String,
}

/// Find the mail check/send tools among advertised `mcp__mail__*` tools.
/// Returns None when the mail server isn't configured or has no tools
/// (the tick then no-ops gracefully).
pub fn resolve_mail_tools() -> Option<ResolvedMailTools> {
    let prefix = format!("mcp__{}__", MAIL_SERVER_NAME);
    let names: Vec<String> = crate::mcp_client::tool_definitions()
        .iter()
        .filter_map(|d| {
            d.get("function")?
                .get("name")?
                .as_str()
                .map(str::to_string)
        })
        .filter(|n| n.starts_with(&prefix))
        .collect();
    if names.is_empty() {
        return None;
    }
    let pick = |cands: &[&str]| {
        names
            .iter()
            .find(|n| {
                let lower = n.to_lowercase();
                cands.iter().any(|c| lower.contains(c))
            })
            .cloned()
    };
    // Prefer explicit check/list names; fall back to the first mail tool so
    // a differently-named surface still yields *something* to probe with.
    let check = pick(&["check_inbox", "list_messages", "check", "inbox", "list", "messages"])
        .or_else(|| names.first().cloned())?;
    let send = pick(&["send_message", "send", "reply"])?;
    Some(ResolvedMailTools { check, send })
}

/// Remove the task-235 `<untrusted-data>` wrapper that `tools::execute`
/// puts around MCP tool output. The wrapper is a transport annotation for
/// the model layer (treat as DATA, never instructions) — the poll loop
/// parses the payload underneath it. Verified: every `mcp__*` name matches
/// `security::is_untrusted_tool`, so this call site is always tagged.
fn strip_untrusted_tags(output: &str) -> &str {
    let t = output.trim();
    let t = t
        .strip_prefix(crate::security::UNTRUSTED_OPEN)
        .unwrap_or(t)
        .trim_start();
    t.strip_suffix(crate::security::UNTRUSTED_CLOSE)
        .unwrap_or(t)
        .trim_end()
}

/// Parse a mail-list tool result into messages. Tries JSON first (array of
/// objects, or an object with a messages/items/results array); falls back to
/// treating unparseable output as zero messages rather than crashing.
/// Id-ish fields are tried in order: id, uid, message_id.
/// The task-235 untrusted-data wrapper is stripped before parsing.
pub fn parse_mail_list(output: &str) -> Vec<IncomingMail> {
    let output = strip_untrusted_tags(output);
    let value: serde_json::Value = match serde_json::from_str(output) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = if let Some(a) = value.as_array() {
        a.clone()
    } else {
        value
            .get("messages")
            .or_else(|| value.get("items"))
            .or_else(|| value.get("results"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };
    arr.iter()
        .filter_map(|m| {
            let id = m
                .get("id")
                .or_else(|| m.get("uid"))
                .or_else(|| m.get("message_id"))
                .and_then(|v| v.as_str())
                .or_else(|| m.get("id").and_then(|v| v.as_u64()).map(|_| ""))?;
            if id.is_empty() {
                return None;
            }
            let str_field = |k: &str| {
                m.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            Some(IncomingMail {
                id: id.to_string(),
                from: str_field("from")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string(),
                subject: str_field("subject"),
                snippet: {
                    let b = str_field("body").replace('\n', " ");
                    b.chars().take(200).collect()
                },
            })
        })
        .collect()
}

/// True when `now_ms` falls inside the configured business window.
/// Unknown timezone names fall back to America/New_York with a warning
/// (a misconfigured TZ must not silently shift the schedule).
pub fn in_business_hours(now_ms: u64, cfg: &SupportConfig) -> bool {
    let tz: Tz = cfg.timezone.parse().unwrap_or_else(|_| {
        warn!(
            "support_watch: unknown timezone '{}', falling back to America/New_York",
            cfg.timezone
        );
        chrono_tz::America::New_York
    });
    let dt = tz.timestamp_millis_opt(now_ms as i64).single();
    let time = match dt {
        Some(d) => d.time(),
        None => return false,
    };
    let (sh, sm) = cfg.business_start_hm();
    let (eh, em) = cfg.business_end_hm();
    let mins = time.hour() * 60 + time.minute();
    let start = sh * 60 + sm;
    let end = eh * 60 + em;
    if start <= end {
        mins >= start && mins < end
    } else {
        // Overnight window (e.g. 22:00–06:00).
        mins >= start || mins < end
    }
}

/// Desired cadence for the current state + time.
pub fn desired_cadence_ms(now_ms: u64, mode: WatchMode, cfg: &SupportConfig) -> u64 {
    match mode {
        WatchMode::Burst => BURST_CADENCE_MS,
        WatchMode::NormalDay | WatchMode::NormalNight => {
            if in_business_hours(now_ms, cfg) {
                DAY_CADENCE_MS
            } else {
                NIGHT_CADENCE_MS
            }
        }
    }
}

/// Should the tick perform a real mailbox check now?
pub fn should_check(now_ms: u64, state: &WatchState, cfg: &SupportConfig) -> bool {
    match state.last_check_ms {
        None => true, // first run: check immediately
        Some(last) => {
            now_ms.saturating_sub(last) >= desired_cadence_ms(now_ms, state.mode, cfg)
        }
    }
}

/// Apply a completed check's outcome to the state. Returns the new mode.
/// `fresh_ids` = ids seen for the first time on this check.
pub fn record_check(
    state: &mut WatchState,
    now_ms: u64,
    fresh_ids: &[String],
    cfg: &SupportConfig,
) -> WatchMode {
    state.last_check_ms = Some(now_ms);
    if !fresh_ids.is_empty() {
        state.mode = WatchMode::Burst;
        state.quiet_streak = 0;
    } else if state.mode == WatchMode::Burst {
        state.quiet_streak += 1;
        if state.quiet_streak >= BURST_EXIT_STREAK {
            state.quiet_streak = 0;
            state.mode = if in_business_hours(now_ms, cfg) {
                WatchMode::NormalDay
            } else {
                WatchMode::NormalNight
            };
        }
    } else {
        // Normal mode just tracks day/night for the status line.
        state.mode = if in_business_hours(now_ms, cfg) {
            WatchMode::NormalDay
        } else {
            WatchMode::NormalNight
        };
    }
    state.mode
}

/// Read the live `[support]` config. Missing file/section → defaults.
fn live_config() -> SupportConfig {
    let path = RUNTIME_DIR
        .get()
        .map(|d| d.join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"));
    // Also honor the same file mcp_client reads when it was set explicitly.
    std::fs::read_to_string(&path)
        .map(|s| SupportConfig::load_from_toml(&s))
        .unwrap_or_default()
}

/// One poll-loop tick. Called by the `support-mail-watch` routine (every
/// 1m); performs a real mailbox check only when the adaptive cadence says
/// so. Never spawns an LLM. Errors are logged, never propagated — a failed
/// tick must not kill the routine.
pub fn tick(sink: &dyn MailSink) {
    let rt = runtime_dir();
    let cfg = live_config();
    if !cfg.enabled {
        debug!("support_watch: disabled in [support]; skipping tick");
        return;
    }
    let mut state = WatchState::load(&rt);
    let now_ms = crate::utils::now_ms();

    if !should_check(now_ms, &state, &cfg) {
        debug!(
            "support_watch: tick skipped (mode={}, last_check={:?})",
            state.mode.as_str(),
            state.last_check_ms
        );
        return;
    }

    let tools = match resolve_mail_tools() {
        Some(t) => t,
        None => {
            // mail-mcp not installed/configured yet (task 255): stay quiet,
            // don't spam the log every minute.
            debug!("support_watch: no mcp__mail__* tools advertised; skipping check");
            state.last_check_ms = Some(now_ms);
            if let Err(e) = state.save(&rt) {
                warn!("support_watch: failed to persist state: {e}");
            }
            return;
        }
    };

    // Task 235: tools::execute wraps MCP outputs with the untrusted-content
    // tag, so downstream model use treats message bodies as DATA.
    let raw = crate::tools::execute(&tools.check, &serde_json::json!({"limit": 50}));
    let messages = parse_mail_list(&raw);
    let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    let fresh_ids = state.note_seen(&ids);
    let fresh: Vec<IncomingMail> = messages
        .into_iter()
        .filter(|m| fresh_ids.contains(&m.id))
        .collect();

    let new_mode = record_check(&mut state, now_ms, &fresh_ids, &cfg);
    if let Err(e) = state.save(&rt) {
        warn!("support_watch: failed to persist state: {e}");
    }

    info!(
        "support_watch: check done mode={} new={} unhandled={} quiet_streak={}",
        new_mode.as_str(),
        fresh.len(),
        state.unhandled_count(),
        state.quiet_streak
    );
    if !fresh.is_empty() {
        sink.on_new_mail(&fresh);
    }
}

/// Mark messages handled (draft staged, reply sent, or dismissed by John).
/// Task 257 calls this; the poll loop consults `unhandled_ids` for BURST.
pub fn mark_handled(ids: &[String]) {
    let rt = runtime_dir();
    let mut state = WatchState::load(&rt);
    state.mark_handled(ids);
    if let Err(e) = state.save(&rt) {
        warn!("support_watch: failed to persist mark_handled: {e}");
    }
}

/// Snapshot for the web UI status line (task 259).
pub fn status_snapshot() -> serde_json::Value {
    let rt = runtime_dir();
    let state = WatchState::load(&rt);
    let cfg = live_config();
    let now_ms = crate::utils::now_ms();
    let cadence = desired_cadence_ms(now_ms, state.mode, &cfg);
    serde_json::json!({
        "mode": state.mode.as_str(),
        "last_check_ms": state.last_check_ms,
        "next_check_ms": state.last_check_ms.map(|l| l + cadence),
        "cadence_ms": cadence,
        "quiet_streak": state.quiet_streak,
        "unhandled_count": state.unhandled_count(),
        "in_business_hours": in_business_hours(now_ms, &cfg),
        "mail_configured": resolve_mail_tools().is_some(),
    })
}

use chrono::Timelike;
use chrono::TimeZone;

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingSink {
        got: std::sync::Mutex<Vec<IncomingMail>>,
    }
    impl MailSink for RecordingSink {
        fn on_new_mail(&self, messages: &[IncomingMail]) {
            self.got.lock().unwrap().extend(messages.iter().cloned());
        }
    }

    /// 2026-10-13 14:00:00 EDT = 18:00 UTC (a Wednesday afternoon).
    fn wednesday_afternoon_ms() -> u64 {
        1_760_294_400_000
    }
    /// 2026-10-13 02:00:00 EDT = 06:00 UTC (overnight).
    fn wednesday_night_ms() -> u64 {
        1_760_251_200_000
    }

    fn cfg() -> SupportConfig {
        SupportConfig::default()
    }

    #[test]
    fn in_business_hours_respects_configured_timezone() {
        let c = cfg();
        assert!(in_business_hours(wednesday_afternoon_ms(), &c));
        assert!(!in_business_hours(wednesday_night_ms(), &c));
    }

    #[test]
    fn business_hours_boundaries() {
        // Exactly 08:00 EDT counts as business hours; 18:00 does not.
        let c = cfg();
        // 2026-10-13 08:00 EDT = 12:00 UTC
        assert!(in_business_hours(1_760_272_800_000, &c));
        // 2026-10-13 18:00 EDT = 22:00 UTC
        assert!(!in_business_hours(1_760_308_800_000, &c));
    }

    #[test]
    fn unknown_timezone_falls_back_to_new_york() {
        let mut c = cfg();
        c.timezone = "Not/AZone".into();
        assert!(in_business_hours(wednesday_afternoon_ms(), &c));
    }

    #[test]
    fn cadence_is_15m_by_day_30m_by_night_1m_in_burst() {
        let c = cfg();
        assert_eq!(
            desired_cadence_ms(wednesday_afternoon_ms(), WatchMode::NormalDay, &c),
            15 * 60_000
        );
        assert_eq!(
            desired_cadence_ms(wednesday_night_ms(), WatchMode::NormalNight, &c),
            30 * 60_000
        );
        assert_eq!(
            desired_cadence_ms(wednesday_night_ms(), WatchMode::Burst, &c),
            60_000
        );
    }

    #[test]
    fn first_run_checks_immediately() {
        let st = WatchState::default();
        assert!(should_check(wednesday_afternoon_ms(), &st, &cfg()));
    }

    #[test]
    fn no_check_before_cadence_elapses() {
        let mut st = WatchState::default();
        st.last_check_ms = Some(wednesday_afternoon_ms());
        st.mode = WatchMode::NormalDay;
        // 5 minutes later: not yet.
        assert!(!should_check(wednesday_afternoon_ms() + 5 * 60_000, &st, &cfg()));
        // 16 minutes later: yes.
        assert!(should_check(wednesday_afternoon_ms() + 16 * 60_000, &st, &cfg()));
    }

    #[test]
    fn new_mail_enters_burst_and_two_clean_checks_exit() {
        let c = cfg();
        let mut st = WatchState::default();
        st.mode = WatchMode::NormalDay;
        // GIVEN new mail arrives:
        let m = record_check(&mut st, wednesday_afternoon_ms(), &["m1".into()], &c);
        assert_eq!(m, WatchMode::Burst);
        assert_eq!(st.quiet_streak, 0);
        // WHEN one clean check passes: still burst, streak 1.
        let m = record_check(&mut st, wednesday_afternoon_ms() + 60_000, &[], &c);
        assert_eq!(m, WatchMode::Burst);
        assert_eq!(st.quiet_streak, 1);
        // WHEN a second clean check passes: back to day cadence.
        let m = record_check(&mut st, wednesday_afternoon_ms() + 120_000, &[], &c);
        assert_eq!(m, WatchMode::NormalDay);
        assert_eq!(st.quiet_streak, 0);
    }

    #[test]
    fn burst_exits_to_night_mode_overnight() {
        let c = cfg();
        let mut st = WatchState::default();
        st.mode = WatchMode::Burst;
        st.quiet_streak = 1;
        let m = record_check(&mut st, wednesday_night_ms(), &[], &c);
        assert_eq!(m, WatchMode::NormalNight);
    }

    #[test]
    fn resolve_mail_tools_picks_sensible_names() {
        // No mail server configured in the test process: None, no panic.
        // (The positive path is covered by the task-260 mock integration,
        // which stubs tool_definitions.)
        let _ = resolve_mail_tools();
    }

    #[test]
    fn parse_mail_list_handles_json_array() {
        let out = r#"[{"id":"42","from":"alice@example.com","subject":"laptop dead","body":"It won't turn on at all."},{"uid":"43","subject":"hi"}]"#;
        let msgs = parse_mail_list(out);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, "42");
        assert_eq!(msgs[0].subject, "laptop dead");
        assert!(msgs[0].snippet.contains("won't turn on"));
        assert_eq!(msgs[1].id, "43");
    }

    #[test]
    fn parse_mail_list_handles_wrapped_object_and_garbage() {
        let out = r#"{"messages":[{"message_id":"7","subject":"x"}]}"#;
        assert_eq!(parse_mail_list(out).len(), 1);
        assert!(parse_mail_list("not json at all").is_empty());
        assert!(parse_mail_list(r#"[{"noid":"1"}]"#).is_empty());
    }

    #[test]
    fn parse_mail_list_strips_untrusted_tags() {
        // tools::execute wraps MCP output in <untrusted-data> (task 235);
        // the poll loop must parse the payload underneath.
        let tagged = crate::security::tag_untrusted(r#"[{"id":"9","subject":"tagged"}]"#);
        assert!(tagged.starts_with(crate::security::UNTRUSTED_OPEN));
        let msgs = parse_mail_list(&tagged);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, "9");
    }

    #[test]
    fn tick_with_stubbed_mail_tools_drives_burst_lifecycle() {
        // Task 256.5: integration test with stub mcp__mail__* tools — no
        // real binary, no network, no LLM. Exercises the real tick() path
        // including tools::execute + the task-235 untrusted tagging.
        let _lock = crate::tools::subagent_tool::test_env_lock();

        // Isolated runtime dir (test override; the test_env_lock above
        // serializes with the other override users).
        let dir = std::env::temp_dir().join("helix-support-watch-test-tick");
        let _ = std::fs::remove_dir_all(&dir);
        override_runtime_dir_for_tests(dir.clone());

        crate::mcp_client::clear_test_overrides();
        crate::mcp_client::override_tool_definitions_for_tests(vec![
            serde_json::json!({"type":"function","function":{"name":"mcp__mail__check_inbox","description":"[MCP:mail] check","parameters":{}}}),
            serde_json::json!({"type":"function","function":{"name":"mcp__mail__send_message","description":"[MCP:mail] send","parameters":{}}}),
        ]);

        let sink = RecordingSink {
            got: std::sync::Mutex::new(Vec::new()),
        };

        // GIVEN one new message in the mailbox:
        crate::mcp_client::stub_try_execute_for_tests(
            "mcp__mail__check_inbox",
            r#"[{"id":"m1","from":"customer@example.com","subject":"laptop dead","body":"It won't turn on."}]"#,
        );
        // WHEN the tick runs:
        tick(&sink);
        // THEN burst is entered, the sink got the message, state persisted.
        let st = WatchState::load(&dir);
        assert_eq!(st.mode, WatchMode::Burst);
        assert_eq!(st.unhandled_count(), 1);
        assert_eq!(st.last_seen_uid.as_deref(), Some("m1"));
        assert_eq!(sink.got.lock().unwrap().len(), 1);
        assert_eq!(sink.got.lock().unwrap()[0].subject, "laptop dead");

        // WHEN two clean checks follow (backdating last_check so the
        // cadence gate doesn't skip them):
        for _ in 0..2 {
            crate::mcp_client::stub_try_execute_for_tests("mcp__mail__check_inbox", "[]");
            let mut st = WatchState::load(&dir);
            st.last_check_ms = Some(st.last_check_ms.unwrap_or(0).saturating_sub(61_000));
            st.save(&dir).unwrap();
            tick(&sink);
        }
        // THEN burst is exited and the streak reset.
        let st = WatchState::load(&dir);
        assert_ne!(st.mode, WatchMode::Burst, "two clean checks must exit burst");
        assert_eq!(st.quiet_streak, 0);
        // No new mail seen: sink still has exactly the first message.
        assert_eq!(sink.got.lock().unwrap().len(), 1);

        crate::mcp_client::clear_test_overrides();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn draft_backend_is_pinned_to_gemini_never_auto_or_local() {
        // John's architectural call (task 258): the R630's local models may
        // never draft customer replies.
        assert_eq!(SUPPORT_DRAFT_BACKEND, "gemini");
        assert_ne!(SUPPORT_DRAFT_BACKEND, "auto");
        assert!(!SUPPORT_DRAFT_BACKEND.contains("ollama"));
        assert!(!SUPPORT_DRAFT_BACKEND.contains("local"));
    }
}
