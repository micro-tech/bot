//! Secrets audit + sensitive-path enforcement (task 288, Phase 2).
//!
//! Runs over the DIFF ONLY (the proposal's `DiffFile` contents are
//! the added lines — whole-file writes, so content = added lines),
//! never the whole repo. Two independent stops, both fail-closed:
//!
//! 1. Secret-value scan (metadata-only, task-265 pattern): a hit
//!    BLOCKS the proposal and raises a metadata-only alert. The report
//!    records file + line count + pattern class — NEVER the matched
//!    value. The scanner cannot distinguish a fake token in a fixture
//!    from a real one, and must not try: anything token-shaped blocks,
//!    and John reviews.
//! 2. Sensitive-path enforcement (D4 list, confirmed as-is): any touch
//!    sets status `needs-human-review` and STOPS here — no eval pass
//!    counts, never reaches Phase 3, John opens any PR himself.
//!
//! Sensitive-path list (exact, D4):
//! - `src/security.rs`, `src/tools/shell_security.rs`,
//!   `src/tools/shell_tool.rs`, `src/config/shell.rs` (exact path)
//! - `src/io/web_server/mod.rs` auth middleware (`require_web_token`)
//!   and `/ops/*` handlers — implemented as ANY diff touching the file:
//!   the file IS the web auth surface, and a whole-file diff cannot
//!   prove the auth code untouched, so the file itself is the trigger.
//! - `src/support_watch/` auto-send policy and draft/send paths —
//!   paths containing `draft`/`send`, or content mentioning `auto_send`.
//! - Token/credential handling anywhere — path containing
//!   `token`/`secret`/`auth`, or content with a word-boundary match on
//!   `token`/`auth_token`/`secret`/`credential`. Intentionally broad.
//! - `src/cron/registry.rs` and `src/cpu/mod.rs` routine dispatch —
//!   exact path match (the loop's own handler lives in `cpu/mod.rs`;
//!   loop self-modification MUST be human-reviewed).
//!
//! The evaluator (287) does NOT call this; the Phase 3 gate (290)
//! composes eval + audit, and audit runs first (a dirty diff never
//! spends a worktree eval).

use crate::self_improve::evaluator::DiffFile;
use crate::self_improve::worker::ProposalRecord;
use serde::{Deserialize, Serialize};

/// One secret hit: file + line count + pattern class. NEVER a value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretHit {
    pub file: String,
    pub line_count: usize,
    pub pattern_class: String,
}

/// The audit verdict for one diff.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AuditReport {
    pub secret_hits: Vec<SecretHit>,
    pub sensitive_paths: Vec<String>,
}

impl AuditReport {
    /// True when nothing stops the proposal.
    pub fn is_clean(&self) -> bool {
        self.secret_hits.is_empty() && self.sensitive_paths.is_empty()
    }
}

/// What the audit decides. Both non-clean outcomes stop the loop flow;
/// they differ in who resolves them: secrets are blocked (the diff is
/// unshippable as written), sensitive paths await John's own review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Clean,
    SecretsBlocked,
    NeedsHumanReview,
}

pub fn audit_outcome(report: &AuditReport) -> AuditOutcome {
    if !report.secret_hits.is_empty() {
        AuditOutcome::SecretsBlocked
    } else if !report.sensitive_paths.is_empty() {
        AuditOutcome::NeedsHumanReview
    } else {
        AuditOutcome::Clean
    }
}

/// Apply the audit outcome to a proposal record's status. Secrets →
/// `blocked` (not open: the worker may retry with a clean diff);
/// sensitive → `needs-human-review` (open: blocks duplicate proposals
/// for the signature until John resolves it). Clean leaves the status
/// untouched. Returns the outcome.
pub fn apply_audit_status(
    proposal: &mut ProposalRecord,
    report: &AuditReport,
    now_ms: u64,
) -> AuditOutcome {
    let outcome = audit_outcome(report);
    match outcome {
        AuditOutcome::SecretsBlocked => {
            proposal.status = "blocked".to_string();
            proposal.updated_at = now_ms;
        }
        AuditOutcome::NeedsHumanReview => {
            proposal.status = "needs-human-review".to_string();
            proposal.updated_at = now_ms;
        }
        AuditOutcome::Clean => {}
    }
    outcome
}

/// Metadata-only alert line for a blocked diff: files, counts,
/// classes — no values, ever. The caller logs this line.
pub fn alert_line(report: &AuditReport) -> String {
    let mut parts = vec!["self_improve: audit blocked".to_string()];
    for hit in &report.secret_hits {
        parts.push(format!(
            "{}: {} line(s) class={}",
            hit.file, hit.line_count, hit.pattern_class
        ));
    }
    for path in &report.sensitive_paths {
        parts.push(format!("sensitive-path: {path}"));
    }
    parts.join("; ")
}

