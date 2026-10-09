//! API spend tracking for the commander pattern (task 214; policy in 217).
//!
//! Every Gemini/Grok call made by `spawn_subagent` (or any other agent path)
//! is recorded here as an estimated USD cost. The counter persists to a small
//! JSON state file so a restart doesn't reset the day's spend.
//!
//! Cost honesty: we don't meter real tokens yet, so per-call costs are rough
//! estimates, clearly marked. Task 217 may replace these with metered values.

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

/// Rough per-call cost estimates (USD). NOT metered — replace with real
/// token accounting when the backends report usage.
pub const EST_COST_PER_CALL_USD_GEMINI: f64 = 0.02;
pub const EST_COST_PER_CALL_USD_GROK: f64 = 0.05;

fn est_cost_per_call(backend: &LLMBackend) -> Option<f64> {
    match backend {
        LLMBackend::Gemini => Some(EST_COST_PER_CALL_USD_GEMINI),
        LLMBackend::Grok => Some(EST_COST_PER_CALL_USD_GROK),
        _ => None,
    }
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SpendState {
    /// "YYYY-MM-DD" (UTC) -> backend name -> USD spent (estimated).
    days: HashMap<String, HashMap<String, f64>>,
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

    fn spent_today(&self, backend: &LLMBackend) -> f64 {
        self.days
            .get(&Self::today_key())
            .and_then(|m| m.get(Self::backend_key(backend)))
            .copied()
            .unwrap_or(0.0)
    }

    fn add(&mut self, backend: &LLMBackend, usd: f64) {
        let day = self.days.entry(Self::today_key()).or_default();
        *day.entry(Self::backend_key(backend).to_string()).or_default() += usd;
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
    let path = state_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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

/// Record one paid API call. No-op for local backends.
pub fn note_api_call(backend: &LLMBackend) {
    let Some(cost) = est_cost_per_call(backend) else {
        return;
    };
    let lock = tracker();
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    state.add(backend, cost);
    save_state(&state);
    log::debug!(
        "spend: {:?} call noted (est ${:.2}); paid total today ${:.2}",
        backend,
        cost,
        state.paid_total_today()
    );
}

/// Check whether a paid backend may be used right now.
/// Returns Err with a John-facing message when the daily cap is hit.
pub fn check_api_allowed(backend: &LLMBackend) -> Result<(), String> {
    if est_cost_per_call(backend).is_none() {
        return Ok(()); // local backends are always allowed
    }
    let cap = daily_api_cap_usd();
    let lock = tracker();
    let state = lock.lock().unwrap_or_else(|e| e.into_inner());
    let total = state.paid_total_today();
    if total >= cap {
        Err(format!(
            "Daily API spend cap reached (${:.2} of ${:.2} estimated today). \
             Paid backends are paused until tomorrow. The job can run on a local backend instead, \
             or raise [router] daily_api_cap_usd.",
            total, cap
        ))
    } else {
        Ok(())
    }
}

/// Estimated USD spent on paid backends today (for status displays/tests).
pub fn paid_spent_today_usd() -> f64 {
    let lock = tracker();
    let state = lock.lock().unwrap_or_else(|e| e.into_inner());
    state.paid_total_today()
}

/// Test/maintenance hook: reset the in-memory + on-disk state.
#[cfg(test)]
pub fn reset_for_tests() {
    let lock = tracker();
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    *state = SpendState::default();
    save_state(&state);
    if let Ok(mut g) = API_CAP_OVERRIDE_USD.lock() {
        *g = None;
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
    fn gemini_calls_accumulate_spend() {
        let g = lock();
        let _p = with_test_file(&g);
        note_api_call(&LLMBackend::Gemini);
        note_api_call(&LLMBackend::Gemini);
        let spent = paid_spent_today_usd();
        assert!(
            (spent - 2.0 * EST_COST_PER_CALL_USD_GEMINI).abs() < 1e-9,
            "spent={}",
            spent
        );
        // Well under the default $5 cap.
        assert!(check_api_allowed(&LLMBackend::Gemini).is_ok());
    }

    #[test]
    fn cap_blocks_when_exceeded() {
        let g = lock();
        let p = with_test_file(&g);
        // Hammer the counter past the cap.
        for _ in 0..400 {
            note_api_call(&LLMBackend::Grok);
        }
        let err = check_api_allowed(&LLMBackend::Grok).expect_err("cap should trip");
        assert!(err.contains("cap"), "message should mention the cap: {}", err);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn cap_override_changes_effective_cap() {
        // Task 217.2: [router] daily_api_cap_usd overrides the $5 default.
        let g = lock();
        let p = with_test_file(&g);
        set_daily_api_cap_usd(0.05);
        for _ in 0..3 {
            note_api_call(&LLMBackend::Gemini); // 3 x $0.02 = $0.06 > $0.05
        }
        let err = check_api_allowed(&LLMBackend::Gemini).expect_err("override cap should trip");
        assert!(err.contains("$0.05"), "message names the override cap: {}", err);
        reset_for_tests(); // clears the override too
        assert!(check_api_allowed(&LLMBackend::Gemini).is_ok());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn spend_persists_across_reload() {
        let g = lock();
        let p = with_test_file(&g);
        note_api_call(&LLMBackend::Gemini);
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
