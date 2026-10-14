//! API spend tracking + real token usage metering (tasks 214, 217, 236).
//!
//! Every paid-backend call made by `spawn_subagent` (or any other agent path)
//! is recorded here. Task 236 replaced the flat per-call estimates with real
//! token metering: Gemini reports `usageMetadata`, Ollama reports
//! `prompt_eval_count`/`eval_count`, and we price actual tokens. Estimates
//! survive only as fallback when a backend omits usage fields.
//!
//! The counter persists to a small JSON state file so a restart doesn't
//! reset the day's spend. The on-disk format grew in task 236 (per-backend
//! token counts); legacy `date -> backend -> f64` files are auto-migrated.

use crate::router::LLMBackend;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Default daily cap (USD) for paid API backends. Overridable at startup
/// via `[router] daily_api_cap_usd` (see `set_daily_api_cap_usd`).
pub const DEFAULT_DAILY_API_CAP_USD: f64 = 5.00;

/// Operator override for the daily API cap, set once at startup from
/// `[router] daily_api_cap_usd`. Non-positive values are ignored (default wins).
static API_CAP_OVERRIDE_USD: std::sync::Mutex<Option<f64>> =
    std::sync::Mutex::new(None);

/// Set the daily API spend cap (USD) from config.
pub fn set_daily_api_cap_usd(v: f64) {
    if let Ok(mut g) = API_CAP_OVERRIDE_USD.lock() {
        *g = Some(v);
    }
}

/// Effective daily API cap (USD): config override, else the default.
fn daily_api_cap_usd() -> f64 {
    API_CAP_OVERRIDE_USD
        .lock()
        .ok()
        .and_then(|g| *g)
        .filter(|v| *v > 0.0)
        .unwrap_or(DEFAULT_DAILY_API_CAP_USD)
}

/// When the daily cap is exceeded: block further paid calls (true) or warn
/// and continue (false, default). Task 236 recommendation: warn by default —
/// a hard block mid-task orphans in-flight work; the operator opts into the
/// block via `[router] cap_block_on_exceed = true`. John's call to flip.
static CAP_BLOCK_ON_EXCEED: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

/// Set cap-exceed behavior from config.
pub fn set_cap_block_on_exceed(v: bool) {
    if let Ok(mut g) = CAP_BLOCK_ON_EXCEED.lock() {
        *g = v;
    }
}

fn cap_block_on_exceed() -> bool {
    CAP_BLOCK_ON_EXCEED.lock().map(|g| *g).unwrap_or(false)
}

/// Real token counts from a backend response.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// Approximate public pricing, USD per 1M tokens (Flash tier). Update when
/// the model changes — these drift. Deliberately conservative.
const GEMINI_USD_PER_MTOK_INPUT: f64 = 0.10;
const GEMINI_USD_PER_MTOK_OUTPUT: f64 = 0.40;

/// Legacy flat per-call estimates (USD), kept ONLY as fallback when a
/// backend omits usage fields. Metered values always win when present.
pub const EST_COST_PER_CALL_USD_GEMINI: f64 = 0.02;
pub const EST_COST_PER_CALL_USD_GROK: f64 = 0.05;

fn est_cost_per_call(backend: &LLMBackend) -> Option<f64> {
    match backend {
        LLMBackend::Gemini => Some(EST_COST_PER_CALL_USD_GEMINI),
        LLMBackend::Grok => Some(EST_COST_PER_CALL_USD_GROK),
        _ => None,
    }
}

/// Price real tokens. Local backends cost $0; Grok isn't metered yet
/// (falls back to the flat estimate via `note_api_call`).
fn metered_cost_usd(backend: &LLMBackend, usage: &TokenUsage) -> Option<f64> {
    match backend {
        LLMBackend::Gemini => Some(
            (usage.prompt_tokens as f64 * GEMINI_USD_PER_MTOK_INPUT
                + usage.completion_tokens as f64 * GEMINI_USD_PER_MTOK_OUTPUT)
                / 1_000_000.0,
        ),
        _ => None,
    }
}

