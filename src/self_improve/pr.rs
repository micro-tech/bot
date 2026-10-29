//! PR-opening code path (task 290, Phase 3) — default OFF, queue-only.
//!
//! The shipped state is queue-only: with `[self_improve] pr_opening =
//! false` (the default), `request_pr_open` logs `pr-opening-disabled`
//! and makes ZERO backend calls — the proposal stays as it was
//! (`evaluated`, awaiting a future open), never dropped, never opened.
//!
//! Why queue-only: the R630 Helix has no in-crate GitHub API client,
//! so the production backend is a `QueueBackend` that appends the PR
//! request to `rt/self_improve/pr_queue.jsonl`. The payload (eval
//! report + diff stat + budget/cooldown refs, label `self-improve`,
//! target allowlist `["PreRelese"]` only) is what gets built and
//! gated here; actual GitHub opening from the queue is an external
//! step (the queue consumer), which is also where the connector
//! capability applies.
//!
//! Connector capability (verified 2026-10-28 via live `list-tools`,
//! 46 tools): `create_pull_request` EXISTS (the open path) and
//! `update_pull_request` EXISTS with `state` enum `["open","closed"]`
//! — so closing a PR is a real call (`state=closed`), and the
//! withdrawal path below uses it via `close_pr`. `merge_pull_request`
//! also exists and the loop must NEVER call it.
//!
//! The Phase 3 gate (all must hold, else refuse — never a partial open):
//! 1. `pr_opening` true (else `pr-opening-disabled`).
//! 2. Target in the allowlist: `PreRelese` only; `master` is rejected
//!    by construction (the payload builder errors on any other target).
//! 3. Full eval gate passed: `promoted`, `eval_passed`, a non-null eval
//!    report, AND the D5 gate currently open (`eval_set_approved` — a
//!    proposal promoted while the gate was open must not open after
//!    John closes the gate again).
//! 4. Clean audit: any secret hit or sensitive path refuses, forever
//!    for that proposal — sensitive-path proposals are queue-only
//!    forever; John opens any PR himself.
//!
//! Withdrawal (290.2): `withdraw_proposal` closes via the backend
//! when a PR number is known (queueing a close action under
//! `QueueBackend`), marks the record `withdrawn`, and appends the
//! regression note — so a failed close still leaves a manual-close
//! note for John instead of a silent open PR.

use crate::self_improve::audit::{AuditReport, audit_outcome, AuditOutcome};
use crate::self_improve::evaluator::EvalReport;
use crate::self_improve::worker::{ProposalRecord, upsert_proposal};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Target branches a loop PR may ever point at. One entry, by design.
pub const PR_TARGET_ALLOWLIST: [&str; 1] = ["PreRelese"];

/// The PR request payload: everything the queue consumer (or a future
/// direct backend) needs to open exactly this PR, and nothing else.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrPayload {
    pub title: String,
    pub body: String,
    pub head_branch: String,
    pub target_branch: String,
    pub labels: Vec<String>,
    pub signature: String,
}