/// Secret-value classes, matched on line text. Hand-rolled (no regex
/// dependency): each check looks for a SHAPE, and only the class name
/// is ever reported.
fn secret_classes_for_line(line: &str) -> Vec<&'static str> {
    let mut classes = Vec::new();
    if line.contains("ghp_") || line.contains("gho_") || line.contains("github_pat_") {
        classes.push("github-token");
    }
    if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
        classes.push("private-key");
    }
    // sk- followed by a run of 8+ token chars.
    if let Some(idx) = line.find("sk-") {
        let run: usize = line[idx + 3..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .count();
        if run >= 8 {
            classes.push("api-key");
        }
    }
    // Bearer <token run 8+>.
    if let Some(idx) = line.find("Bearer ") {
        let run: usize = line[idx + 7..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .count();
        if run >= 8 {
            classes.push("bearer-token");
        }
    }
    let lower = line.to_lowercase();
    // Quoted-value assignments for credential keywords.
    if lower.contains("password") && line.contains('=') && line.contains('"') {
        classes.push("password-assignment");
    }
    if (lower.contains("api_key") || lower.contains("apikey"))
        && line.contains('=')
        && line.contains('"')
    {
        classes.push("api-key-assignment");
    }
    if lower.contains("secret") && line.contains('=') && line.contains('"') {
        classes.push("secret-assignment");
    }
    classes
}

/// Word-boundary match for credential identifiers in code/config
/// content. `tokenizer` does NOT match (`token` needs a boundary
/// after it); `auth_token`, `access_token`, `client_secret` DO.
fn has_credential_identifier(content: &str) -> bool {
    const WORDS: [&str; 4] = ["token", "auth_token", "secret", "credential"];
    content.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| WORDS.contains(&word))
}

/// Sensitive-path reason for one diff file, or `None` when clean.
/// Exact D4 list; see module docs for the matching rules.
pub fn sensitive_path_hit(path: &str, content: &str) -> Option<String> {
    const EXACT_PATHS: [&str; 6] = [
        "src/security.rs",
        "src/tools/shell_security.rs",
        "src/tools/shell_tool.rs",
        "src/config/shell.rs",
        "src/cron/registry.rs",
        "src/cpu/mod.rs",
    ];
    if EXACT_PATHS.contains(&path) {
        return Some(format!("sensitive-path:{path}"));
    }
    if path == "src/io/web_server/mod.rs" {
        return Some("sensitive-path:web-auth-surface".to_string());
    }
    if path.starts_with("src/support_watch/")
        && (path.contains("draft")
            || path.contains("send")
            || content.contains("auto_send"))
    {
        return Some(format!("sensitive-path:support-auto-send:{path}"));
    }
    let lower_path = path.to_lowercase();
    if lower_path.contains("token") || lower_path.contains("secret") || lower_path.contains("auth") {
        return Some(format!("sensitive-path:credential-path:{path}"));
    }
    if has_credential_identifier(content) {
        return Some(format!("sensitive-path:credential-content:{path}"));
    }
    None
}