/// Extract token counts from a Gemini `generateContent` response.
/// Returns None when `usageMetadata` is absent (caller falls back to estimate).
pub fn parse_gemini_usage(v: &serde_json::Value) -> Option<TokenUsage> {
    let meta = v.get("usageMetadata")?;
    Some(TokenUsage {
        prompt_tokens: meta
            .get("promptTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        completion_tokens: meta
            .get("candidatesTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
    })
}

/// Extract token counts from an Ollama `/api/chat` response.
/// Returns None when the eval fields are absent.
pub fn parse_ollama_usage(v: &serde_json::Value) -> Option<TokenUsage> {
    let prompt = v.get("prompt_eval_count")?.as_u64()?;
    let completion = v.get("eval_count")?.as_u64()?;
    Some(TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
    })
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct DaySpend {
    usd: f64,
    prompt_tokens: u64,
    completion_tokens: u64,
    calls: u64,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SpendState {
    /// "YYYY-MM-DD" (UTC) -> backend name -> spend detail.
    days: HashMap<String, HashMap<String, DaySpend>>,
}

impl SpendState {
    fn today_key() -> String {
        chrono::Utc::now().format("%Y-%m-%d").to_string()
    }

    fn backend_key(backend: &LLMBackend) -> &'static str {
        match backend {
            LLMBackend::Gemini => "gemini",
            LLMBackend::Grok => "grok",
            LLMBackend::LocalOllama => "local_ollama",
            LLMBackend::LanOllama => "lan_ollama",
            LLMBackend::Fallback => "fallback",
        }
    }

    fn day_mut(&mut self, backend: &LLMBackend) -> &mut DaySpend {
        self.days
            .entry(Self::today_key())
            .or_default()
            .entry(Self::backend_key(backend).to_string())
            .or_default()
    }

    fn spent_today(&self, backend: &LLMBackend) -> f64 {
        self.days
            .get(&Self::today_key())
            .and_then(|m| m.get(Self::backend_key(backend)))
            .map(|d| d.usd)
            .unwrap_or(0.0)
    }

    /// Total across paid backends for today.
    fn paid_total_today(&self) -> f64 {
        self.spent_today(&LLMBackend::Gemini) + self.spent_today(&LLMBackend::Grok)
    }
}

fn state_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("HELIX_SPEND_FILE") {
        return std::path::PathBuf::from(p);
    }
    std::path::PathBuf::from("spend_state.json")
}

fn load_state() -> SpendState {
    let raw = match std::fs::read_to_string(state_path()) {
        Ok(s) => s,
        Err(_) => return SpendState::default(),
    };
    // Current format first...
    if let Ok(state) = serde_json::from_str::<SpendState>(&raw) {
        return state;
    }
    // ...then the pre-236 legacy format (date -> backend -> flat f64):
    // migrate USD into DaySpend, token counts start at zero.
    if let Ok(legacy) = serde_json::from_str::<HashMap<String, HashMap<String, f64>>>(&raw)
    {
        let mut state = SpendState::default();
        for (day, backends) in legacy {
            for (backend, usd) in backends {
                state.days.entry(day.clone()).or_default().insert(
                    backend,
                    DaySpend {
                        usd,
                        ..Default::default()
                    },
                );
            }
        }
        return state;
    }
    SpendState::default()
}

fn save_state(state: &SpendState) {
    let path = state_path();
    if let Ok(s) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(&path, s);
    }
}

static SPEND: OnceLock<Mutex<SpendState>> = OnceLock::new();

fn tracker() -> &'static Mutex<SpendState> {
    SPEND.get_or_init(|| Mutex::new(load_state()))
}

fn record(backend: &LLMBackend, usd: f64, usage: TokenUsage) {
    let lock = tracker();
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    let day = state.day_mut(backend);
    day.usd += usd;
    day.prompt_tokens += usage.prompt_tokens;
    day.completion_tokens += usage.completion_tokens;
    day.calls += 1;
    let total = state.paid_total_today();
    save_state(&state);
    log::debug!(
        "spend: {:?} call noted ({} in / {} out tok, ${:.4}); paid total today ${:.2}",
        backend,
        usage.prompt_tokens,
        usage.completion_tokens,
        usd,
        total,
    );
}

/// Record one backend call with REAL token counts (task 236).
/// Falls back to the flat estimate for backends without metering (Grok).
/// Local backends record $0 (tokens still counted for usage stats).
pub fn note_api_usage(backend: &LLMBackend, usage: TokenUsage) {
    let usd = metered_cost_usd(backend, &usage)
        .or_else(|| est_cost_per_call(backend))
        .unwrap_or(0.0);
    record(backend, usd, usage);
}

/// Record one backend call without token data (estimate fallback).
/// Kept for callers that don't have access to the raw response.
pub fn note_api_call(backend: &LLMBackend) {
    let Some(cost) = est_cost_per_call(backend) else {
        return;
    };
    record(backend, cost, TokenUsage::default());
}

