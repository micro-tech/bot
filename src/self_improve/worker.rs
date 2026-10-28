//! Loop worker: brief, tight allowlist, proposal record (task 282).
//!
//! The loop worker is a read-only investigator. It may look (repo,
//! logs, notes, status) and never touch: no file writes, no shell, no
//! mail, no chat, and absolutely no spawning (the depth guard is
//! absolute — an allowlist can never re-add `spawn_subagent`).
//!
//! Two layers enforce this, and they fail differently on purpose:
//! 1. The persona file `agents/loop-worker.md` (shipped in the repo,
//!    seeded into the runtime persona dir by the handler before any
//!    dispatch, never overwritten once present) lists the 8 tools in
//!    frontmatter, so the general persona mechanism filters to them.
//! 2. `loop_worker_tool_definitions()` — interposed in `run_subagent`
//!    for role `loop-worker` ONLY — intersects whatever the persona
//!    mechanism returned with `LOOP_WORKER_TOOLS` and FAILS CLOSED on
//!    a zero match. This is a deliberate tightening versus the general
//!    mechanism, which falls back to the full toolset with a warning
//!    on zero match: for the loop worker, a typo must never silently
//!    grant the full toolset.
//!
//! Known limit (documented, not hidden): the Gemini draft path
//! (`call_gemini_direct`) takes no tool definitions today, so on Gemini
//! the binding constraint is the brief + persona text, not the tool
//! filter. The filter still runs before backend dispatch — a zero-match
//! persona refuses the spawn on ANY backend — and it fully binds the
//! Ollama tool path (`chat_with_tools`).
//!
//! The worker writes NO files via tools: proposal records and diffs are
//! written handler-side (`intake_proposal`, the evaluator), because no
//! Helix tool writes `proposals.json`. This is strictly tighter than
//! the spec's allowance and is the shipped behavior.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The loop worker's entire tool universe: read-only, nothing else.
/// `spawn_subagent`, `run_shell`, and `send_email` are excluded by
/// construction (they are not in this list, so no filter input can
/// re-add them).
pub const LOOP_WORKER_TOOLS: [&str; 8] = [
    "repo_read",
    "repo_glob",
    "repo_grep",
    "read_log",
    "read_note",
    "list_notes",
    "system_status",
    "scan_directory",
];

/// Persona file content, baked in from the repo source of truth
/// (`agents/loop-worker.md`). The handler seeds this into the runtime
/// persona dir when missing; John's edits to the seeded copy always win.
pub const LOOP_WORKER_PERSONA: &str = include_str!("../../agents/loop-worker.md");

/// Intersect persona-filtered tool definitions with the loop allowlist.
/// Returns Err on a zero match — fail-closed, never the full toolset.
/// `spawn_subagent` is excluded even if present in the input.
pub fn filter_loop_worker_tools(defs: &serde_json::Value) -> Result<serde_json::Value, String> {
    let arr = defs.as_array().cloned().unwrap_or_default();
    let filtered: Vec<serde_json::Value> = arr
        .into_iter()
        .filter(|d| {
            let name = d
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            name != "spawn_subagent" && LOOP_WORKER_TOOLS.contains(&name)
        })
        .collect();
    if filtered.is_empty() {
        return Err(
            "loop-worker allowlist matched zero known tools; refusing to fall back \
             to the full toolset (fail-closed — fix agents/loop-worker.md)"
                .to_string(),
        );
    }
    Ok(serde_json::Value::Array(filtered))
}

/// The fail-closed tool definitions for a `loop-worker` spawn: the
/// persona mechanism's output, intersected with the allowlist.
pub fn loop_worker_tool_definitions() -> Result<serde_json::Value, String> {
    let defs = crate::persona::subagent_tool_definitions("loop-worker");
    filter_loop_worker_tools(&defs)
}

/// Ensure the loop-worker persona file exists in the runtime persona
/// dir (`<rt>/agents/loop-worker.md`). Seeds the shipped content when
/// missing; NEVER overwrites an existing file (John's edits win).
/// This is runtime-dir state seeding (like routines.json), not a
/// live-tree write — the runtime dir is not the repo checkout.
pub fn ensure_loop_worker_persona(rt_dir: &Path) -> Result<PathBuf, String> {
    let path = rt_dir.join("agents").join("loop-worker.md");
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("persona dir create failed: {e}"))?;
    }
    std::fs::write(&path, LOOP_WORKER_PERSONA)
        .map_err(|e| format!("persona seed failed: {e}"))?;
    Ok(path)
}

