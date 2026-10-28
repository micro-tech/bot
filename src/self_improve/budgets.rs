//! Budget, cooldown, and lesson engine (task 283).
//!
//! Hard caps, enforced before any proposal is intaked — a violation
//! aborts the cycle and is logged as `phase_failure`, never a quiet
//! skip:
//! - rolling 3 proposals/week (config `max_proposals_per_week`),
//! - 1 open proposal per failing signature (config
//!   `max_open_per_signature`),
//! - declined → 30-day cooldown (config `declined_cooldown_days`),
//! - failed-eval → 7-day cooldown (config
//!   `failed_eval_cooldown_days`), per signature: a CHANGED signature
//!   bypasses the failed-eval cooldown (the cooldown cools a failing
//!   signature, not the whole loop — a naive loop-wide block would
//!   stall every proposal for a week after one bad eval).
//! - cycle timeout (config `max_cycle_minutes`): the worker dispatch
//!   is bounded; an expired cycle abandons cleanly, intakes nothing
//!   (no partial proposal), and records `cycle_timeout`.
//!
//! Cooldowns run from `updated_at` (the status-change time), not from
//! intake: a proposal declined months after intake is cooled from its
//! decline. The 500-line diff cap is NOT here — it is recorded as an
//! estimate at Phase 1 intake and enforced HARD in the Phase 2
//! evaluator (task 287), where a real diff exists; there is no Phase 1
//! diff to measure.
//!
//! Lessons go through `memory_continuity` (the same path the nightly
//! consolidation uses), so loop outcomes become durable memory, not
//! log lines that scroll away.

use crate::self_improve::worker::ProposalRecord;

const WEEK_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Why a proposal may not be intaked. The handler logs the variant as
/// `phase_failure`, never a quiet skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetBlock {
    /// Rolling-week cap reached (count includes the candidate: prior
    /// proposals this week already at the max).
    WeeklyCap { count: usize, max: u32 },
    /// An open proposal for this signature already exists.
    SignatureOpen { signature: String },
    /// A proposal for this signature was declined within the cooldown.
    DeclinedCooldown { signature: String, days_left: u64 },
    /// A proposal for this signature failed evaluation within the
    /// cooldown (same signature only — a changed signature bypasses).
    FailedEvalCooldown { signature: String, days_left: u64 },
}

impl std::fmt::Display for BudgetBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetBlock::WeeklyCap { count, max } => {
                write!(f, "weekly cap reached ({count}/{max} proposals this week)")
            }
            BudgetBlock::SignatureOpen { signature } => {
                write!(f, "open proposal already exists for signature {signature}")
            }
            BudgetBlock::DeclinedCooldown { signature, days_left } => {
                write!(
                    f,
                    "signature {signature} declined {days_left} day(s) inside cooldown"
                )
            }
            BudgetBlock::FailedEvalCooldown { signature, days_left } => {
                write!(
                    f,
                    "signature {signature} failed eval {days_left} day(s) inside cooldown"
                )
            }
        }
    }
}

/// Proposals recorded in the last rolling week (by intake time).
pub fn proposals_this_week(proposals: &[ProposalRecord], now_ms: u64) -> usize {
    let week_ago = now_ms.saturating_sub(WEEK_MS);
    proposals
        .iter()
        .filter(|p| p.created_at >= week_ago)
        .count()
}

/// Open proposals for one signature ("proposed", "evaluating", "queued").
pub fn open_for_signature(proposals: &[ProposalRecord], signature: &str) -> usize {
    proposals
        .iter()
        .filter(|p| {
            p.signature == signature
                && matches!(p.status.as_str(), "proposed" | "evaluating" | "queued")
        })
        .count()
}

/// Latest record with this signature and status, by `updated_at`.
fn latest_with_status<'a>(
    proposals: &'a [ProposalRecord],
    signature: &str,
    status: &str,
) -> Option<&'a ProposalRecord> {
    proposals
        .iter()
        .filter(|p| p.signature == signature && p.status == status)
        .max_by_key(|p| p.updated_at)
}

