//! Review gate (task 366, Phase 3).
//!
//! The ordered minimum a self-authored skill (or context, or any
//! loop proposal) must pass before John merges it:
//! 1. Unit tests green — the evaluator's bounded subset ran and every
//!    result passed.
//! 2. Bounded subset green — the subset actually ran (a zero-result
//!    run is not a green subset) and the diff is within the line cap.
//! 3. Approved eval set green — `cargo test --test evals` ran, every
//!    result passed, and the D5 gate (`eval_set_approved`) is open.
//! 4. Secrets / sensitive-path audit clean (task 288) — any secret hit
//!    or sensitive path fails; sensitive-path proposals are queue-only
//!    forever and John opens any PR himself.
//! 5. John's read — a human check, always `PendingHuman` here.
//! 6. John's merge tap — a human check, always `PendingHuman` here.
//!
//! This module is a READ-ONLY report: `review_gate_checklist`
//! returns the checklist; it enforces nothing. Gate enforcement
//! stays in `phase3_gate` (`pr.rs`), which refuses on any failed
//! machine check. The human checks can never be `Pass` from code —
//! only John can pass them, by reading and by merging.
//!
//! Sandbox prerequisite: a FULL sandbox IS a prerequisite for any
//! self-deployment. Until one exists, every self-authored skill goes
//! through John's merge tap — the loop never self-deploys by
//! construction (no code path here writes the live tree, opens a PR
//! without the gate, or merges anything). No full sandbox is designed
//! here: that is a separate follow-up workstream, filed as task 368
//! in the task list (pending, high priority).

use crate::self_improve::audit::{AuditOutcome, AuditReport, audit_outcome};
use crate::self_improve::evaluator::EvalReport;
use crate::self_improve::worker::ProposalRecord;

/// One check's outcome. `PendingHuman` is the only outcome a human
/// check can ever have from code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Fail,
    PendingHuman,
}

/// One row of the review gate checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

impl GateCheck {
    fn pass(name: &str, detail: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::Pass,
            detail: detail.to_string(),
        }
    }

    fn fail(name: &str, detail: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::Fail,
            detail: detail.to_string(),
        }
    }

    fn pending_human(name: &str, detail: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::PendingHuman,
            detail: detail.to_string(),
        }
    }
}