/// The worker brief for one cycle (task 282.1). The handler (task 281)
/// fills in the cycle's worst signal; the worker returns a structured
/// proposal — it writes nothing itself.
pub fn build_loop_brief(signal: &str) -> String {
    format!(
        "Helix self-improvement loop worker (Phase 1, proposal-only). Worst signal this cycle: {signal}. Investigate read-only with your tools (repo, logs, notes, status only — you cannot write files, run shell commands, send mail, or spawn sub-agents). Return a proposal with exactly these fields: problem_statement, trace (the failing test name OR the metric key behind this signal — REQUIRED, an exact key, never free-text hearsay), diff_description (what would change; no branch yet), proving_test (the test that would prove the fix). You write no files and change nothing; the handler records your proposal."
    )
}

// ── Proposal record (task 282.2) ──────────────────────────────────────────

/// A proposal as the worker submits it (before intake derives the rest).
#[derive(Debug, Clone, Default)]
pub struct NewProposal {
    pub problem_statement: String,
    pub trace: String,
    pub diff_description: String,
    pub proving_test: String,
}

/// A stored proposal record. Lives in `rt/self_improve/proposals.json`
/// (a JSON array) — NOT the repo `.zed/task_list.json`: writing the
/// repo task list would be a live-tree write. "Task-list-shaped" means
/// the shape (problem, trace, test, status), not the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalRecord {
    pub problem_statement: String,
    pub trace: String,
    pub diff_description: String,
    pub proving_test: String,
    /// Stable dedupe key: the normalized (trimmed) trace. Two proposals
    /// about the same failing signal share a signature (task 283).
    pub signature: String,
    /// Lifecycle: "proposed" at intake; later "evaluating", "queued",
    /// "promoted", "declined", "needs-human-review", "withdrawn".
    pub status: String,
    /// Unix milliseconds at intake.
    pub created_at: u64,
    /// Unix milliseconds of the last status change (intake time at
    /// creation). Cooldowns (task 283) run from this timestamp, not
    /// from intake: a proposal declined months after intake is still
    /// cooled from its decline, not its birth.
    pub updated_at: u64,
    /// Budget refs at intake (enforced by task 283): proposals already
    /// recorded in the last 7 days, and open proposals for this
    /// signature, at the moment of intake.
    pub proposals_this_week: usize,
    pub open_for_signature: usize,
    /// Evaluation outcome (task 287; default = unevaluated). The full
    /// report lives in `eval_report_json` as JSON so this record does
    /// not depend on the evaluator module's types.
    #[serde(default)]
    pub eval_passed: bool,
    #[serde(default)]
    pub promoted: bool,
    #[serde(default)]
    pub promotion_reason: String,
    #[serde(default)]
    pub eval_report_json: serde_json::Value,
}

const OPEN_STATUSES: [&str; 5] = [
    "proposed",
    "evaluating",
    "evaluated",
    "queued",
    "needs-human-review",
];

/// The set of traces intake accepts (R3). Metric keys come from the
/// latest ledger entry's live signals; test names come from prior
/// accepted proposals (the ledger carries counts, not test names).
pub struct TraceIndex {
    pub metric_keys: Vec<String>,
    pub known_test_names: Vec<String>,
}

/// Build the R3 index from the latest ledger entry (or `None` when no
/// cycle has been collected yet — then only test-shaped traces from
/// prior proposals can pass) plus already-stored proposals.
pub fn trace_index_from_ledger(
    latest: Option<&crate::self_improve::metrics::LedgerEntry>,
    prior: &[ProposalRecord],
) -> TraceIndex {
    let mut metric_keys = Vec::new();
    if let Some(entry) = latest {
        for (path, counts) in &entry.error_by_file {
            if counts.errors > 0 {
                metric_keys.push(format!("errors:{path}"));
            }
        }
        for (id, outcome) in &entry.routine_outcomes {
            if outcome.failed > 0 {
                metric_keys.push(format!("routine:{id}:failed"));
            }
        }
        if entry.restart_count > 0 {
            metric_keys.push("restarts".to_string());
        }
        if let Some(unhandled) = entry
            .support_stats
            .get("unhandled_count")
            .and_then(|v| v.as_u64())
        {
            if unhandled > 0 {
                metric_keys.push("support:unhandled".to_string());
            }
        }
    }
    let known_test_names = prior
        .iter()
        .filter(|p| is_test_shaped(&p.trace))
        .map(|p| p.trace.clone())
        .collect();
    TraceIndex { metric_keys, known_test_names }
}