/// Build the payload for a proposal. `target_branch` must be in the
/// allowlist — `master` (or anything else) errors by construction.
/// The body carries the eval report (per-test results, diff stat,
/// runtime), the proposal's budget/cooldown refs from intake, and the
/// full problem/trace/test context John needs to merge on PR evidence
/// alone (gate 291's standard).
pub fn build_pr_payload(
    proposal: &ProposalRecord,
    target_branch: &str,
    head_branch: &str,
) -> Result<PrPayload, String> {
    if !PR_TARGET_ALLOWLIST.contains(&target_branch) {
        return Err(format!(
            "target branch rejected by allowlist: {target_branch} (allowed: {:?})",
            PR_TARGET_ALLOWLIST
        ));
    }
    let report: EvalReport = serde_json::from_value(proposal.eval_report_json.clone())
        .map_err(|_| "missing or unparseable eval report".to_string())?;
    let mut body = String::new();
    body.push_str(&format!(
        "Self-improvement proposal for `{}`.\n\n",
        proposal.signature
    ));
    body.push_str(&format!("Problem: {}\n\n", proposal.problem_statement));
    body.push_str(&format!("Trace: {}\n\n", proposal.trace));
    body.push_str(&format!("Diff: {}\n\n", proposal.diff_description));
    body.push_str(&format!("Proving test: {}\n\n", proposal.proving_test));
    body.push_str("## Evaluation\n\n");
    body.push_str(&format!(
        "Result: {} (promoted: {}, reason: {})\n\n",
        if report.passed { "PASS" } else { "FAIL" },
        report.promoted,
        report.promotion_reason
    ));
    body.push_str(&format!(
        "Diff stat: {} file(s), {} new lines, {} old lines; runtime {} ms.\n\n",
        report.diff_stat.files_changed,
        report.diff_stat.new_lines,
        report.diff_stat.old_lines,
        report.runtime_ms
    ));
    body.push_str("Subset results:\n");
    for r in &report.subset_results {
        body.push_str(&format!(
            "- {}: {}\n",
            r.name,
            if r.passed { "pass" } else { "FAIL" }
        ));
    }
    if !report.eval_set_results.is_empty() {
        body.push_str("\nEval set results:\n");
        for r in &report.eval_set_results {
            body.push_str(&format!(
                "- {}: {}\n",
                r.name,
                if r.passed { "pass" } else { "FAIL" }
            ));
        }
    }
    body.push_str(&format!(
        "\nBudget refs at intake: {} proposal(s) this week, {} open for this signature.\n",
        proposal.proposals_this_week, proposal.open_for_signature
    ));
    body.push_str("\nOpened by the Helix self-improvement loop. John merges.\n");
    Ok(PrPayload {
        title: format!("self-improve: {}", proposal.signature),
        body,
        head_branch: head_branch.to_string(),
        target_branch: target_branch.to_string(),
        labels: vec!["self-improve".to_string()],
        signature: proposal.signature.clone(),
    })
}

/// The Phase 3 gate decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Payload is ready for the backend.
    Open(PrPayload),
    /// Refused; the reason string is logged, no backend call happens.
    Refuse(String),
}

/// The full Phase 3 gate, in order: D1 flag, target, eval gate, audit.
/// `head_branch` is the evaluated branch name (the evaluator's branch);
/// in queue-only operation the consumer resolves it from the payload.
pub fn phase3_gate(
    proposal: &ProposalRecord,
    audit: &AuditReport,
    cfg: &crate::self_improve::SelfImproveConfig,
    target_branch: &str,
    head_branch: &str,
) -> GateDecision {
    if !cfg.pr_opening {
        return GateDecision::Refuse("pr-opening-disabled".to_string());
    }
    if audit_outcome(audit) != AuditOutcome::Clean {
        return GateDecision::Refuse("audit-not-clean".to_string());
    }
    if !proposal.promoted || !proposal.eval_passed || proposal.eval_report_json.is_null() {
        return GateDecision::Refuse("incomplete-eval-gate".to_string());
    }
    if !cfg.eval_set_approved {
        return GateDecision::Refuse("eval-set-not-approved".to_string());
    }
    match build_pr_payload(proposal, target_branch, head_branch) {
        Ok(payload) => GateDecision::Open(payload),
        Err(e) => GateDecision::Refuse(e),
    }
}

/// A PR backend. Production is `QueueBackend`; tests use
/// `RecordingBackend`. `open_pr` returns an id (queue line id or PR
/// number as a string); `close_pr` closes by PR number.
pub trait PrBackend {
    fn open_pr(&mut self, payload: &PrPayload) -> Result<String, String>;
    fn close_pr(&mut self, pull_number: u64, reason: &str) -> Result<(), String>;
}

/// Queue entries: an open request, or a close action for a PR the
/// consumer opened earlier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action")]
pub enum QueueEntry {
    #[serde(rename = "open")]
    Open { payload: PrPayload, queued_at_ms: u64 },
    #[serde(rename = "close")]
    Close { pull_number: u64, reason: String, queued_at_ms: u64 },
}

/// Production backend: appends to `rt/self_improve/pr_queue.jsonl`
/// (one JSON object per line, atomic append via temp + rename over a
/// read-modify-write — the queue is append-mostly and small). No
/// GitHub call happens in-crate, ever.
pub struct QueueBackend {
    path: PathBuf,
}