/// The review gate checklist for a proposal, in the ordered minimum
/// (task 366). Machine checks are computed from the proposal's eval
/// report, its diff, the audit, and the config; the two human checks
/// are always `PendingHuman`.
pub fn review_gate_checklist(
    proposal: &ProposalRecord,
    audit: &AuditReport,
    cfg: &crate::self_improve::SelfImproveConfig,
) -> Vec<GateCheck> {
    let report: Option<EvalReport> = serde_json::from_value(proposal.eval_report_json.clone()).ok();
    let new_lines: usize = proposal
        .diff_files
        .iter()
        .map(|f| f.content.lines().count())
        .sum();

    let mut checks = Vec::new();

    // 1. Unit tests green: the subset ran and every result passed.
    match &report {
        Some(r) if !r.subset_results.is_empty() && r.subset_results.iter().all(|t| t.passed) => {
            checks.push(GateCheck::pass(
                "unit-tests-green",
                &format!("{} subset test(s) passed", r.subset_results.len()),
            ));
        }
        Some(r) if r.subset_results.is_empty() => {
            checks.push(GateCheck::fail("unit-tests-green", "subset ran zero tests"));
        }
        Some(_) => {
            checks.push(GateCheck::fail(
                "unit-tests-green",
                "subset has failing test(s)",
            ));
        }
        None => {
            checks.push(GateCheck::fail(
                "unit-tests-green",
                "no eval report on proposal",
            ));
        }
    }

    // 2. Bounded subset green: the subset ran AND the diff is within
    // the line cap (a green run on an over-cap diff is not bounded).
    if new_lines > cfg.max_diff_lines {
        checks.push(GateCheck::fail(
            "bounded-subset-green",
            &format!(
                "diff of {new_lines} lines exceeds the {}-line cap",
                cfg.max_diff_lines
            ),
        ));
    } else if report
        .as_ref()
        .map(|r| !r.subset_results.is_empty())
        .unwrap_or(false)
    {
        checks.push(GateCheck::pass(
            "bounded-subset-green",
            &format!(
                "subset ran; diff of {new_lines} lines within the {}-line cap",
                cfg.max_diff_lines
            ),
        ));
    } else {
        checks.push(GateCheck::fail("bounded-subset-green", "subset never ran"));
    }

    // 3. Approved eval set green: the D5 gate is open, the eval set
    // ran, and every result passed.
    match &report {
        _ if !cfg.eval_set_approved => {
            checks.push(GateCheck::fail(
                "eval-set-green",
                "eval_set_approved is false (D5 gate closed)",
            ));
        }
        Some(r)
            if !r.eval_set_results.is_empty() && r.eval_set_results.iter().all(|t| t.passed) =>
        {
            checks.push(GateCheck::pass(
                "eval-set-green",
                &format!("{} eval set test(s) passed", r.eval_set_results.len()),
            ));
        }
        Some(r) if r.eval_set_results.is_empty() => {
            checks.push(GateCheck::fail("eval-set-green", "eval set ran zero tests"));
        }
        Some(_) => {
            checks.push(GateCheck::fail(
                "eval-set-green",
                "eval set has failing test(s)",
            ));
        }
        None => {
            checks.push(GateCheck::fail(
                "eval-set-green",
                "no eval report on proposal",
            ));
        }
    }

    // 4. Audit clean.
    if audit_outcome(audit) == AuditOutcome::Clean {
        checks.push(GateCheck::pass(
            "audit-clean",
            "no secret hits, no sensitive paths",
        ));
    } else {
        checks.push(GateCheck::fail(
            "audit-clean",
            "audit is not clean (secrets and/or sensitive paths)",
        ));
    }

    // Promotion itself (the evaluator's verdict) is a machine check.
    if proposal.promoted && proposal.eval_passed {
        checks.push(GateCheck::pass(
            "promoted",
            "evaluator promoted the proposal",
        ));
    } else {
        checks.push(GateCheck::fail(
            "promoted",
            "evaluator did not promote the proposal",
        ));
    }

    // 5–6. Human checks — never Pass from code.
    checks.push(GateCheck::pending_human(
        "john-read",
        "John has read the proposal and the PR evidence",
    ));
    checks.push(GateCheck::pending_human(
        "john-merge-tap",
        "John merges (the loop never merges)",
    ));

    checks
}