/// A trace is test-shaped when it looks like a Rust test path:
/// `a::b::c` with non-empty identifier segments. This is intake's
/// shape check; existence of the test is verified by the Phase 2
/// evaluator, which actually runs it (a cited test that does not exist
/// fails evaluation — see task 287). Intake cannot verify existence
/// without running cargo, which is the evaluator's job, not intake's.
pub fn is_test_shaped(trace: &str) -> bool {
    let t = trace.trim();
    if !t.contains("::") {
        return false;
    }
    t.split("::").all(|seg| {
        !seg.is_empty()
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// R3 trace validation: empty/missing traces are rejected; a trace is
/// accepted when it is an exact known metric key, an exact known test
/// name, or (bootstrap case) a test-shaped name — free-text hearsay
/// ("someone said the login is slow") matches none of these and is
/// auto-rejected. See `is_test_shaped` for why test existence is the
/// evaluator's check, not intake's.
pub fn validate_trace(trace: &str, index: &TraceIndex) -> Result<(), String> {
    let t = trace.trim();
    if t.is_empty() {
        return Err("trace is required (missing/empty trace auto-rejected)".to_string());
    }
    if index.metric_keys.iter().any(|k| k == t) {
        return Ok(());
    }
    if index.known_test_names.iter().any(|k| k == t) {
        return Ok(());
    }
    if is_test_shaped(t) {
        return Ok(());
    }
    Err(format!(
        "trace does not match a known failing test name or metric key: {t}"
    ))
}

/// Proposals file under a runtime dir.
pub fn proposals_path(rt_dir: &Path) -> PathBuf {
    rt_dir.join("self_improve").join("proposals.json")
}

/// Read stored proposals. Missing file reads as empty; an unparseable
/// file is an error (intake must not silently append over corrupt data).
pub fn read_proposals(rt_dir: &Path) -> Result<Vec<ProposalRecord>, String> {
    let path = proposals_path(rt_dir);
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    serde_json::from_str::<Vec<ProposalRecord>>(&data)
        .map_err(|e| format!("proposals read failed: {e}"))
}

/// Upsert a proposal record by `created_at` (the intake-time identity):
/// replace the matching record, or append when absent. Atomic temp +
/// rename. Used by the evaluator to persist post-eval state.
pub fn upsert_proposal(rt_dir: &Path, record: &ProposalRecord) -> Result<(), String> {
    let mut all = read_proposals(rt_dir)?;
    if let Some(slot) = all.iter_mut().find(|p| p.created_at == record.created_at) {
        *slot = record.clone();
    } else {
        all.push(record.clone());
    }
    let path = proposals_path(rt_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("proposals dir create failed: {e}"))?;
    }
    let data = serde_json::to_string_pretty(&all)
        .map_err(|e| format!("proposals serialize failed: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("proposals temp write failed: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("proposals rename failed: {e}"))?;
    Ok(())
}

/// Validate a proposal (R3 + completeness) and append it to
/// `proposals.json` (atomic temp + rename). Returns the stored record.
/// Budget enforcement (caps, cooldowns, dedupe) is task 283's job —
/// intake records the budget refs but does not block on them.
pub fn intake_proposal(
    rt_dir: &Path,
    proposal: NewProposal,
    now_ms: u64,
) -> Result<ProposalRecord, String> {
    if proposal.problem_statement.trim().is_empty() {
        return Err("problem_statement is required".to_string());
    }
    if proposal.diff_description.trim().is_empty() {
        return Err("diff_description is required".to_string());
    }
    if proposal.proving_test.trim().is_empty() {
        return Err("proving_test is required".to_string());
    }
    let cfg = crate::self_improve::live_config_at(rt_dir);
    let latest = crate::self_improve::metrics::latest_entry(rt_dir, &cfg);
    let prior = read_proposals(rt_dir)?;
    let index = trace_index_from_ledger(latest.as_ref(), &prior);
    validate_trace(&proposal.trace, &index)?;
    let signature = proposal.trace.trim().to_string();
    let week_ago = now_ms.saturating_sub(7 * 24 * 60 * 60 * 1000);
    let proposals_this_week = prior.iter().filter(|p| p.created_at >= week_ago).count();
    let open_for_signature = prior
        .iter()
        .filter(|p| p.signature == signature && OPEN_STATUSES.contains(&p.status.as_str()))
        .count();
    let record = ProposalRecord {
        problem_statement: proposal.problem_statement,
        trace: proposal.trace.trim().to_string(),
        diff_description: proposal.diff_description,
        proving_test: proposal.proving_test,
        signature,
        status: "proposed".to_string(),
        created_at: now_ms,
        updated_at: now_ms,
        proposals_this_week,
        open_for_signature,
        eval_passed: false,
        promoted: false,
        promotion_reason: String::new(),
        eval_report_json: serde_json::Value::Null,
    };
    let mut all = prior;
    all.push(record.clone());
    let path = proposals_path(rt_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("proposals dir create failed: {e}"))?;
    }
    let data = serde_json::to_string_pretty(&all)
        .map_err(|e| format!("proposals serialize failed: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("proposals temp write failed: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("proposals rename failed: {e}"))?;
    Ok(record)
}

/// Parse a worker report into a structured proposal (Phase 1 path
/// completion: without this, the handler could never intake what the
/// worker returns). The brief asks for exactly these fields, one per
/// line, `key: value` (values may contain colons — split on the first).
/// A missing field is an error, never a partial proposal: the handler
/// logs it and intakes nothing.
pub fn parse_worker_report(report: &str) -> Result<NewProposal, String> {
    let mut fields = std::collections::HashMap::new();
    for line in report.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if matches!(
                key,
                "problem_statement" | "trace" | "diff_description" | "proving_test"
            ) {
                fields
                    .entry(key.to_string())
                    .or_insert_with(|| value.trim().to_string());
            }
        }
    }
    let get = |key: &str| -> Result<String, String> {
        fields
            .get(key)
            .filter(|v| !v.is_empty())
            .cloned()
            .ok_or_else(|| format!("worker report missing field: {key}"))
    };
    Ok(NewProposal {
        problem_statement: get("problem_statement")?,
        trace: get("trace")?,
        diff_description: get("diff_description")?,
        proving_test: get("proving_test")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::self_improve::metrics::*;

    fn defs(names: &[&str]) -> serde_json::Value {
        serde_json::Value::Array(
            names
                .iter()
                .map(|n| serde_json::json!({"type": "function", "function": {"name": n}}))
                .collect(),
        )
    }

    fn def_names(v: &serde_json::Value) -> Vec<String> {
        v.as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|d| {
                d.get("function")?
                    .get("name")?
                    .as_str()
                    .map(|s| s.to_string())
            })
            .collect()
    }

    #[test]
    fn allowlist_filters_to_exactly_the_permitted_set() {
        // GIVEN definitions including the 8 permitted tools plus
        // spawn_subagent, run_shell, send_email, and an unknown tool,
        // WHEN filtered, THEN exactly the 8 permitted tools remain.
        let input = defs(&[
            "repo_read",
            "repo_glob",
            "repo_grep",
            "read_log",
            "read_note",
            "list_notes",
            "system_status",
            "scan_directory",
            "spawn_subagent",
            "run_shell",
            "send_email",
            "bogus_tool",
        ]);
        let out = filter_loop_worker_tools(&input).unwrap();
        let mut names = def_names(&out);
        names.sort();
        let mut expected: Vec<String> =
            LOOP_WORKER_TOOLS.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(names, expected);
    }

    #[test]
    fn spawn_subagent_can_never_be_re_added() {
        // GIVEN definitions containing spawn_subagent alongside a
        // permitted tool, THEN spawn_subagent is absent from the output.
        let out = filter_loop_worker_tools(&defs(&["spawn_subagent", "repo_read"])).unwrap();
        assert!(!def_names(&out).contains(&"spawn_subagent".to_string()));
        assert!(def_names(&out).contains(&"repo_read".to_string()));
    }

    #[test]
    fn zero_match_fails_closed_instead_of_falling_back() {
        // GIVEN definitions with no permitted tool (unknown names only,
        // or empty), WHEN filtered, THEN Err — never the full toolset.
        // (Deliberate tightening vs the general persona mechanism.)
        assert!(filter_loop_worker_tools(&defs(&["bogus_tool"])).is_err());
        assert!(filter_loop_worker_tools(&serde_json::Value::Array(vec![])).is_err());
    }

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_ledger_with_error(dir: &Path) {
        // One ledger cycle with a genuine error signal, so the metric
        // key "errors:logs/error_log.md" is known to intake.
        let cfg = crate::self_improve::SelfImproveConfig::default();
        let mut inputs = CycleInputs::default();
        inputs.error_total = 3;
        inputs.error_by_file.insert(
            "logs/error_log.md".to_string(),
            ErrorFileCounts { errors: 3, total_lines: 100 },
        );
        collect_cycle(dir, &cfg, inputs, 1_000).unwrap();
    }

    fn good_proposal(trace: &str) -> NewProposal {
        NewProposal {
            problem_statement: "Error log review counts chatter".to_string(),
            trace: trace.to_string(),
            diff_description: "Filter counting to genuine recent errors".to_string(),
            proving_test: "cpu::tests::error_log_check_reports_findings_on_fixture".to_string(),
        }
    }

    #[test]
    fn intake_accepts_metric_traced_and_test_traced_proposals() {
        // GIVEN a ledger with an error signal, WHEN intaking a proposal
        // traced to that metric key AND one traced to a test-shaped
        // name, THEN both are stored as "proposed" with signature =
        // trace and correct budget refs.
        let dir = tmpdir("helix-self-improve-intake-accept");
        seed_ledger_with_error(&dir);
        let r1 = intake_proposal(&dir, good_proposal("errors:logs/error_log.md"), 2_000).unwrap();
        assert_eq!(r1.status, "proposed");
        assert_eq!(r1.signature, "errors:logs/error_log.md");
        // Budget refs describe state AT intake (prior proposals only):
        // the first proposal sees 0 already recorded this week.
        assert_eq!(r1.proposals_this_week, 0);
        assert_eq!(r1.open_for_signature, 0);
        let r2 = intake_proposal(
            &dir,
            good_proposal("cpu::nightly_tests::error_log_check_reports_findings_on_fixture"),
            3_000,
        )
        .unwrap();
        assert_eq!(r2.status, "proposed");
        assert_eq!(r2.proposals_this_week, 1);
        let stored = read_proposals(&dir).unwrap();
        assert_eq!(stored.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn intake_rejects_message_content_traced_and_missing_trace() {
        // GIVEN the same ledger, WHEN intaking a proposal traced to
        // message content ("someone said...") or with a missing/empty
        // trace, THEN intake rejects it and stores nothing.
        let dir = tmpdir("helix-self-improve-intake-reject");
        seed_ledger_with_error(&dir);
        assert!(intake_proposal(&dir, good_proposal("someone said the login is slow"), 2_000).is_err());
        assert!(intake_proposal(&dir, good_proposal(""), 2_000).is_err());
        assert!(intake_proposal(&dir, good_proposal("   "), 2_000).is_err());
        assert!(read_proposals(&dir).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn proposals_json_round_trip() {
        // GIVEN an intake, WHEN reading back, THEN the record survives
        // with all fields intact.
        let dir = tmpdir("helix-self-improve-intake-roundtrip");
        seed_ledger_with_error(&dir);
        let r = intake_proposal(&dir, good_proposal("errors:logs/error_log.md"), 2_000).unwrap();
        let stored = read_proposals(&dir).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0], r);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persona_seed_writes_once_and_never_overwrites() {
        // GIVEN an empty runtime dir, WHEN seeding twice with a manual
        // edit in between, THEN the first seed writes the shipped
        // content and the second leaves the edit untouched.
        let dir = tmpdir("helix-self-improve-persona-seed");
        let path = ensure_loop_worker_persona(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            LOOP_WORKER_PERSONA
        );
        std::fs::write(&path, "---\ntools: [repo_read]\n---\nJohn's edit.\n").unwrap();
        let path2 = ensure_loop_worker_persona(&dir).unwrap();
        assert_eq!(path, path2);
        assert_eq!(
            std::fs::read_to_string(&path2).unwrap(),
            "---\ntools: [repo_read]\n---\nJohn's edit.\n",
            "seed must never overwrite an existing persona file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worker_report_parses_fields_and_rejects_partial_reports() {
        // GIVEN a well-formed worker report, WHEN parsed, THEN all four
        // fields come out (values may contain colons); a report missing
        // a field errors — never a partial proposal.
        let report = "problem_statement: Error log counts chatter\ntrace: errors:logs/error_log.md\ndiff_description: Filter to genuine recent errors: see src/cpu/mod.rs\nproving_test: cpu::nightly_tests::error_log_check_reports_findings_on_fixture\n";
        let p = parse_worker_report(report).unwrap();
        assert_eq!(p.problem_statement, "Error log counts chatter");
        assert_eq!(p.trace, "errors:logs/error_log.md");
        assert_eq!(
            p.diff_description,
            "Filter to genuine recent errors: see src/cpu/mod.rs"
        );
        assert!(parse_worker_report(
            "problem_statement: x\ntrace: errors:logs/error_log.md\ndiff_description: y\n"
        )
        .is_err());
    }
}