impl QueueBackend {
    pub fn new(rt_dir: &Path) -> Self {
        Self { path: rt_dir.join("self_improve").join("pr_queue.jsonl") }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append(&mut self, entry: &QueueEntry) -> Result<String, String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("queue dir create failed: {e}"))?;
        }
        let mut existing = String::new();
        if self.path.exists() {
            existing = std::fs::read_to_string(&self.path)
                .map_err(|e| format!("queue read failed: {e}"))?;
            if !existing.is_empty() && !existing.ends_with('\n') {
                existing.push('\n');
            }
        }
        let line = serde_json::to_string(entry)
            .map_err(|e| format!("queue serialize failed: {e}"))?;
        let tmp = self.path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, format!("{existing}{line}\n"))
            .map_err(|e| format!("queue temp write failed: {e}"))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("queue rename failed: {e}"))?;
        Ok(format!("queued:{}", existing.lines().count() + 1))
    }

    /// Read all queued entries (for the consumer, and for tests).
    pub fn read_queue(&self) -> Result<Vec<QueueEntry>, String> {
        let Ok(data) = std::fs::read_to_string(&self.path) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for line in data.lines() {
            if line.trim().is_empty() {
                continue;
            }
            out.push(
                serde_json::from_str::<QueueEntry>(line)
                    .map_err(|e| format!("queue parse failed: {e}"))?,
            );
        }
        Ok(out)
    }
}

impl PrBackend for QueueBackend {
    fn open_pr(&mut self, payload: &PrPayload) -> Result<String, String> {
        self.append(&QueueEntry::Open {
            payload: payload.clone(),
            queued_at_ms: crate::utils::now_ms(),
        })
    }

    fn close_pr(&mut self, pull_number: u64, reason: &str) -> Result<(), String> {
        self.append(&QueueEntry::Close {
            pull_number,
            reason: reason.to_string(),
            queued_at_ms: crate::utils::now_ms(),
        })?;
        Ok(())
    }
}

/// Request a PR open through the gate. On refuse, nothing is called
/// and (for the D1-disabled case) the `pr-opening-disabled` line is
/// logged — the proposal stays as it was, never dropped. On open, the
/// backend is called once and the record becomes `queued`.
pub fn request_pr_open(
    backend: &mut dyn PrBackend,
    rt_dir: &Path,
    proposal: &mut ProposalRecord,
    audit: &AuditReport,
    cfg: &crate::self_improve::SelfImproveConfig,
    target_branch: &str,
    head_branch: &str,
    now_ms: u64,
) -> Result<String, String> {
    match phase3_gate(proposal, audit, cfg, target_branch, head_branch) {
        GateDecision::Refuse(reason) => {
            if reason == "pr-opening-disabled" {
                crate::utils::log_to_file("self_improve: pr-opening-disabled");
            }
            Err(format!("PR refused: {reason}"))
        }
        GateDecision::Open(payload) => {
            let id = backend.open_pr(&payload)?;
            proposal.status = "queued".to_string();
            proposal.updated_at = now_ms;
            upsert_proposal(rt_dir, proposal)?;
            Ok(id)
        }
    }
}

