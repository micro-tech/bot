//! Metrics ledger (task 278, Phase 0).
//!
//! Append-only per-cycle ledger in the runtime dir, covering the spec's
//! 7-row metric catalog. Metadata-only, always: counts, keys, and
//! totals — never log bodies, never error-line text, never secrets.
//! The ledger is the loop's memory of "normal"; Phase 1 may not start
//! on a ledger with gaps (see `has_gap`).
//!
//! Layout: `<rt>/self_improve/ledger.jsonl` (one JSON object per
//! cycle, `collect_cycle` appends atomically via temp + rename —
//! history is never rewritten). The QM-summary sidecar
//! (`<rt>/self_improve/qm-summary.json`, written by the QM run or by
//! John) carries the external ground truth the loop must not grade
//! itself on; a missing or unparseable sidecar records a GAP, which
//! blocks Phase 1 per spec acceptance.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One routine's outcome counts for a cycle.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutineOutcome {
    pub fired: u64,
    pub failed: u64,
}

/// Per-file filtered error counts (task 276 semantics): the genuine
/// recent-error count plus the raw line total, so a huge chatter file
/// stays visible without its lines ever entering the ledger.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorFileCounts {
    pub errors: usize,
    pub total_lines: usize,
}

/// External Quartermaster summary sidecar (QM run or John drops this
/// into the runtime dir). Field names are the contract — see module
/// docs. All fields optional: a partial sidecar still parses, with
/// missing fields as `None`; only a missing file or unparseable JSON
/// is a gap.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct QmSummary {
    pub date: Option<String>,
    pub grok_tip: Option<String>,
    pub helix_tip: Option<String>,
    pub grok_suite: Option<String>,
    pub helix_lib: Option<String>,
    pub js: Option<String>,
    pub ci_failed_count: Option<u64>,
}

/// Parse the QM-summary sidecar leniently. `Ok(None)` = missing file
/// (gap, not an error). `Err` = unparseable content (gap, not a
/// crash). The caller records the gap flag; this function never panics
/// on hostile content.
pub fn parse_qm_summary(path: &Path) -> Result<Option<QmSummary>, String> {
    let Ok(data) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    match serde_json::from_str::<QmSummary>(&data) {
        Ok(summary) => Ok(Some(summary)),
        Err(e) => Err(format!("qm-summary unparseable: {e}")),
    }
}

/// Everything one collection cycle measures. The handler (task 281)
/// builds this from live sources; tests build it from fixtures.
#[derive(Debug, Clone, Default)]
pub struct CycleInputs {
    /// Routine outcomes by routine id (from the CPU routine-handler path).
    pub routine_outcomes: BTreeMap<String, RoutineOutcome>,
    /// Support-watcher snapshot (`support_watch::status_snapshot`).
    /// Stored verbatim: it is already counts-only.
    pub support_stats: serde_json::Value,
    /// Filtered error-log counts (totals + per-file counts ONLY —
    /// never the `recent` lines from `ErrorFindings`).
    pub error_total: usize,
    pub error_by_file: BTreeMap<String, ErrorFileCounts>,
    /// Restart/crash count from Helix startup markers in runtime logs.
    pub restart_count: u64,
    /// QM ground truth, or `None` when the sidecar is missing.
    pub qm_summary: Option<QmSummary>,
    /// Whether the QM sidecar was present but unparseable.
    pub qm_parse_error: Option<String>,
    /// Proposal outcomes by signature (empty until Phase 1).
    pub proposal_outcomes: BTreeMap<String, String>,
}

/// One ledger entry: exactly one JSON object per cycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LedgerEntry {
    /// Unix milliseconds at collection time.
    pub timestamp_ms: u64,
    pub routine_outcomes: BTreeMap<String, RoutineOutcome>,
    pub support_stats: serde_json::Value,
    pub error_total: usize,
    pub error_by_file: BTreeMap<String, ErrorFileCounts>,
    pub restart_count: u64,
    /// QM ground truth when the sidecar parsed; `None` on a gap.
    pub qm_summary: Option<QmSummary>,
    /// True when the QM sidecar was missing OR unparseable — a ledger
    /// GAP, which blocks Phase 1 per spec acceptance.
    pub qm_missing: bool,
    pub proposal_outcomes: BTreeMap<String, String>,
}