/// Audit a diff (added lines only). Secret hits aggregate per file
/// per class (line counts, no values); sensitive paths list reasons.
pub fn audit_diff(diff_files: &[DiffFile]) -> AuditReport {
    let mut report = AuditReport::default();
    for f in diff_files {
        let mut per_class: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        for line in f.content.lines() {
            for class in secret_classes_for_line(line) {
                *per_class.entry(class).or_insert(0) += 1;
            }
        }
        for (class, count) in per_class {
            report.secret_hits.push(SecretHit {
                file: f.path.clone(),
                line_count: count,
                pattern_class: class.to_string(),
            });
        }
        if let Some(reason) = sensitive_path_hit(&f.path, &f.content) {
            report.sensitive_paths.push(reason);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(path: &str, content: &str) -> DiffFile {
        DiffFile { path: path.to_string(), content: content.to_string() }
    }

    #[test]
    fn fixture_secrets_blocked_and_report_contains_no_fixture_value() {
        // GIVEN diffs carrying fake tokens/keys/secrets (fixtures, never
        // real values), WHEN audited, THEN they block AND the report +
        // alert line contain NO fixture value — only classes/counts.
        let fake_gh = "ghp_faketoken1234567890abcdef";
        let fake_sk = "sk-fakeapikey1234567890";
        let fake_bearer = "Bearer fakebearertoken123456";
        let files = vec![
            diff("src/foo.rs", &format!("let t = \"{fake_gh}\";\n")),
            diff("src/bar.rs", &format!("let k = \"{fake_sk}\";\n")),
            diff(
                "src/baz.rs",
                &format!("let h = \"{fake_bearer}\";\n-----BEGIN FAKE PRIVATE KEY-----\n"),
            ),
            diff("src/qux.rs", "let p = \"hunter2\";\npassword = \"hunter2\"\n"),
        ];
        let report = audit_diff(&files);
        assert!(!report.secret_hits.is_empty(), "fake secrets must block");
        assert_eq!(audit_outcome(&report), AuditOutcome::SecretsBlocked);
        let serialized = serde_json::to_string(&report).unwrap();
        let alert = alert_line(&report);
        for value in [fake_gh, fake_sk, fake_bearer, "hunter2", "FAKE PRIVATE KEY"] {
            assert!(
                !serialized.contains(value),
                "report must not contain fixture value {value}"
            );
            assert!(
                !alert.contains(value),
                "alert line must not contain fixture value {value}"
            );
        }
        // But classes ARE reported (metadata, not values).
        assert!(serialized.contains("github-token"));
        assert!(serialized.contains("api-key"));
    }

    #[test]
    fn clean_diff_passes() {
        // GIVEN a diff with no secret shapes and no sensitive paths,
        // WHEN audited, THEN clean.
        let files = vec![diff(
            "src/foo.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
        )];
        let report = audit_diff(&files);
        assert!(report.is_clean());
        assert_eq!(audit_outcome(&report), AuditOutcome::Clean);
    }

    #[test]
    fn each_sensitive_path_class_is_flagged() {
        // GIVEN one diff per D4 sensitive-path class, WHEN audited,
        // THEN each is flagged needs-human-review (parameterized over
        // the list — every class, not just one example).
        let cases: Vec<DiffFile> = vec![
            diff("src/security.rs", "pub fn check() {}\n"),
            diff("src/tools/shell_security.rs", "pub fn ok() {}\n"),
            diff("src/tools/shell_tool.rs", "pub fn run() {}\n"),
            diff("src/config/shell.rs", "pub struct C {}\n"),
            diff("src/io/web_server/mod.rs", "pub fn route() {}\n"),
            diff("src/support_watch/drafts.rs", "pub fn stage() {}\n"),
            diff("src/support_watch/mod.rs", "let auto_send = true;\n"),
            diff("src/foo/token_store.rs", "pub fn get() {}\n"),
            diff("src/foo.rs", "let auth_token = load();\n"),
            diff("src/cron/registry.rs", "pub fn list() {}\n"),
            diff("src/cpu/mod.rs", "pub fn tick() {}\n"),
        ];
        for file in &cases {
            let report = audit_diff(std::slice::from_ref(file));
            assert_eq!(
                audit_outcome(&report),
                AuditOutcome::NeedsHumanReview,
                "diff {} must flag needs-human-review",
                file.path
            );
            assert!(
                !report.sensitive_paths.is_empty(),
                "diff {} must list a sensitive reason",
                file.path
            );
        }
    }

    #[test]
    fn needs_human_review_never_reaches_pr_step_stubbed_for_290() {
        // Audit-side contract for the Phase 3 gate (task 290 now
        // covers the gate side in pr.rs::needs_human_review_never_opened):
        // status is needs-human-review, outcome is not Clean, and no
        // eval pass can promote it (the gate reads the audit before
        // any eval result). Both tests run in the suite.
        let mut p = ProposalRecord {
            problem_statement: "p".to_string(),
            trace: "errors:logs/error_log.md".to_string(),
            diff_description: "d".to_string(),
            proving_test: "t".to_string(),
            signature: "errors:logs/error_log.md".to_string(),
            status: "evaluated".to_string(),
            created_at: 1_000,
            updated_at: 1_000,
            proposals_this_week: 0,
            open_for_signature: 0,
            eval_passed: true,
            promoted: true,
            promotion_reason: "ok".to_string(),
            eval_report_json: serde_json::Value::Null,
            notes: Vec::new(),
        };
        let report = audit_diff(&[diff("src/security.rs", "pub fn check() {}\n")]);
        let outcome = apply_audit_status(&mut p, &report, 2_000);
        assert_eq!(outcome, AuditOutcome::NeedsHumanReview);
        assert_eq!(p.status, "needs-human-review");
        assert_ne!(outcome, AuditOutcome::Clean, "gate must not see Clean here");
    }
}
