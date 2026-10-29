//! `[self_improve]` config section (task 277).
//!
//! The loop's master switchboard — every later phase reads this struct
//! and nothing else for its knobs. All defaults are fail-closed: a
//! missing section, a malformed value, or an unparseable document yields
//! a loop that is OFF, collect-only, queue-only, and eval-gated. The loop
//! never writes config (only John edits `phase`, `pr_opening`, and
//! `eval_set_approved` in `config.toml`); a wrong-typed value falls the
//! WHOLE section back to defaults, because `config::load_section` cannot
//! tell a typo from a secret-shaped value and refuses to guess (task 265
//! pattern: log keys, never values).

use serde::Deserialize;

/// Parsed `[self_improve]` section. Field order mirrors the documented
/// order in `example.config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SelfImproveConfig {
    /// Master switch, kill-switch #1. Default false: the loop ships
    /// dead until John enables it.
    #[serde(default)]
    pub enabled: bool,

    /// Loop phase: 0 = collect-only (ledger, no proposals),
    /// 1 = propose, 2 = evaluate, 3 = PR-if-allowed. Only John edits
    /// this in config; the loop never writes it. Default 0.
    #[serde(default)]
    pub phase: u8,

    /// Routine schedule string for the `self-improve` routine
    /// (task 281 seeds the same value). Default "daily 03:30".
    #[serde(default = "default_cadence")]
    pub cadence: String,

    /// Hard budget: max proposals per rolling week. Default 3.
    #[serde(default = "default_proposals_per_week")]
    pub max_proposals_per_week: u32,

    /// Hard budget: max open proposals per failing signature. Default 1.
    #[serde(default = "default_open_per_signature")]
    pub max_open_per_signature: u32,

    /// Hard budget: max wall-clock minutes per cycle. Default 60.
    #[serde(default = "default_cycle_minutes")]
    pub max_cycle_minutes: u64,

    /// Hard budget: max diff lines per proposal (enforced in the
    /// Phase 2 evaluator, task 287, where a real diff exists).
    /// Default 500.
    #[serde(default = "default_diff_lines")]
    pub max_diff_lines: usize,

    /// Cooldown after John declines a proposal, days. Default 30.
    #[serde(default = "default_declined_cooldown_days")]
    pub declined_cooldown_days: u64,

    /// Cooldown after a proposal fails evaluation (unchanged failing
    /// signature), days. Default 7.
    #[serde(default = "default_failed_eval_cooldown_days")]
    pub failed_eval_cooldown_days: u64,

    /// Max worktrees alive at once. Default 1.
    #[serde(default = "default_max_worktrees")]
    pub max_worktrees: usize,

    /// Backend for measurement/triage (D3 default: local).
    #[serde(default = "default_triage_backend")]
    pub triage_backend: String,

    /// Backend for proposal drafting (D3 default: local-triage +
    /// Gemini-draft). Default "gemini".
    #[serde(default = "default_draft_backend")]
    pub draft_backend: String,

    /// D1 RUNTIME flag: open PRs from evaluated branches. Default
    /// false (queue-only). Flipping this is John's explicit decision,
    /// recorded separately — the loop never flips it.
    #[serde(default)]
    pub pr_opening: bool,

    /// D5 gate: the task-242 eval set is approved. Default false
    /// (CLOSED — the set is still DRAFT). While false, the evaluator
    /// never promotes anything.
    #[serde(default)]
    pub eval_set_approved: bool,

    /// Kill-switch #2 stop-file name, resolved inside the runtime dir.
    /// Default "SELF_IMPROVE_OFF".
    #[serde(default = "default_stop_file")]
    pub stop_file: String,

    /// Worktree root, relative to the runtime dir.
    /// Default "self_improve/worktrees".
    #[serde(default = "default_worktree_dir")]
    pub worktree_dir: String,

    /// Ledger path, relative to the runtime dir.
    /// Default "self_improve/ledger.jsonl".
    #[serde(default = "default_ledger_path")]
    pub ledger_path: String,
}

fn default_cadence() -> String {
    "daily 03:30".to_string()
}
fn default_proposals_per_week() -> u32 {
    3
}
fn default_open_per_signature() -> u32 {
    1
}
fn default_cycle_minutes() -> u64 {
    60
}
fn default_diff_lines() -> usize {
    500
}
fn default_declined_cooldown_days() -> u64 {
    30
}
fn default_failed_eval_cooldown_days() -> u64 {
    7
}
fn default_max_worktrees() -> usize {
    1
}
fn default_triage_backend() -> String {
    "local".to_string()
}
fn default_draft_backend() -> String {
    "gemini".to_string()
}
fn default_stop_file() -> String {
    "SELF_IMPROVE_OFF".to_string()
}
fn default_worktree_dir() -> String {
    "self_improve/worktrees".to_string()
}
fn default_ledger_path() -> String {
    "self_improve/ledger.jsonl".to_string()
}

