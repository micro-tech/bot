//! Helix self-improving loop (tasks 277–291).
//!
//! The loop watches Helix's own health, proposes fixes as task-list-shaped
//! records with test evidence, and John merges the ones he likes. The
//! loop NEVER merges, NEVER pushes to master, NEVER restarts the service,
//! NEVER writes the live tree or live config, and NEVER sends outbound
//! mail/chat — those rules are enforced by construction in this module
//! (no code path here performs any of them), not by convention.
//!
//! Module map (one file per task, in dependency order):
//! - `config` (277): `[self_improve]` section, fail-closed defaults.
//! - `metrics` (278): append-only per-cycle ledger, metadata-only.
//! - `worker` (282): worker brief, tight allowlist, proposal record,
//!   R3 trace validation at intake.
//! - `budgets` (283): hard caps, cooldowns, dedupe, cycle timeout.
//! - `kill` is folded into this file (284): three kill mechanisms.
//! - `git` (286): narrow git wrapper (allowlist, not denylist).
//! - `evaluator` (287): worktree evaluation, D5 gate, 500-line cap.
//! - `audit` (288): metadata-only secrets scan + sensitive paths.
//! - `pr` (290): PR payload + target allowlist + pr_opening flag.
//!
//! Phase gates (280, 285, 289, 291) are John's eyeball + elapsed time —
//! they are NOT code and this module never claims to pass them.

pub mod audit;
pub mod budgets;
pub mod config;
pub mod evaluator;
pub mod git;
pub mod metrics;
pub mod pr;
pub mod worker;

// Later tasks add their modules here in dependency order:
// audit (288), budgets (283), evaluator (287), git (286),
// metrics (278), pr (290), worker (282).

pub use config::SelfImproveConfig;

use std::path::PathBuf;
use std::sync::OnceLock;

static RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Set at startup from main.rs (next to `support_watch::set_runtime_dir`).
/// First call wins (production); falls back to the process CWD.
pub fn set_runtime_dir(dir: PathBuf) {
    let _ = RUNTIME_DIR.set(dir);
}

/// Test seam: override the runtime dir. Tests that use it must hold
/// `test_env_lock()` — the override is process-global.
#[cfg(test)]
static TEST_RUNTIME_DIR: std::sync::Mutex<Option<PathBuf>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub fn override_runtime_dir_for_tests(dir: PathBuf) {
    *TEST_RUNTIME_DIR.lock().unwrap() = Some(dir);
}

/// Runtime dir for the loop's ledger, proposals, worktrees, and queue.
/// Mirrors the `support_watch` pattern.
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

/// Live `[self_improve]` config, read from `<runtime>/config.toml`
/// (same file `support_watch::live_config` reads). Missing file yields
/// the fail-closed defaults.
pub(crate) fn live_config() -> SelfImproveConfig {
    live_config_at(&runtime_dir())
}

/// Same as `live_config`, but against an explicit runtime dir. The CPU
/// routine handler uses this with its own `runtime_dir` (authoritative
/// for the running process) instead of the module global, so config can
/// never come from a different directory than the ledger and stop file.
pub(crate) fn live_config_at(dir: &std::path::Path) -> SelfImproveConfig {
    let path = dir.join("config.toml");
    std::fs::read_to_string(&path)
        .map(|s| SelfImproveConfig::load_from_toml(&s))
        .unwrap_or_default()
}

// ── Kill switch (task 284, D6) ────────────────────────────────────────────

/// Which kill condition stopped a cycle, if any. Logged verbatim so John
/// can tell a kill from a crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillCondition {
    /// `[self_improve] enabled = false` in config.
    ConfigDisabled,
    /// `SELF_IMPROVE_OFF` (or the configured stop-file name) present
    /// in the runtime dir.
    StopFile,
    /// The `self-improve` routine is disabled in the registry, so the
    /// scheduler should not have fired — belt-and-braces re-check.
    RoutineDisabled,
}

/// The abort log line for a kill condition. The handler logs this
/// verbatim: the condition name is always in the line, so a kill is
/// never mistaken for a crash in the logs.
pub fn kill_abort_message(cond: KillCondition) -> String {
    format!("self_improve: cycle aborted: {cond:?}")
}

/// Check all three kill mechanisms, in order, before any cycle work.
/// Returns the stopping condition, or `None` when the loop may proceed.
/// The caller (the CPU routine handler) aborts on `Some` and logs which
/// condition stopped it. Any ONE suffices; there is no "finish the
/// current cycle" exception for work that would write outside the
/// worktree — in-flight worktree writes are simply abandoned with the
/// worktree (the handler drops its worktree handle and runs cleanup).
pub fn check_kill_switches(
    rt_dir: &std::path::Path,
    cfg: &SelfImproveConfig,
    routine_enabled: bool,
) -> Option<KillCondition> {
    if !cfg.enabled {
        return Some(KillCondition::ConfigDisabled);
    }
    if rt_dir.join(&cfg.stop_file).exists() {
        return Some(KillCondition::StopFile);
    }
    if !routine_enabled {
        return Some(KillCondition::RoutineDisabled);
    }
    None
}