/// The full pre-intake gate. Order: weekly cap, signature dedupe,
/// declined cooldown, failed-eval cooldown. All checks are pure over
/// the stored proposals + config + clock, so they unit-test without
/// any runtime dir.
pub fn check_proposal_allowed(
    proposals: &[ProposalRecord],
    signature: &str,
    cfg: &crate::self_improve::SelfImproveConfig,
    now_ms: u64,
) -> Result<(), BudgetBlock> {
    if proposals_this_week(proposals, now_ms) as u32 >= cfg.max_proposals_per_week {
        return Err(BudgetBlock::WeeklyCap {
            count: proposals_this_week(proposals, now_ms),
            max: cfg.max_proposals_per_week,
        });
    }
    if open_for_signature(proposals, signature) as u32 >= cfg.max_open_per_signature {
        return Err(BudgetBlock::SignatureOpen {
            signature: signature.to_string(),
        });
    }
    if let Some(declined) = latest_with_status(proposals, signature, "declined") {
        let cooldown = cfg.declined_cooldown_days.saturating_mul(DAY_MS);
        if now_ms < declined.updated_at.saturating_add(cooldown) {
            let left =
                (declined.updated_at.saturating_add(cooldown).saturating_sub(now_ms) + DAY_MS - 1)
                    / DAY_MS;
            return Err(BudgetBlock::DeclinedCooldown {
                signature: signature.to_string(),
                days_left: left,
            });
        }
    }
    if let Some(failed) = latest_with_status(proposals, signature, "eval-failed") {
        let cooldown = cfg.failed_eval_cooldown_days.saturating_mul(DAY_MS);
        if now_ms < failed.updated_at.saturating_add(cooldown) {
            let left =
                (failed.updated_at.saturating_add(cooldown).saturating_sub(now_ms) + DAY_MS - 1)
                    / DAY_MS;
            return Err(BudgetBlock::FailedEvalCooldown {
                signature: signature.to_string(),
                days_left: left,
            });
        }
    }
    Ok(())
}

/// A cycle's wall-clock deadline. The handler stamps the start before
/// dispatch and checks after: an expired cycle abandons cleanly —
/// nothing is intaked, no partial proposal exists, and the outcome
/// recorded is `cycle_timeout`.
#[derive(Debug, Clone, Copy)]
pub struct CycleDeadline {
    pub start_ms: u64,
    pub timeout_ms: u64,
}

impl CycleDeadline {
    pub fn new(start_ms: u64, max_cycle_minutes: u64) -> Self {
        Self {
            start_ms,
            timeout_ms: max_cycle_minutes.saturating_mul(60 * 1000),
        }
    }

    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms >= self.start_ms.saturating_add(self.timeout_ms)
    }
}

/// Record a loop lesson in durable memory, following the task-206
/// pattern: a fresh unified agent state takes the record, then its
/// deltas are persisted into the disk-backed MemoryManager. (Note:
/// the CPU's own `self.state` is a DIFFERENT `AgentState` — runtime
/// mode/counters, not the continuity type — so lessons go through a
/// fresh unified state, never `self.state`.)
pub fn record_loop_lesson(
    memory: &mut crate::memory::MemoryManager,
    lesson: &str,
    confidence: f32,
) {
    let mut state = crate::agents::agent_state::AgentState::new();
    crate::agents::memory_continuity::record_lesson(&mut state, "self-improve", lesson, confidence);
    crate::agents::memory_continuity::persist_memory_deltas(&mut state, memory);
}