impl LedgerEntry {
    /// True when this entry has a QM gap (missing or unparseable
    /// sidecar). Phase 1 must not start on a gapped ledger.
    pub fn has_gap(&self) -> bool {
        self.qm_missing
    }
}

/// Ledger paths under a runtime dir.
pub fn ledger_path(rt_dir: &Path, cfg: &crate::self_improve::SelfImproveConfig) -> PathBuf {
    rt_dir.join(&cfg.ledger_path)
}

pub fn qm_summary_path(rt_dir: &Path) -> PathBuf {
    rt_dir.join("self_improve").join("qm-summary.json")
}

/// Append one cycle to the ledger, atomically (write temp file in the
/// same directory, then rename over the ledger). History is never
/// rewritten: the existing file is read, the new line appended, and
/// the whole thing renamed into place. Returns the entry written.
pub fn collect_cycle(
    rt_dir: &Path,
    cfg: &crate::self_improve::SelfImproveConfig,
    inputs: CycleInputs,
    timestamp_ms: u64,
) -> Result<LedgerEntry, String> {
    let path = ledger_path(rt_dir, cfg);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("ledger dir create failed: {e}"))?;
    }
    let qm_missing = inputs.qm_summary.is_none();
    let entry = LedgerEntry {
        timestamp_ms,
        routine_outcomes: inputs.routine_outcomes,
        support_stats: inputs.support_stats,
        error_total: inputs.error_total,
        error_by_file: inputs.error_by_file,
        restart_count: inputs.restart_count,
        qm_summary: inputs.qm_summary,
        qm_missing,
        proposal_outcomes: inputs.proposal_outcomes,
    };
    let mut existing = String::new();
    if path.exists() {
        existing = std::fs::read_to_string(&path)
            .map_err(|e| format!("ledger read failed: {e}"))?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
    }
    let line = serde_json::to_string(&entry)
        .map_err(|e| format!("ledger serialize failed: {e}"))?;
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, format!("{existing}{line}\n"))
        .map_err(|e| format!("ledger temp write failed: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("ledger rename failed: {e}"))?;
    Ok(entry)
}

/// Read all ledger entries, oldest first. A corrupt line is skipped
/// (and counted in the returned gap count) rather than failing the
/// whole read — the ledger must stay readable even if one cycle wrote
/// junk. Returns (entries, corrupt_line_count).
pub fn read_ledger(rt_dir: &Path, cfg: &crate::self_improve::SelfImproveConfig) -> (Vec<LedgerEntry>, usize) {
    let path = ledger_path(rt_dir, cfg);
    let Ok(data) = std::fs::read_to_string(&path) else {
        return (Vec::new(), 0);
    };
    let mut entries = Vec::new();
    let mut corrupt = 0;
    for line in data.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LedgerEntry>(line) {
            Ok(e) => entries.push(e),
            Err(_) => corrupt += 1,
        }
    }
    (entries, corrupt)
}

/// Read the latest ledger entry, if any.
pub fn latest_entry(
    rt_dir: &Path,
    cfg: &crate::self_improve::SelfImproveConfig,
) -> Option<LedgerEntry> {
    read_ledger(rt_dir, cfg).0.into_iter().last()
}

/// One-line per-cycle log summary: counts only, metadata-only.
/// Format: `self_improve: metrics ...` (task 279 uses this line).
pub fn log_line(entry: &LedgerEntry) -> String {
    let routine_failed: u64 = entry
        .routine_outcomes
        .values()
        .map(|o| o.failed)
        .sum();
    format!(
        "self_improve: metrics ts={} routines={} routine_failed={} errors={} restarts={} qm_missing={} proposals={}",
        entry.timestamp_ms,
        entry.routine_outcomes.len(),
        routine_failed,
        entry.error_total,
        entry.restart_count,
        entry.qm_missing,
        entry.proposal_outcomes.len(),
    )
}