/// Withdraw a proposal (290.2): close via the backend when a PR number
/// is known (under `QueueBackend` this queues a close action for the
/// consumer, which executes `update_pull_request(state=closed)`),
/// then mark `withdrawn` and append the regression note — so even a
/// failed close leaves a manual-close note for John, never a silent
/// open PR. When no PR number is known, the record is still withdrawn
/// with the note (nothing exists to close).
pub fn withdraw_proposal(
    backend: &mut dyn PrBackend,
    rt_dir: &Path,
    proposal: &mut ProposalRecord,
    pull_number: Option<u64>,
    regression_note: &str,
    now_ms: u64,
) -> Result<(), String> {
    let mut note = format!("withdrawn: {regression_note}");
    if let Some(n) = pull_number {
        if let Err(e) = backend.close_pr(n, regression_note) {
            note.push_str(&format!(" (close failed: {e}; John closes manually)"));
        }
    }
    proposal.status = "withdrawn".to_string();
    proposal.notes.push(note);
    proposal.updated_at = now_ms;
    upsert_proposal(rt_dir, proposal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::self_improve::evaluator::EvalReport;
    use crate::self_improve::evaluator::{DiffStat, TestResult};

    /// Test backend that records calls instead of acting.
    struct RecordingBackend {
        opened: Vec<PrPayload>,
        closed: Vec<(u64, String)>,
    }

    impl RecordingBackend {
        fn new() -> Self {
            Self { opened: Vec::new(), closed: Vec::new() }
        }
    }

    impl PrBackend for RecordingBackend {
        fn open_pr(&mut self, payload: &PrPayload) -> Result<String, String> {
            self.opened.push(payload.clone());
            Ok(format!("pr-{}", self.opened.len()))
        }

        fn close_pr(&mut self, pull_number: u64, reason: &str) -> Result<(), String> {
            self.closed.push((pull_number, reason.to_string()));
            Ok(())
        }
    }

    fn green_proposal() -> ProposalRecord {
        let report = EvalReport {
            passed: true,
            promoted: true,
            promotion_reason: "ok".to_string(),
            subset_results: vec![TestResult {
                name: "a::b".to_string(),
                passed: true,
                detail: "ok".to_string(),
            }],
            eval_set_results: vec![TestResult {
                name: "eval1".to_string(),
                passed: true,
                detail: "ok".to_string(),
            }],
            diff_stat: DiffStat { files_changed: 1, new_lines: 10, old_lines: 2 },
            runtime_ms: 1234,
        };
        ProposalRecord {
            problem_statement: "Error log counts chatter".to_string(),
            trace: "errors:logs/error_log.md".to_string(),
            diff_description: "Filter counting".to_string(),
            proving_test: "cpu::tests::x".to_string(),
            signature: "errors:logs/error_log.md".to_string(),
            status: "evaluated".to_string(),
            created_at: 1_000,
            updated_at: 1_000,
            proposals_this_week: 1,
            open_for_signature: 1,
            eval_passed: true,
            promoted: true,
            promotion_reason: "ok".to_string(),
            eval_report_json: serde_json::to_value(&report).unwrap(),
            notes: Vec::new(),
        }
    }

    fn cfg_open() -> crate::self_improve::SelfImproveConfig {
        let mut cfg = crate::self_improve::SelfImproveConfig::default();
        cfg.pr_opening = true;
        cfg.eval_set_approved = true;
        cfg
    }

    fn clean_audit() -> AuditReport {
        AuditReport::default()
    }

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pr_opening_disabled_makes_zero_backend_calls_and_keeps_proposal() {
        // GIVEN pr_opening=false (the shipped default), WHEN a PR is
        // requested for a green proposal, THEN no backend call happens
        // and the proposal stays evaluated (queued for a future open,
        // never dropped, never opened).
        let dir = tmpdir("helix-self-improve-pr-disabled");
        let mut backend = RecordingBackend::new();
        let mut p = green_proposal();
        let cfg = crate::self_improve::SelfImproveConfig::default();
        assert!(!cfg.pr_opening);
        let r = request_pr_open(
            &mut backend,
            &dir,
            &mut p,
            &clean_audit(),
            &cfg,
            "PreRelese",
            "self-improve/test",
            2_000,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("pr-opening-disabled"));
        assert!(backend.opened.is_empty(), "zero backend calls when disabled");
        assert_eq!(p.status, "evaluated", "proposal stays queued for a future open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pr_opening_enabled_with_full_gate_green_opens_correct_payload() {
        // GIVEN pr_opening=true + promoted + eval green + clean audit,
        // WHEN requested, THEN the backend is called once with the
        // correct payload: target PreRelese, label self-improve, body
        // containing the eval report, diff stat, and budget refs.
        let dir = tmpdir("helix-self-improve-pr-green");
        let mut backend = RecordingBackend::new();
        let mut p = green_proposal();
        let r = request_pr_open(
            &mut backend,
            &dir,
            &mut p,
            &clean_audit(),
            &cfg_open(),
            "PreRelese",
            "self-improve/test",
            2_000,
        );
        assert!(r.is_ok(), "green gate must open: {:?}", r);
        assert_eq!(backend.opened.len(), 1);
        let payload = &backend.opened[0];
        assert_eq!(payload.target_branch, "PreRelese");
        assert_eq!(payload.head_branch, "self-improve/test");
        assert_eq!(payload.labels, vec!["self-improve".to_string()]);
        assert!(payload.body.contains("a::b"));
        assert!(payload.body.contains("eval1"));
        assert!(payload.body.contains("1 file(s), 10 new lines, 2 old lines"));
        assert!(payload.body.contains("1 proposal(s) this week"));
        assert_eq!(p.status, "queued");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn master_target_rejected_by_construction() {
        // GIVEN a green proposal, WHEN the target is master, THEN the
        // payload builder errors and the gate refuses — master can
        // never be a loop PR target.
        let p = green_proposal();
        assert!(build_pr_payload(&p, "master", "self-improve/test").is_err());
        let gate = phase3_gate(&p, &clean_audit(), &cfg_open(), "master", "self-improve/test");
        assert!(matches!(gate, GateDecision::Refuse(_)));
    }

    #[test]
    fn needs_human_review_never_opened_even_with_flag_on() {
        // GIVEN a needs-human-review audit (sensitive path) + a green
        // eval + pr_opening=true, WHEN requested, THEN the gate refuses
        // and the backend is never called — sensitive-path proposals
        // are queue-only forever; John opens any PR himself.
        // (This is the 288 stub test's gate-level replacement.)
        let dir = tmpdir("helix-self-improve-pr-sensitive");
        let mut backend = RecordingBackend::new();
        let mut p = green_proposal();
        let mut audit = AuditReport::default();
        audit.sensitive_paths.push("sensitive-path:src/security.rs".to_string());
        let r = request_pr_open(
            &mut backend,
            &dir,
            &mut p,
            &audit,
            &cfg_open(),
            "PreRelese",
            "self-improve/test",
            2_000,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("audit-not-clean"));
        assert!(backend.opened.is_empty());
        assert_eq!(p.status, "evaluated");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incomplete_gate_blocks_opening() {
        // GIVEN each incomplete-gate shape, WHEN requested, THEN refused
        // with zero backend calls: (a) promoted=false, (b) missing eval
        // report, (c) secret-dirty audit, (d) D5 gate closed after eval.
        let dir = tmpdir("helix-self-improve-pr-incomplete");
        let mut backend = RecordingBackend::new();
        // (a) promoted=false.
        let mut p = green_proposal();
        p.promoted = false;
        assert!(request_pr_open(
            &mut backend, &dir, &mut p, &clean_audit(), &cfg_open(),
            "PreRelese", "self-improve/test", 2_000
        )
        .is_err());
        // (b) missing eval report.
        let mut p = green_proposal();
        p.eval_report_json = serde_json::Value::Null;
        assert!(request_pr_open(
            &mut backend, &dir, &mut p, &clean_audit(), &cfg_open(),
            "PreRelese", "self-improve/test", 2_000
        )
        .is_err());
        // (c) secret-dirty audit.
        let mut p = green_proposal();
        let mut audit = AuditReport::default();
        audit.secret_hits.push(crate::self_improve::audit::SecretHit {
            file: "src/x.rs".to_string(),
            line_count: 1,
            pattern_class: "github-token".to_string(),
        });
        assert!(request_pr_open(
            &mut backend, &dir, &mut p, &audit, &cfg_open(),
            "PreRelese", "self-improve/test", 2_000
        )
        .is_err());
        // (d) D5 gate closed after eval.
        let mut p = green_proposal();
        let mut cfg = cfg_open();
        cfg.eval_set_approved = false;
        assert!(request_pr_open(
            &mut backend, &dir, &mut p, &clean_audit(), &cfg,
            "PreRelese", "self-improve/test", 2_000
        )
        .is_err());
        assert!(backend.opened.is_empty(), "no incomplete gate may open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn withdrawal_closes_and_marks_withdrawn_with_note() {
        // GIVEN a queued proposal with a known PR number, WHEN
        // withdrawn with a regression note, THEN the backend close is
        // called with that number, the record is withdrawn, and the
        // note is appended.
        let dir = tmpdir("helix-self-improve-pr-withdraw");
        let mut backend = RecordingBackend::new();
        let mut p = green_proposal();
        p.status = "queued".to_string();
        withdraw_proposal(
            &mut backend,
            &dir,
            &mut p,
            Some(42),
            "regression: nightly suite red after merge",
            2_000,
        )
        .unwrap();
        assert_eq!(backend.closed.len(), 1);
        assert_eq!(backend.closed[0].0, 42);
        assert_eq!(p.status, "withdrawn");
        assert!(p.notes.iter().any(|n| n.contains("regression: nightly suite red")));
        // Persisted: the stored record is withdrawn with the note.
        let stored = crate::self_improve::worker::read_proposals(&dir).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, "withdrawn");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_backend_appends_open_and_close_entries() {
        // GIVEN a QueueBackend, WHEN opening then closing, THEN the
        // queue file holds one open entry (full payload) and one close
        // action — the consumer's input, with no GitHub call in-crate.
        let dir = tmpdir("helix-self-improve-pr-queue");
        let mut backend = QueueBackend::new(&dir);
        let p = green_proposal();
        let payload = build_pr_payload(&p, "PreRelese", "self-improve/test").unwrap();
        backend.open_pr(&payload).unwrap();
        backend.close_pr(7, "regression").unwrap();
        let entries = backend.read_queue().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0], QueueEntry::Open { .. }));
        assert!(matches!(
            &entries[1],
            QueueEntry::Close { pull_number: 7, .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
