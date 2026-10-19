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
pub mod drafter;

use std::path::PathBuf;
use std::sync::OnceLock;

use chrono_tz::Tz;
use log::{debug, info, warn};

pub use config::SupportConfig;
pub use state::{WatchMode, WatchState, MAX_DRAFT_ATTEMPTS};

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
const DAY_CADENCE_MS: u64 = 5 * 60_000; // John: 5-min business-hours checks
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

/// Runtime dir for the Helix working tree. Shared with the voice module
/// (task 267) so both read the same live config.toml.
pub(crate) fn runtime_dir() -> PathBuf {
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
pub(crate) fn strip_untrusted_tags(output: &str) -> &str {
    let t = output.trim();
    let t = t
        .strip_prefix(crate::security::UNTRUSTED_OPEN)
        .unwrap_or(t)
        .trim_start();
    t.strip_suffix(crate::security::UNTRUSTED_CLOSE)
        .unwrap_or(t)
        .trim_end()
}

/// Pull the addr-spec out of a `From:` value. Handles `"Alice
/// <alice@example.com>"`, bare `alice@example.com`, and junk — a display
/// name must never become the reply recipient (review B1: the old
/// `split_whitespace().next()` turned "Alice <alice@example.com>" into
/// "Alice", misaddressing the draft).
pub fn extract_email(from: &str) -> String {
    if let Some(a) = from.find('<') {
        if let Some(b) = from[a..].find('>') {
            let inner = from[a + 1..a + b].trim();
            if !inner.is_empty() {
                return inner.to_string();
            }
        }
    }
    from.split_whitespace()
        .find(|t| t.contains('@'))
        .or_else(|| from.split_whitespace().next())
        .unwrap_or("")
        .trim_matches(|c| c == '<' || c == '>')
        .to_string()
}

/// Parse a mail-list tool result into messages. Tries JSON first (array of
/// objects, or an object with a messages/items/results array); falls back to
/// treating unparseable output as zero messages rather than crashing.
/// Id-ish fields are tried in order: id, uid, message_id.
/// Id values may be strings or JSON numbers (mail-mcp emits numeric UIDs,
/// task 264); numbers stringify so downstream code parsing back to u32
/// keeps working.
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
            // mail-mcp emits numeric UIDs (task 264): accept a string id
            // or a JSON number for any of the id-ish keys. A numeric id
            // stringifies ("123") so downstream code parsing it back to
            // u32 keeps working. Items with no id at all are dropped.
            let id_value = m
                .get("id")
                .or_else(|| m.get("uid"))
                .or_else(|| m.get("message_id"))?;
            let id = id_value
                .as_str()
                .map(str::to_string)
                .or_else(|| id_value.as_u64().map(|n| n.to_string()))?;
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
                from: extract_email(&str_field("from")),
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
pub(crate) fn live_config() -> SupportConfig {
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
    let fresh = poll_new_mail();
    if !fresh.is_empty() {
        sink.on_new_mail(&fresh);
    }
}

/// The tick's core: run a cadence-gated mailbox check and return newly seen
/// messages. Split out so async callers (the CPU routine handler, task 258)
/// can draft replies without the sync `MailSink` boundary.
pub fn poll_new_mail() -> Vec<IncomingMail> {
    let rt = runtime_dir();
    let cfg = live_config();
    if !cfg.enabled {
        debug!("support_watch: disabled in [support]; skipping tick");
        return Vec::new();
    }
    let mut state = WatchState::load(&rt);
    let now_ms = crate::utils::now_ms();

    if !should_check(now_ms, &state, &cfg) {
        debug!(
            "support_watch: tick skipped (mode={}, last_check={:?})",
            state.mode.as_str(),
            state.last_check_ms
        );
        return Vec::new();
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
            return Vec::new();
        }
    };

    // Task 235: tools::execute wraps MCP outputs with the untrusted-content
    // tag, so downstream model use treats message bodies as DATA.
    let raw = crate::tools::execute(&tools.check, &serde_json::json!({"limit": 50}));
    let messages = parse_mail_list(&raw);
    let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    let fresh_ids = state.note_seen(&ids);

    // Review B2: re-drive previously-seen but still-unhandled messages
    // (their draft failed on an earlier tick). The attempt counter caps
    // retries so a poison message can't burn a Gemini call every minute
    // forever; past the cap it stays unhandled on the status line for
    // John's manual handling.
    let mut out: Vec<IncomingMail> = Vec::new();
    for m in &messages {
        if fresh_ids.contains(&m.id) {
            out.push(m.clone());
        } else if state.unhandled_ids.contains(&m.id)
            && state.draft_attempts.get(&m.id).copied().unwrap_or(0) < MAX_DRAFT_ATTEMPTS
        {
            *state.draft_attempts.entry(m.id.clone()).or_insert(0) += 1;
            out.push(m.clone());
        }
    }
    let retry_count = out.len().saturating_sub(fresh_ids.len());
    let fresh = out;

    let new_mode = record_check(&mut state, now_ms, &fresh_ids, &cfg);
    if let Err(e) = state.save(&rt) {
        warn!("support_watch: failed to persist state: {e}");
    }

    info!(
        "support_watch: check done mode={} new={} retried={} unhandled={} quiet_streak={}",
        new_mode.as_str(),
        fresh_ids.len(),
        retry_count,
        state.unhandled_count(),
        state.quiet_streak
    );
    fresh
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
    fn cadence_is_5m_by_day_30m_by_night_1m_in_burst() {
        let c = cfg();
        assert_eq!(
            desired_cadence_ms(wednesday_afternoon_ms(), WatchMode::NormalDay, &c),
            5 * 60_000
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
        // 4 minutes later: not yet.
        assert!(!should_check(wednesday_afternoon_ms() + 4 * 60_000, &st, &cfg()));
        // 6 minutes later: yes.
        assert!(should_check(wednesday_afternoon_ms() + 6 * 60_000, &st, &cfg()));
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
    fn extract_email_prefers_addr_spec_over_display_name() {
        // Review B1: "Alice <alice@example.com>" must yield the address,
        // never the display name — it becomes the draft's `to`.
        assert_eq!(
            extract_email("Alice <alice@example.com>"),
            "alice@example.com"
        );
        assert_eq!(
            extract_email("\"Doe, Jane\" <jane.doe@example.com>"),
            "jane.doe@example.com"
        );
        assert_eq!(extract_email("bob@example.com"), "bob@example.com");
        assert_eq!(extract_email(""), "");
        // Degenerate angle brackets fall back to the @-bearing token.
        assert_eq!(extract_email("Carol <>"), "Carol");
    }

    #[test]
    fn parse_mail_list_uses_addr_spec_for_from() {
        let out = r#"[{"id":"1","from":"Alice <alice@example.com>","subject":"hi","body":"x"}]"#;
        let msgs = parse_mail_list(out);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from, "alice@example.com");
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
    fn parse_mail_list_accepts_numeric_uids_like_real_mail_search() {
        // Task 264: mail-mcp's mail_search emits numeric UIDs; the parser
        // used to drop every such message (it only accepted string ids).
        // GIVEN the real mail_search payload shape with a numeric uid:
        let out = r#"{"mailbox":"INBOX","criteria":"ALL","returned":1,"messages":[{"uid":123,"subject":"Hi","from":"john mcconnell <john@example.com>","date":"Thu, 16 Oct 2026 01:13:00 +0000"}]}"#;
        // WHEN parsed, THEN the message survives with the uid stringified:
        let msgs = parse_mail_list(out);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, "123");
        assert_eq!(msgs[0].subject, "Hi");
        assert_eq!(msgs[0].from, "john@example.com");
        // Numeric "id" and "message_id" variants also stringify:
        assert_eq!(parse_mail_list(r#"[{"id":7,"subject":"x"}]"#)[0].id, "7");
        assert_eq!(
            parse_mail_list(r#"[{"message_id":99,"subject":"x"}]"#)[0].id,
            "99"
        );
        // String ids still work, and id-less / null-id items still drop:
        assert_eq!(parse_mail_list(r#"[{"id":"abc","subject":"x"}]"#)[0].id, "abc");
        assert!(parse_mail_list(r#"[{"uid":null,"subject":"x"}]"#).is_empty());
        assert!(parse_mail_list(r#"[{"subject":"x"}]"#).is_empty());
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

    /// Backdate the persisted last_check so the cadence gate lets the
    /// next poll_new_mail() through (burst cadence is 1 minute).
    fn backdate_last_check(dir: &std::path::Path) {
        let mut st = WatchState::load(dir);
        let backdated = st.last_check_ms.unwrap_or(0).saturating_sub(61_000);
        st.last_check_ms = Some(backdated);
        st.save(dir).unwrap();
    }

    #[test]
    fn poll_new_mail_redrives_unhandled_messages_up_to_attempt_cap() {
        // Review B2: a message whose draft failed stays unhandled and must
        // be re-driven on later checks — capped so a poison message can't
        // burn a Gemini call every minute forever.
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let dir = std::env::temp_dir().join("helix-support-watch-test-retry");
        let _ = std::fs::remove_dir_all(&dir);
        override_runtime_dir_for_tests(dir.clone());
        crate::mcp_client::clear_test_overrides();
        crate::mcp_client::override_tool_definitions_for_tests(vec![
            serde_json::json!({"type":"function","function":{"name":"mcp__mail__check_inbox"}}),
            serde_json::json!({"type":"function","function":{"name":"mcp__mail__send_message"}}),
        ]);
        crate::mcp_client::stub_try_execute_for_tests(
            "mcp__mail__check_inbox",
            r#"[{"id":"r1","from":"customer@example.com","subject":"help","body":"broken"}]"#,
        );

        // GIVEN one fresh message:
        let first = poll_new_mail();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, "r1");
        // WHEN its draft fails (stays unhandled — no mark_handled call):
        backdate_last_check(&dir);
        // THEN the next check re-drives it:
        let retry1 = poll_new_mail();
        assert_eq!(retry1.len(), 1, "failed draft must be retried");
        assert_eq!(retry1[0].id, "r1");
        assert_eq!(
            WatchState::load(&dir).draft_attempts.get("r1"),
            Some(&1),
            "retry must consume an attempt"
        );

        // WHEN the cap is reached:
        let mut st = WatchState::load(&dir);
        st.draft_attempts.insert("r1".to_string(), MAX_DRAFT_ATTEMPTS);
        st.save(&dir).unwrap();
        backdate_last_check(&dir);
        // THEN it is no longer re-driven, but stays unhandled for John:
        let retry_capped = poll_new_mail();
        assert!(
            retry_capped.is_empty(),
            "past the attempt cap nothing may be re-driven"
        );
        let st = WatchState::load(&dir);
        assert_eq!(st.unhandled_count(), 1, "still visible on the status line");

        // WHEN John handles it manually:
        super::mark_handled(&["r1".to_string()]);
        // THEN the attempt counter is cleared too:
        let st = WatchState::load(&dir);
        assert_eq!(st.unhandled_count(), 0);
        assert!(!st.draft_attempts.contains_key("r1"));

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

    /// Task 260.1: mock end-to-end of the support-mail loop. Stubbed
    /// `mcp__mail__*` tools deliver a fake inbound message; the poll loop
    /// must enter burst, a draft must stage via the (stubbed) Gemini path,
    /// the draft must be visible as Pending, and NO send may be attempted
    /// before approval. Then approve + send: the send tool must fire exactly
    /// once, carrying the approved text.
    #[tokio::test]
    async fn mock_end_to_end_support_loop() {
        let _g = crate::tools::subagent_tool::test_env_lock();
        let dir = std::env::temp_dir().join("helix-support-e2e-mock");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        override_runtime_dir_for_tests(dir.clone());
        crate::mcp_client::clear_test_overrides();
        crate::mcp_client::override_tool_definitions_for_tests(vec![
            serde_json::json!({"function": {"name": "mcp__mail__check_inbox"}}),
            serde_json::json!({"function": {"name": "mcp__mail__send_message"}}),
        ]);
        // Task 264: the stub emits the REAL mail_search shape — the
        // {"messages": [...]} wrapper with a NUMERIC uid — so the mock
        // cannot mask a parser shape mismatch again.
        crate::mcp_client::stub_try_execute_for_tests(
            "mcp__mail__check_inbox",
            r#"{"mailbox":"INBOX","criteria":"ALL","returned":1,"messages":[{"uid":4242,"from":"customer@example.com","subject":"laptop won't turn on","body":"My laptop won't turn on. It was working yesterday."}]}"#,
        );
        crate::mcp_client::stub_try_execute_for_tests(
            "mcp__mail__send_message",
            r#"{"ok":true,"message_id":"sent-mock-1"}"#,
        );

        // 1. The poll loop picks up the fake inbound message.
        let fresh = poll_new_mail();
        assert_eq!(fresh.len(), 1, "expected one fresh message");
        assert_eq!(fresh[0].id, "4242");

        // 2. Burst mode entered while the message is unhandled.
        let state = WatchState::load(&dir);
        assert_eq!(state.mode, WatchMode::Burst, "expected burst after new mail");
        assert_eq!(state.unhandled_count(), 1);

        // 3. Draft via the Gemini-delegated path (sub-agent dispatch stubbed;
        // no API key or network needed).
        unsafe { std::env::set_var("HELIX_SUBAGENT_TEST_STUB", "1") };
        let draft = drafter::draft_reply(&fresh[0])
            .await
            .expect("draft_reply with stubbed sub-agent");
        unsafe { std::env::remove_var("HELIX_SUBAGENT_TEST_STUB") };
        assert_eq!(draft.status, drafts::DraftStatus::Pending);
        assert_eq!(draft.in_reply_to, "4242");

        // 4. Draft is visible as pending; CRITICAL: no send attempted yet.
        let calls = crate::mcp_client::take_try_execute_calls_for_tests();
        assert!(
            calls.iter().any(|(n, _)| n == "mcp__mail__check_inbox"),
            "check tool should have been called"
        );
        assert!(
            !calls.iter().any(|(n, _)| n == "mcp__mail__send_message"),
            "send attempted before approval: {calls:?}"
        );
        let listed = drafts::list_drafts();
        assert!(
            listed
                .iter()
                .any(|d| d.id == draft.id && d.status == drafts::DraftStatus::Pending),
            "staged draft should be listed as pending"
        );

        // 5. John's two taps: approve, then send. The send tool must fire
        // exactly once, carrying the approved text to the customer.
        let approved = drafts::approve_draft(&draft.id).expect("approve");
        assert_eq!(approved.status, drafts::DraftStatus::Approved);
        let sent = drafts::send_draft(&draft.id).expect("send");
        assert_eq!(sent.status, drafts::DraftStatus::Sent);
        let calls = crate::mcp_client::take_try_execute_calls_for_tests();
        let sends: Vec<_> = calls
            .iter()
            .filter(|(n, _)| n == "mcp__mail__send_message")
            .collect();
        assert_eq!(sends.len(), 1, "send tool must fire exactly once: {calls:?}");
        assert_eq!(
            sends[0].1.get("body").and_then(|v| v.as_str()).unwrap_or(""),
            approved.body.as_str(),
            "send must carry the approved text"
        );
        assert_eq!(
            sends[0].1.get("to").and_then(|v| v.as_str()).unwrap_or(""),
            "customer@example.com"
        );

        crate::mcp_client::clear_test_overrides();
    }
}