/// Record a loop outcome (intake, promotion, decline) in durable memory.
pub fn record_loop_outcome(
    memory: &mut crate::memory::MemoryManager,
    summary: &str,
    tags: Vec<String>,
) {
    let mut state = crate::agents::agent_state::AgentState::new();
    crate::agents::memory_continuity::record_important_outcome(
        &mut state, "self-improve", summary, tags,
    );
    crate::agents::memory_continuity::persist_memory_deltas(&mut state, memory);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> crate::self_improve::SelfImproveConfig {
        crate::self_improve::SelfImproveConfig::default()
    }

    fn proposal(signature: &str, status: &str, created_at: u64, updated_at: u64) -> ProposalRecord {
        ProposalRecord {
            problem_statement: "p".to_string(),
            trace: signature.to_string(),
            diff_description: "d".to_string(),
            proving_test: "t".to_string(),
            signature: signature.to_string(),
            status: status.to_string(),
            created_at,
            updated_at,
            proposals_this_week: 0,
            open_for_signature: 0,
            eval_passed: false,
            promoted: false,
            promotion_reason: String::new(),
            eval_report_json: serde_json::Value::Null,
            notes: Vec::new(),
        }
    }

    #[test]
    fn fourth_proposal_in_a_week_is_blocked() {
        // GIVEN 3 proposals already recorded this week (the max),
        // WHEN a 4th is gated, THEN the weekly cap blocks it.
        let now = 100 * WEEK_MS;
        let proposals: Vec<_> = (0..3)
            .map(|i| proposal(&format!("sig-{i}"), "promoted", now - 1000, now - 1000))
            .collect();
        assert_eq!(
            check_proposal_allowed(&proposals, "sig-new", &cfg(), now),
            Err(BudgetBlock::WeeklyCap { count: 3, max: 3 })
        );
        // And 2 prior proposals leave room for a 3rd.
        assert!(check_proposal_allowed(&proposals[..2], "sig-new", &cfg(), now).is_ok());
    }

    #[test]
    fn second_open_proposal_for_same_signature_is_blocked() {
        // GIVEN an open ("proposed") proposal for a signature,
        // WHEN another for the same signature is gated, THEN dedupe
        // blocks it — but a different signature passes.
        let now = 100 * WEEK_MS;
        let proposals = vec![proposal("sig-a", "proposed", now - 1000, now - 1000)];
        assert_eq!(
            check_proposal_allowed(&proposals, "sig-a", &cfg(), now),
            Err(BudgetBlock::SignatureOpen { signature: "sig-a".to_string() })
        );
        assert!(check_proposal_allowed(&proposals, "sig-b", &cfg(), now).is_ok());
    }

    #[test]
    fn declined_cooldown_blocks_at_29_days_and_allows_at_31() {
        // GIVEN a decline 29 days ago (inside the 30-day cooldown),
        // WHEN gated, THEN blocked; at 31 days, allowed.
        let now = 100 * WEEK_MS;
        let declined_29 = vec![proposal("sig-a", "declined", now - 40 * DAY_MS, now - 29 * DAY_MS)];
        assert!(matches!(
            check_proposal_allowed(&declined_29, "sig-a", &cfg(), now),
            Err(BudgetBlock::DeclinedCooldown { .. })
        ));
        let declined_31 = vec![proposal("sig-a", "declined", now - 40 * DAY_MS, now - 31 * DAY_MS)];
        assert!(check_proposal_allowed(&declined_31, "sig-a", &cfg(), now).is_ok());
    }

    #[test]
    fn failed_eval_cooldown_blocks_at_6_days_allows_at_8_and_changed_signature_bypasses() {
        // GIVEN a failed eval 6 days ago (inside the 7-day cooldown),
        // WHEN gated on the SAME signature, THEN blocked; at 8 days,
        // allowed; and a CHANGED signature bypasses even at 6 days.
        let now = 100 * WEEK_MS;
        let failed_6 = vec![proposal("sig-a", "eval-failed", now - 10 * DAY_MS, now - 6 * DAY_MS)];
        assert!(matches!(
            check_proposal_allowed(&failed_6, "sig-a", &cfg(), now),
            Err(BudgetBlock::FailedEvalCooldown { .. })
        ));
        let failed_8 = vec![proposal("sig-a", "eval-failed", now - 10 * DAY_MS, now - 8 * DAY_MS)];
        assert!(check_proposal_allowed(&failed_8, "sig-a", &cfg(), now).is_ok());
        assert!(
            check_proposal_allowed(&failed_6, "sig-b", &cfg(), now).is_ok(),
            "a changed signature bypasses the failed-eval cooldown"
        );
    }

    #[test]
    fn cycle_deadline_expires_cleanly() {
        // GIVEN a 60-minute cycle started at T, WHEN checked at T+60m,
        // THEN expired (abandon, no partial proposal); at T+59m, not.
        let d = CycleDeadline::new(1_000, 60);
        assert!(!d.expired(1_000 + 59 * 60 * 1000));
        assert!(d.expired(1_000 + 60 * 60 * 1000));
    }
}