/// True when every MACHINE check passes (human checks are excluded —
/// they are always `PendingHuman` and are John's, not the loop's).
pub fn all_machine_checks_pass(checks: &[GateCheck]) -> bool {
    checks
        .iter()
        .filter(|c| c.status != CheckStatus::PendingHuman)
        .all(|c| c.status == CheckStatus::Pass)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::self_improve::evaluator::{DiffStat, TestResult};

    fn report(subset_pass: bool, eval_pass: bool) -> EvalReport {
        EvalReport {
            passed: subset_pass && eval_pass,
            promoted: subset_pass && eval_pass,
            promotion_reason: if subset_pass && eval_pass {
                "ok".to_string()
            } else {
                "x".to_string()
            },
            subset_results: vec![TestResult {
                name: "a::b".to_string(),
                passed: subset_pass,
                detail: if subset_pass {
                    "ok".to_string()
                } else {
                    "FAILED".to_string()
                },
            }],
            eval_set_results: vec![TestResult {
                name: "eval1".to_string(),
                passed: eval_pass,
                detail: if eval_pass {
                    "ok".to_string()
                } else {
                    "FAILED".to_string()
                },
            }],
            diff_stat: DiffStat {
                files_changed: 1,
                new_lines: 10,
                old_lines: 2,
            },
            runtime_ms: 1234,
        }
    }

    fn proposal_with(report: EvalReport, promoted: bool, eval_passed: bool) -> ProposalRecord {
        ProposalRecord {
            problem_statement: "p".to_string(),
            trace: "errors:logs/error_log.md".to_string(),
            diff_description: "d".to_string(),
            proving_test: "t".to_string(),
            signature: "errors:logs/error_log.md".to_string(),
            status: "evaluated".to_string(),
            created_at: 1_000,
            updated_at: 1_000,
            proposals_this_week: 1,
            open_for_signature: 1,
            eval_passed,
            promoted,
            promotion_reason: "ok".to_string(),
            eval_report_json: serde_json::to_value(&report).unwrap(),
            diff_files: vec![crate::self_improve::evaluator::DiffFile {
                path: "src/a.rs".to_string(),
                content: "x\n".to_string(),
            }],
            eval_branch: String::new(),
            notes: Vec::new(),
        }
    }

    fn cfg_open() -> crate::self_improve::SelfImproveConfig {
        let mut cfg = crate::self_improve::SelfImproveConfig::default();
        cfg.pr_opening = true;
        cfg.eval_set_approved = true;
        cfg
    }

    fn check_named<'a>(checks: &'a [GateCheck], name: &str) -> &'a GateCheck {
        checks
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("missing check {name}"))
    }

    #[test]
    fn all_green_checklist_passes_machine_checks() {
        // GIVEN a promoted proposal with a green report + clean audit
        // + open gate, WHEN the checklist runs, THEN every machine
        // check passes and both human checks are PendingHuman.
        let p = proposal_with(report(true, true), true, true);
        let checks = review_gate_checklist(&p, &AuditReport::default(), &cfg_open());
        assert!(all_machine_checks_pass(&checks));
        assert_eq!(
            check_named(&checks, "john-read").status,
            CheckStatus::PendingHuman
        );
        assert_eq!(
            check_named(&checks, "john-merge-tap").status,
            CheckStatus::PendingHuman
        );
        assert_eq!(
            check_named(&checks, "unit-tests-green").status,
            CheckStatus::Pass
        );
        assert_eq!(
            check_named(&checks, "bounded-subset-green").status,
            CheckStatus::Pass
        );
        assert_eq!(
            check_named(&checks, "eval-set-green").status,
            CheckStatus::Pass
        );
        assert_eq!(
            check_named(&checks, "audit-clean").status,
            CheckStatus::Pass
        );
        assert_eq!(check_named(&checks, "promoted").status, CheckStatus::Pass);
    }

    #[test]
    fn failing_subset_fails_unit_tests_and_promotion() {
        let p = proposal_with(report(false, true), false, false);
        let checks = review_gate_checklist(&p, &AuditReport::default(), &cfg_open());
        assert!(!all_machine_checks_pass(&checks));
        assert_eq!(
            check_named(&checks, "unit-tests-green").status,
            CheckStatus::Fail
        );
        assert_eq!(check_named(&checks, "promoted").status, CheckStatus::Fail);
        // The human checks are still PendingHuman, never Fail.
        assert_eq!(
            check_named(&checks, "john-read").status,
            CheckStatus::PendingHuman
        );
    }

    #[test]
    fn gate_closed_fails_eval_set() {
        let p = proposal_with(report(true, true), false, true);
        let mut cfg = cfg_open();
        cfg.eval_set_approved = false;
        let checks = review_gate_checklist(&p, &AuditReport::default(), &cfg);
        assert!(!all_machine_checks_pass(&checks));
        assert_eq!(
            check_named(&checks, "eval-set-green").status,
            CheckStatus::Fail
        );
    }

    #[test]
    fn dirty_audit_fails_audit_check() {
        let p = proposal_with(report(true, true), true, true);
        let mut audit = AuditReport::default();
        audit
            .sensitive_paths
            .push("sensitive-path:src/security.rs".to_string());
        let checks = review_gate_checklist(&p, &audit, &cfg_open());
        assert!(!all_machine_checks_pass(&checks));
        assert_eq!(
            check_named(&checks, "audit-clean").status,
            CheckStatus::Fail
        );
    }

    #[test]
    fn missing_report_fails_eval_checks() {
        let mut p = proposal_with(report(true, true), false, false);
        p.eval_report_json = serde_json::Value::Null;
        let checks = review_gate_checklist(&p, &AuditReport::default(), &cfg_open());
        assert!(!all_machine_checks_pass(&checks));
        assert_eq!(
            check_named(&checks, "unit-tests-green").status,
            CheckStatus::Fail
        );
        assert_eq!(
            check_named(&checks, "eval-set-green").status,
            CheckStatus::Fail
        );
    }

    #[test]
    fn over_cap_diff_fails_bounded_subset() {
        let mut p = proposal_with(report(true, true), true, true);
        let big = (0..501)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        p.diff_files = vec![crate::self_improve::evaluator::DiffFile {
            path: "src/a.rs".to_string(),
            content: big,
        }];
        let checks = review_gate_checklist(&p, &AuditReport::default(), &cfg_open());
        assert!(!all_machine_checks_pass(&checks));
        assert_eq!(
            check_named(&checks, "bounded-subset-green").status,
            CheckStatus::Fail
        );
    }
}