/// Check whether a paid backend may be used right now.
/// Default posture (task 236): WARN + log when the cap is hit, continue.
/// With `[router] cap_block_on_exceed = true`: Err, blocking the call.
/// Local backends are always allowed.
pub fn check_api_allowed(backend: &LLMBackend) -> Result<(), String> {
    if est_cost_per_call(backend).is_none() {
        return Ok(()); // local backends are always allowed
    }
    let cap = daily_api_cap_usd();
    let lock = tracker();
    let state = lock.lock().unwrap_or_else(|e| e.into_inner());
    let total = state.paid_total_today();
    if total >= cap {
        let msg = format!(
            "Daily API spend cap reached (${:.2} of ${:.2} today).",
            total, cap
        );
        if cap_block_on_exceed() {
            return Err(format!(
                "{} Paid backends are paused until tomorrow. The job can run on a local backend instead, or raise [router] daily_api_cap_usd.",
                msg
            ));
        }
        log::warn!(
            "{} Continuing (warn mode). Set [router] cap_block_on_exceed = true to block instead.",
            msg
        );
    }
    Ok(())
}

/// Token usage + spend per backend for today, for status displays / /ops/usage.
pub fn usage_summary_today() -> serde_json::Value {
    let lock = tracker();
    let state = lock.lock().unwrap_or_else(|e| e.into_inner());
    let day = state.days.get(&SpendState::today_key());
    let mut backends = serde_json::Map::new();
    if let Some(day) = day {
        for (name, d) in day {
            backends.insert(
                name.clone(),
                serde_json::json!({
                    "usd": d.usd,
                    "prompt_tokens": d.prompt_tokens,
                    "completion_tokens": d.completion_tokens,
                    "calls": d.calls,
                }),
            );
        }
    }
    serde_json::json!({
        "date": SpendState::today_key(),
        "daily_cap_usd": daily_api_cap_usd(),
        "paid_total_usd": state.paid_total_today(),
        "backends": backends,
    })
}

/// Estimated USD spent on paid backends today (for status displays/tests).
pub fn paid_spent_today_usd() -> f64 {
    let lock = tracker();
    let state = lock.lock().unwrap_or_else(|e| e.into_inner());
    state.paid_total_today()
}