/// Read-only check of the `self-improve` routine's enabled flag from a
/// routines.json file, WITHOUT seeding or saving (unlike
/// `RoutineRegistry::load_or_seed`). Missing file, unparseable content,
/// or a missing routine entry all read as disabled — fail-closed, so a
/// handler that cannot prove the routine is enabled must not proceed.
pub fn routine_enabled_in_file(
    routines_path: &std::path::Path,
    routine_id: &str,
) -> bool {
    let Ok(data) = std::fs::read_to_string(routines_path) else {
        return false;
    };
    let Ok(routines) = serde_json::from_str::<Vec<serde_json::Value>>(&data) else {
        return false;
    };
    routines.iter().any(|r| {
        r.get("id").and_then(|v| v.as_str()) == Some(routine_id)
            && r.get("enabled").and_then(|v| v.as_bool()) == Some(true)
    })
}

/// Count currently-open proposals from the proposals file (task 282
/// defines the record schema; this reader is deliberately lenient).
/// Missing or unparseable file counts as zero — the loop has proposed
/// nothing yet, which is the fail-closed reading.
fn count_open_proposals(rt_dir: &std::path::Path) -> usize {
    let path = rt_dir.join("self_improve").join("proposals.json");
    let Ok(data) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(records) = serde_json::from_str::<Vec<serde_json::Value>>(&data) else {
        return 0;
    };
    records
        .iter()
        .filter(|r| {
            matches!(
                r.get("status").and_then(|v| v.as_str()),
                Some("proposed")
                    | Some("evaluating")
                    | Some("evaluated")
                    | Some("queued")
                    | Some("needs-human-review")
            )
        })
        .count()
}

/// Read-only status snapshot for `GET /self_improve/status` (task 279):
/// current phase, kill-switch states, budget limits + current open
/// count, and the latest ledger entry (or null before the first cycle).
/// Metadata-only: the ledger entry it embeds is already counts-only.
pub fn status_snapshot() -> serde_json::Value {
    let rt = runtime_dir();
    let cfg = live_config();
    let latest = metrics::latest_entry(&rt, &cfg);
    serde_json::json!({
        "phase": cfg.phase,
        "enabled": cfg.enabled,
        "kill_switches": {
            "config_enabled": cfg.enabled,
            "stop_file_present": rt.join(&cfg.stop_file).exists(),
            "routine_enabled": routine_enabled_in_file(&rt.join("routines.json"), "self-improve"),
        },
        "budgets": {
            "max_proposals_per_week": cfg.max_proposals_per_week,
            "max_open_per_signature": cfg.max_open_per_signature,
            "max_cycle_minutes": cfg.max_cycle_minutes,
            "max_diff_lines": cfg.max_diff_lines,
            "proposals_open": count_open_proposals(&rt),
        },
        "latest_ledger": latest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_enabled() -> SelfImproveConfig {
        SelfImproveConfig {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn kill_config_disabled_aborts() {
        // GIVEN config enabled=false, THEN the config condition stops it.
        let cfg = SelfImproveConfig::default();
        assert_eq!(
            check_kill_switches(std::path::Path::new("/nonexistent"), &cfg, true),
            Some(KillCondition::ConfigDisabled)
        );
    }

    #[test]
    fn kill_stop_file_aborts() {
        // GIVEN an enabled config but a stop file present, THEN the
        // stop-file condition stops it.
        let dir = std::env::temp_dir().join("helix-self-improve-kill-stop");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SELF_IMPROVE_OFF"), b"").unwrap();
        assert_eq!(
            check_kill_switches(&dir, &cfg_enabled(), true),
            Some(KillCondition::StopFile)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_routine_disabled_aborts() {
        // GIVEN config + stop file clear but the routine disabled,
        // THEN the routine-disabled condition stops it.
        let dir = std::env::temp_dir().join("helix-self-improve-kill-routine");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            check_kill_switches(&dir, &cfg_enabled(), false),
            Some(KillCondition::RoutineDisabled)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_all_clear_proceeds() {
        // GIVEN all three mechanisms clear, THEN no condition stops it.
        let dir = std::env::temp_dir().join("helix-self-improve-kill-clear");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(check_kill_switches(&dir, &cfg_enabled(), true), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_abort_message_names_the_condition() {
        // GIVEN each kill condition, THEN its abort line names it —
        // a kill is never mistaken for a crash in the logs.
        assert!(kill_abort_message(KillCondition::ConfigDisabled).contains("ConfigDisabled"));
        assert!(kill_abort_message(KillCondition::StopFile).contains("StopFile"));
        assert!(kill_abort_message(KillCondition::RoutineDisabled).contains("RoutineDisabled"));
    }

    #[test]
    fn routine_enabled_in_file_reads_flag_without_seeding() {
        // GIVEN a routines.json with self-improve enabled=false and
        // another routine enabled=true, THEN only the named routine's
        // flag is reported — and the file is byte-identical after.
        let dir = std::env::temp_dir().join("helix-self-improve-routine-flag");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("routines.json");
        std::fs::write(
            &path,
            r#"[{"id":"self-improve","enabled":false},{"id":"other","enabled":true}]"#,
        )
        .unwrap();
        assert!(!routine_enabled_in_file(&path, "self-improve"));
        assert!(routine_enabled_in_file(&path, "other"));
        assert!(!routine_enabled_in_file(&path, "missing"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"[{"id":"self-improve","enabled":false},{"id":"other","enabled":true}]"#,
            "read-only check must not seed or rewrite the file"
        );
        // Missing file and garbage both read as disabled (fail-closed).
        assert!(!routine_enabled_in_file(&dir.join("nope.json"), "self-improve"));
        std::fs::write(&path, "not json ][[[").unwrap();
        assert!(!routine_enabled_in_file(&path, "self-improve"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