impl Default for SelfImproveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            phase: 0,
            cadence: default_cadence(),
            max_proposals_per_week: default_proposals_per_week(),
            max_open_per_signature: default_open_per_signature(),
            max_cycle_minutes: default_cycle_minutes(),
            max_diff_lines: default_diff_lines(),
            declined_cooldown_days: default_declined_cooldown_days(),
            failed_eval_cooldown_days: default_failed_eval_cooldown_days(),
            max_worktrees: default_max_worktrees(),
            triage_backend: default_triage_backend(),
            draft_backend: default_draft_backend(),
            pr_opening: false,
            eval_set_approved: false,
            stop_file: default_stop_file(),
            worktree_dir: default_worktree_dir(),
            ledger_path: default_ledger_path(),
        }
    }
}

impl SelfImproveConfig {
    /// Load the top-level `[self_improve]` section from a full TOML
    /// string. Missing section (or unparseable input, or a wrong-typed
    /// value anywhere in the section) yields the fail-closed defaults.
    /// Fallbacks are logged (task 265) via `config::load_section`.
    pub fn load_from_toml(toml_str: &str) -> Self {
        crate::config::load_section(toml_str, "[self_improve]", |v| {
            v.get("self_improve").cloned()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_section_yields_all_fail_closed_defaults() {
        // GIVEN no [self_improve] section at all:
        // WHEN loading, THEN every fail-closed default holds.
        let cfg = SelfImproveConfig::load_from_toml("[helix]\nname = \"x\"\n");
        assert!(!cfg.enabled);
        assert_eq!(cfg.phase, 0);
        assert_eq!(cfg.cadence, "daily 03:30");
        assert_eq!(cfg.max_proposals_per_week, 3);
        assert_eq!(cfg.max_open_per_signature, 1);
        assert_eq!(cfg.max_cycle_minutes, 60);
        assert_eq!(cfg.max_diff_lines, 500);
        assert_eq!(cfg.declined_cooldown_days, 30);
        assert_eq!(cfg.failed_eval_cooldown_days, 7);
        assert_eq!(cfg.max_worktrees, 1);
        assert_eq!(cfg.triage_backend, "local");
        assert_eq!(cfg.draft_backend, "gemini");
        assert!(!cfg.pr_opening);
        assert!(!cfg.eval_set_approved);
        assert_eq!(cfg.stop_file, "SELF_IMPROVE_OFF");
        assert_eq!(cfg.worktree_dir, "self_improve/worktrees");
        assert_eq!(cfg.ledger_path, "self_improve/ledger.jsonl");
    }

    #[test]
    fn partial_section_overrides_only_given_keys() {
        // GIVEN a section naming two keys, THEN only those change.
        let cfg = SelfImproveConfig::load_from_toml(
            "[self_improve]\nenabled = true\nmax_proposals_per_week = 5\n",
        );
        assert!(cfg.enabled);
        assert_eq!(cfg.max_proposals_per_week, 5);
        // Everything else keeps fail-closed defaults.
        assert_eq!(cfg.phase, 0);
        assert!(!cfg.pr_opening);
        assert!(!cfg.eval_set_approved);
        assert_eq!(cfg.max_diff_lines, 500);
    }

    #[test]
    fn malformed_value_falls_back_to_whole_section_defaults() {
        // GIVEN a wrong-typed value anywhere in the section, THEN the
        // whole section falls back (load_section cannot trust a
        // partially-parsed section) — including the fail-closed flags.
        let cfg = SelfImproveConfig::load_from_toml(
            "[self_improve]\nenabled = true\nphase = \"banana\"\n",
        );
        assert!(!cfg.enabled);
        assert_eq!(cfg.phase, 0);
        assert!(!cfg.pr_opening);
        assert!(!cfg.eval_set_approved);
    }

    #[test]
    fn garbage_input_yields_defaults() {
        let cfg = SelfImproveConfig::load_from_toml("not toml ][[[");
        assert!(!cfg.enabled);
        assert_eq!(cfg.phase, 0);
        assert!(!cfg.pr_opening);
        assert!(!cfg.eval_set_approved);
    }

    #[test]
    fn full_section_fixture_round_trip() {
        // GIVEN every key set (fixture TOML), THEN every key parses.
        let cfg = SelfImproveConfig::load_from_toml(
            "[self_improve]\nenabled = true\nphase = 2\ncadence = \"daily 03:30\"\nmax_proposals_per_week = 2\nmax_open_per_signature = 1\nmax_cycle_minutes = 30\nmax_diff_lines = 250\ndeclined_cooldown_days = 14\nfailed_eval_cooldown_days = 3\nmax_worktrees = 1\ntriage_backend = \"local\"\ndraft_backend = \"gemini\"\npr_opening = false\neval_set_approved = false\nstop_file = \"SELF_IMPROVE_OFF\"\nworktree_dir = \"self_improve/worktrees\"\nledger_path = \"self_improve/ledger.jsonl\"\n",
        );
        assert!(cfg.enabled);
        assert_eq!(cfg.phase, 2);
        assert_eq!(cfg.max_proposals_per_week, 2);
        assert_eq!(cfg.max_cycle_minutes, 30);
        assert_eq!(cfg.max_diff_lines, 250);
        assert_eq!(cfg.declined_cooldown_days, 14);
        assert_eq!(cfg.failed_eval_cooldown_days, 3);
        assert!(!cfg.pr_opening);
        assert!(!cfg.eval_set_approved);
    }
}