/// Test/maintenance hook: reset the in-memory + on-disk state.
/// Reset spend state. Used by unit tests and the eval harness (task 242);
/// not for production use.
pub fn reset_for_tests() {
    let lock = tracker();
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    *state = SpendState::default();
    save_state(&state);
    if let Ok(mut g) = API_CAP_OVERRIDE_USD.lock() {
        *g = None;
    }
    if let Ok(mut g) = CAP_BLOCK_ON_EXCEED.lock() {
        *g = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Serialize these tests: they share the process-global spend tracker
    // and the HELIX_SPEND_FILE env var.
    static TEST_LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .get_or_init(|| StdMutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn with_test_file(_guard: &std::sync::MutexGuard<'static, ()>) -> String {
        let p = format!("/tmp/helix_spend_test_{}.json", std::process::id());
        // SAFETY: tests in this module hold TEST_LOCK; no other thread reads this var concurrently here.
        unsafe { std::env::set_var("HELIX_SPEND_FILE", &p) };
        let _ = std::fs::remove_file(&p);
        reset_for_tests();
        p
    }

    #[test]
    fn local_backends_are_free_and_always_allowed() {
        let g = lock();
        let _p = with_test_file(&g);
        note_api_call(&LLMBackend::LocalOllama);
        note_api_call(&LLMBackend::LanOllama);
        assert_eq!(paid_spent_today_usd(), 0.0);
        assert!(check_api_allowed(&LLMBackend::LocalOllama).is_ok());
    }

    #[test]
    fn gemini_metered_usage_prices_real_tokens() {
        let g = lock();
        let _p = with_test_file(&g);
        // GIVEN 10k input + 5k output tokens at $0.10/$0.40 per Mtok:
        let usage = TokenUsage {
            prompt_tokens: 10_000,
            completion_tokens: 5_000,
        };
        // WHEN recorded:
        note_api_usage(&LLMBackend::Gemini, usage);
        // THEN cost is metered, not the $0.02 flat estimate:
        let spent = paid_spent_today_usd();
        let expected = (10_000.0 * 0.10 + 5_000.0 * 0.40) / 1_000_000.0;
        assert!(
            (spent - expected).abs() < 1e-9,
            "spent={} expected={}",
            spent,
            expected
        );
        assert!(check_api_allowed(&LLMBackend::Gemini).is_ok());
    }

    #[test]
    fn gemini_estimate_fallback_when_no_usage_data() {
        let g = lock();
        let _p = with_test_file(&g);
        // GIVEN a call with no token data (old caller path):
        note_api_call(&LLMBackend::Gemini);
        note_api_call(&LLMBackend::Gemini);
        // THEN the legacy flat estimate still applies:
        let spent = paid_spent_today_usd();
        assert!(
            (spent - 2.0 * EST_COST_PER_CALL_USD_GEMINI).abs() < 1e-9,
            "spent={}",
            spent
        );
    }

    #[test]
    fn metered_beats_estimate_when_both_possible() {
        let g = lock();
        let _p = with_test_file(&g);
        // GIVEN one metered call and one estimated call:
        note_api_usage(
            &LLMBackend::Gemini,
            TokenUsage {
                prompt_tokens: 1_000,
                completion_tokens: 500,
            },
        );
        note_api_call(&LLMBackend::Gemini);
        // THEN the metered call used real pricing (not $0.02):
        let metered = (1_000.0 * 0.10 + 500.0 * 0.40) / 1_000_000.0;
        let spent = paid_spent_today_usd();
        assert!(
            (spent - (metered + EST_COST_PER_CALL_USD_GEMINI)).abs() < 1e-9,
            "spent={}",
            spent
        );
    }

    #[test]
    fn cap_warns_by_default_and_blocks_when_configured() {
        let g = lock();
        let p = with_test_file(&g);
        // GIVEN spend over the cap:
        set_daily_api_cap_usd(0.05);
        for _ in 0..3 {
            note_api_call(&LLMBackend::Gemini); // 3 x $0.02 = $0.06 > $0.05
        }
        // WHEN block mode is off (default):
        // THEN the call is allowed with a warning (no Err):
        assert!(check_api_allowed(&LLMBackend::Gemini).is_ok());
        // WHEN block mode is enabled:
        set_cap_block_on_exceed(true);
        // THEN the cap trips with Err:
        let err = check_api_allowed(&LLMBackend::Gemini).expect_err("cap should trip");
        assert!(err.contains("cap"), "message should mention the cap: {}", err);
        reset_for_tests();
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_gemini_usage_handles_present_and_absent_metadata() {
        // GIVEN a response with usageMetadata:
        let v = serde_json::json!({
            "usageMetadata": {"promptTokenCount": 123, "candidatesTokenCount": 45}
        });
        // WHEN parsed:
        let u = parse_gemini_usage(&v).expect("should parse");
        // THEN counts are exact:
        assert_eq!(u.prompt_tokens, 123);
        assert_eq!(u.completion_tokens, 45);

        // GIVEN a response without usageMetadata:
        let v2 = serde_json::json!({"candidates": []});
        // WHEN parsed:
        // THEN None (caller falls back to estimate):
        assert_eq!(parse_gemini_usage(&v2), None);
    }

    #[test]
    fn parse_ollama_usage_handles_present_and_absent_counts() {
        // GIVEN a response with eval counts:
        let v = serde_json::json!({"prompt_eval_count": 200, "eval_count": 75});
        let u = parse_ollama_usage(&v).expect("should parse");
        assert_eq!(u.prompt_tokens, 200);
        assert_eq!(u.completion_tokens, 75);

        // GIVEN a response without them:
        assert_eq!(parse_ollama_usage(&serde_json::json!({})), None);
    }

    #[test]
    fn usage_summary_reports_per_backend_tokens() {
        let g = lock();
        let _p = with_test_file(&g);
        note_api_usage(
            &LLMBackend::Gemini,
            TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
            },
        );
        let summary = usage_summary_today();
        assert_eq!(summary["backends"]["gemini"]["prompt_tokens"], 100);
        assert_eq!(summary["backends"]["gemini"]["completion_tokens"], 50);
        assert_eq!(summary["backends"]["gemini"]["calls"], 1);
    }

    #[test]
    fn legacy_state_file_migrates_usd() {
        let g = lock();
        let p = with_test_file(&g);
        // GIVEN a pre-236 state file (date -> backend -> flat f64):
        let legacy = serde_json::json!({
            chrono::Utc::now().format("%Y-%m-%d").to_string(): {"gemini": 0.06}
        });
        std::fs::write(&p, serde_json::to_string(&legacy).unwrap()).unwrap();
        // WHEN loaded:
        let state = load_state();
        // THEN the USD survived migration, tokens start at zero:
        let day = state.days.get(&SpendState::today_key()).unwrap();
        assert!((day["gemini"].usd - 0.06).abs() < 1e-9);
        assert_eq!(day["gemini"].prompt_tokens, 0);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn spend_persists_across_reload() {
        let g = lock();
        let p = with_test_file(&g);
        note_api_usage(
            &LLMBackend::Gemini,
            TokenUsage {
                prompt_tokens: 1000,
                completion_tokens: 100,
            },
        );
        let before = paid_spent_today_usd();
        // Simulate a restart: drop in-memory state, reload from disk.
        {
            let lock = tracker();
            let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
            *state = SpendState::default();
        }
        {
            let lock = tracker();
            let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
            *state = load_state();
        }
        assert!((paid_spent_today_usd() - before).abs() < 1e-9);
        let _ = std::fs::remove_file(&p);
    }
}