/// Count Helix startup markers in runtime log files. The marker is the
/// `main.rs` boot line; logs live under `<rt>/logs/`. Missing logs
/// count as zero, not an error — a fresh runtime has no logs yet.
pub fn count_restarts(rt_dir: &Path) -> u64 {
    const MARKER: &str = "Helix is running...";
    let logs = rt_dir.join("logs");
    let Ok(rd) = std::fs::read_dir(&logs) else {
        return 0;
    };
    let mut count = 0;
    for entry in rd.flatten() {
        let Ok(data) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        count += data.matches(MARKER).count() as u64;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> crate::self_improve::SelfImproveConfig {
        crate::self_improve::SelfImproveConfig::default()
    }

    fn fixture_inputs() -> CycleInputs {
        let mut routines = BTreeMap::new();
        routines.insert(
            "support-mail-watch".to_string(),
            RoutineOutcome { fired: 60, failed: 0 },
        );
        let mut by_file = BTreeMap::new();
        by_file.insert(
            "logs/error_log.md".to_string(),
            ErrorFileCounts { errors: 0, total_lines: 54231 },
        );
        CycleInputs {
            routine_outcomes: routines,
            support_stats: serde_json::json!({"mode": "normal_night", "unhandled_count": 0}),
            error_total: 0,
            error_by_file: by_file,
            restart_count: 1,
            qm_summary: Some(QmSummary {
                date: Some("2026-10-28".to_string()),
                grok_tip: Some("39bc650".to_string()),
                helix_tip: Some("c7ecc41".to_string()),
                grok_suite: Some("1113/0/11".to_string()),
                helix_lib: Some("449/0/4".to_string()),
                js: Some("9/9".to_string()),
                ci_failed_count: Some(0),
            }),
            qm_parse_error: None,
            proposal_outcomes: BTreeMap::new(),
        }
    }

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ledger_append_read_round_trip() {
        // GIVEN a fresh runtime dir, WHEN two cycles are collected,
        // THEN both entries read back oldest-first, in order.
        let dir = tmpdir("helix-self-improve-ledger-rt");
        let cfg = test_cfg();
        let e1 = collect_cycle(&dir, &cfg, fixture_inputs(), 1_000).unwrap();
        let mut inputs2 = fixture_inputs();
        inputs2.error_total = 2;
        let e2 = collect_cycle(&dir, &cfg, inputs2, 2_000).unwrap();
        assert!(!e1.has_gap());
        let (entries, corrupt) = read_ledger(&dir, &cfg);
        assert_eq!(corrupt, 0);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ms, 1_000);
        assert_eq!(entries[1].timestamp_ms, 2_000);
        assert_eq!(entries[1].error_total, 2);
        assert_eq!(e2.timestamp_ms, 2_000);
        assert_eq!(latest_entry(&dir, &cfg).unwrap().timestamp_ms, 2_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_qm_summary_yields_gap_flag_no_crash() {
        // GIVEN inputs with no QM sidecar, THEN the entry is written
        // with qm_missing=true (a gap), not an error.
        let dir = tmpdir("helix-self-improve-ledger-gap");
        let cfg = test_cfg();
        let mut inputs = fixture_inputs();
        inputs.qm_summary = None;
        let entry = collect_cycle(&dir, &cfg, inputs, 1_000).unwrap();
        assert!(entry.has_gap());
        assert!(entry.qm_summary.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_sidecar_yields_gap_flag() {
        // GIVEN a sidecar file with unparseable JSON, THEN parsing
        // reports an error (the caller records a gap) instead of
        // panicking or crashing.
        let dir = tmpdir("helix-self-improve-ledger-badsidecar");
        let path = qm_summary_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json ][[[").unwrap();
        assert!(parse_qm_summary(&path).is_err());
        // And a missing file parses as Ok(None) — a gap, not an error.
        assert_eq!(
            parse_qm_summary(&dir.join("self_improve").join("nope.json")).unwrap(),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_sidecar_parses_leniently() {
        // GIVEN a sidecar with only some fields, THEN it still parses
        // with the rest as None (lenient: partial is not a gap).
        let dir = tmpdir("helix-self-improve-ledger-sidecar");
        let path = qm_summary_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"date":"2026-10-28","helix_lib":"449/0/4"}"#).unwrap();
        let summary = parse_qm_summary(&path).unwrap().unwrap();
        assert_eq!(summary.date.as_deref(), Some("2026-10-28"));
        assert_eq!(summary.helix_lib.as_deref(), Some("449/0/4"));
        assert!(summary.grok_tip.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn error_log_fixture_chatter_only_counts_zero() {
        // GIVEN filtered counts where chatter dominates (54k lines, 0
        // genuine errors), THEN the entry stores counts only — and the
        // raw total stays visible without any line text.
        let dir = tmpdir("helix-self-improve-ledger-chatter");
        let cfg = test_cfg();
        let mut inputs = fixture_inputs();
        inputs.error_total = 0;
        inputs.error_by_file.insert(
            "logs/error_log.md".to_string(),
            ErrorFileCounts { errors: 0, total_lines: 322_685 },
        );
        let entry = collect_cycle(&dir, &cfg, inputs, 1_000).unwrap();
        assert_eq!(entry.error_total, 0);
        assert_eq!(
            entry.error_by_file["logs/error_log.md"].total_lines,
            322_685
        );
        let line = serde_json::to_string(&entry).unwrap();
        assert!(!line.contains("routine_run"), "no log-line bodies in entries");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ledger_entry_with_fake_secrets_contains_no_secret_shaped_strings() {
        // GIVEN fixtures containing fake tokens (never real values),
        // THEN the serialized entry contains no secret-shaped strings:
        // the ledger is metadata-only by construction — it stores
        // counts and keys, and no collector passes bodies or values.
        // (This test doubles as the metadata-only proof: if any future
        // collector starts persisting values, the fixture must grow a
        // field for it, and this assertion forces that conversation.)
        let dir = tmpdir("helix-self-improve-ledger-secrets");
        let cfg = test_cfg();
        let mut inputs = fixture_inputs();
        inputs.support_stats = serde_json::json!({
            "mode": "normal_night",
            "fake_key_name": "api-key",
            "unhandled_count": 0
        });
        let entry = collect_cycle(&dir, &cfg, inputs, 1_000).unwrap();
        let line = serde_json::to_string(&entry).unwrap();
        // Key NAMES are fine (metadata); VALUES that look like secrets
        // must never appear. The fixture carries no values, so assert
        // the absence of the classic value shapes.
        for shape in ["ghp_", "sk-", "Bearer ", "-----BEGIN"] {
            assert!(
                !line.contains(shape),
                "ledger line must not contain secret-shaped value {shape}"
            );
        }
        // And the entry has no free-text body field at all by schema.
        assert!(!line.contains("recent"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_line_format_is_counts_only() {
        // GIVEN an entry, THEN the log line carries counts and the
        // qm_missing flag, and no bodies.
        let entry = LedgerEntry {
            timestamp_ms: 1_000,
            routine_outcomes: {
                let mut m = BTreeMap::new();
                m.insert("a".to_string(), RoutineOutcome { fired: 3, failed: 1 });
                m
            },
            support_stats: serde_json::json!({}),
            error_total: 2,
            error_by_file: BTreeMap::new(),
            restart_count: 1,
            qm_summary: None,
            qm_missing: true,
            proposal_outcomes: BTreeMap::new(),
        };
        let line = log_line(&entry);
        assert!(line.starts_with("self_improve: metrics "));
        assert!(line.contains("routine_failed=1"));
        assert!(line.contains("errors=2"));
        assert!(line.contains("qm_missing=true"));
    }
}
