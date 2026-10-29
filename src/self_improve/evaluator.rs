//! Worktree evaluator (task 287, Phase 2).
//!
//! Evaluates one proposal in a DISPOSABLE worktree: apply its diff,
//! commit it there, run the bounded test subset for the touched
//! modules plus (when the D5 gate is open) the approved eval set, and
//! report. The live tree is never touched — every git op runs through
//! the task-286 wrapper with cwd pinned to the worktree, and the
//! worktree + branch are removed afterwards whether the eval passes
//! or fails.
//!
//! The diff is a list of `DiffFile{path, content}` WHOLE-FILE writes,
//! not a unified diff: `git apply` is deliberately excluded from the
//! 286 allowlist, and adding it would violate that task. Apply = write
//! each file under the worktree root (rejecting `..` and absolute
//! paths escaping the root), then commit via the wrapper.
//!
//! D5 gate: while `[self_improve] eval_set_approved` is false (the
//! task-242 set is still DRAFT), the eval set is SKIPPED and every
//! proposal is marked `promoted=false` with reason
//! `eval-set-not-approved` — even on a green subset. Promotion requires
//! subset + full eval set green AND the gate open. The 500-line cap
//! counts new-content lines across `diff_files` and rejects pre-eval
//! (no worktree created, no test run).
//!
//! Max-1-worktree: a lock file under the worktree root admits one live
//! evaluation; a second eval while one is alive is refused. A 24h
//! orphan sweep removes stale worktree dirs (a crashed eval's
//! leftovers); the sweep takes an explicit clock so tests can plant
//! stale dirs without backdating mtimes.
//!
//! The evaluator does NOT audit secrets or sensitive paths — that is
//! task 288's component, composed with this one at the Phase 3 gate
//! (task 290). It also does NOT decide the handler's Phase 2 cycle
//! flow: `evaluate_proposal` is the callable component, and the
//! per-cycle "evaluate the oldest proposed record" wiring is a
//! follow-up handler arm (flagged in the task note — no task in
//! 277–291 owns it).

use crate::self_improve::git::Git;
use crate::self_improve::worker::{ProposalRecord, upsert_proposal};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One whole-file write in a proposal's diff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffFile {
    pub path: String,
    pub content: String,
}

/// One test's outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Diff statistics, computed in Rust (not via git): files changed,
/// new-content lines, and old lines for replaced files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiffStat {
    pub files_changed: usize,
    pub new_lines: usize,
    pub old_lines: usize,
}

/// The full evaluation report. Persisted onto the proposal record as
/// JSON (see `ProposalRecord.eval_report_json`); the Phase 3 gate
/// (task 290) reads `promoted` + `promotion_reason` from it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalReport {
    pub passed: bool,
    pub promoted: bool,
    pub promotion_reason: String,
    pub subset_results: Vec<TestResult>,
    pub eval_set_results: Vec<TestResult>,
    pub diff_stat: DiffStat,
    pub runtime_ms: u64,
}

/// Runs tests. Production uses `RealRunner` (cargo in the worktree);
/// unit tests use `FakeRunner` (never real cargo).
pub trait TestRunner {
    /// Run the lib subset for one module filter ("" = full lib suite).
    fn run_subset(&mut self, filter: &str) -> Vec<TestResult>;
    /// Run the approved eval set (`cargo test --test evals`).
    fn run_eval_set(&mut self) -> Vec<TestResult>;
}

/// Cargo-backed runner: runs in the worktree dir, so tests execute
/// against the PATCHED code. Parses `test <name> ... ok|FAILED` lines
/// for per-test results; a run with no parseable lines becomes a
/// single synthetic result (pass on exit 0, fail with the stderr tail
/// otherwise). NOTE: a fresh worktree has no `target/` dir, so the
/// first eval build is a full build (minutes) — correct, but heavy;
/// the cycle budget (task 283) bounds it handler-side.
pub struct RealRunner {
    pub work_dir: PathBuf,
}

impl TestRunner for RealRunner {
    fn run_subset(&mut self, filter: &str) -> Vec<TestResult> {
        let mut cmd = std::process::Command::new("cargo");
        cmd.arg("test").arg("--lib");
        if !filter.is_empty() {
            cmd.arg("--").arg(filter);
        }
        cmd.current_dir(&self.work_dir);
        run_cargo_parsed(cmd, filter)
    }

    fn run_eval_set(&mut self) -> Vec<TestResult> {
        let mut cmd = std::process::Command::new("cargo");
        cmd.arg("test").arg("--test").arg("evals");
        cmd.current_dir(&self.work_dir);
        run_cargo_parsed(cmd, "evals")
    }
}

fn run_cargo_parsed(mut cmd: std::process::Command, label: &str) -> Vec<TestResult> {
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            return vec![TestResult {
                name: label.to_string(),
                passed: false,
                detail: format!("cargo spawn failed: {e}"),
            }]
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut results = Vec::new();
    for line in stdout.lines() {
        // `test path::name ... ok` / `... FAILED` / `... ignored`.
        if let Some(rest) = line.strip_prefix("test ") {
            if let Some((name, verdict)) = rest.split_once(" ... ") {
                let verdict = verdict.trim();
                if matches!(verdict, "ok" | "FAILED" | "ignored") {
                    results.push(TestResult {
                        name: name.to_string(),
                        passed: verdict == "ok" || verdict == "ignored",
                        detail: verdict.to_string(),
                    });
                }
            }
        }
    }
    if results.is_empty() {
        let stderr_tail: String = String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        results.push(TestResult {
            name: label.to_string(),
            passed: out.status.success(),
            detail: if out.status.success() {
                "no test lines parsed, exit 0".to_string()
            } else {
                format!("cargo failed: {stderr_tail}")
            },
        });
    }
    results
}

/// Derive the lib-test module filter for a touched path, or `None`
/// when the path is not a lib module (then the caller falls back to
/// the full lib suite — conservative: a change to `main.rs` or a doc
/// file must not silently skip all tests).
/// - `src/self_improve/budgets.rs` → `self_improve::budgets`
/// - `src/cpu/mod.rs` → `cpu`
/// - `src/main.rs`, `Cargo.toml`, `docs/x.md` → `None`
pub fn module_filter_for_path(path: &str) -> Option<String> {
    let rel = path.strip_prefix("src/")?;
    if !rel.ends_with(".rs") {
        return None;
    }
    if rel == "main.rs" || rel == "lib.rs" {
        return None;
    }
    let no_ext = rel.strip_suffix(".rs").unwrap();
    let module = no_ext
        .strip_suffix("/mod")
        .unwrap_or(no_ext)
        .replace('/', "::");
    Some(module)
}

/// Sanitize a signature into a worktree/branch-safe slug.
pub fn slugify_signature(signature: &str) -> String {
    let mut slug: String = signature
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else {
                '-'
            }
        })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-').to_string();
    let slug: String = slug.chars().take(40).collect();
    if slug.is_empty() {
        "eval".to_string()
    } else {
        slug
    }
}

/// Validate a diff path: relative, no `..` segments, no absolute path.
/// Returns the resolved path under the worktree root.
fn resolve_diff_path(worktree: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || Path::new(path).is_absolute() {
        return Err(format!("rejected diff path: {path}"));
    }
    if path.split('/').any(|seg| seg == "..") {
        return Err(format!("rejected diff path escaping worktree: {path}"));
    }
    Ok(worktree.join(path))
}

/// The max-1-worktree lock: a file under the worktree root. Dropped on
/// scope exit (RAII) so a failed eval never leaves a live lock behind.
/// A lock older than 24h is treated as stale (crashed eval) and
/// replaced, logged to stderr.
struct EvalLock {
    path: PathBuf,
}

impl EvalLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        let path = root.join(".eval.lock");
        if path.exists() {
            let stale = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|age| age.as_secs() > 24 * 60 * 60)
                .unwrap_or(false);
            if !stale {
                return Err(
                    "another evaluation is already running (max-1-worktree)".to_string(),
                );
            }
            eprintln!("self_improve: eval lock stale (>24h), replacing");
            let _ = std::fs::remove_file(&path);
        }
        std::fs::write(&path, format!("pid={} start_ms={}", std::process::id(), crate::utils::now_ms()))
            .map_err(|e| format!("eval lock create failed: {e}"))?;
        Ok(Self { path })
    }
}

impl Drop for EvalLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Remove worktree dirs under `root` whose mtime is older than
/// `max_age_ms` before `now_ms` (default call: 24h). The lock file
/// itself is never removed. Returns the removed dir names.
pub fn sweep_stale_worktrees(root: &Path, now_ms: u64, max_age_ms: u64) -> Vec<String> {
    let mut removed = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return removed;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let mtime_ms = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if now_ms.saturating_sub(mtime_ms) > max_age_ms {
            if std::fs::remove_dir_all(&path).is_ok() {
                eprintln!("self_improve: swept stale worktree {}", path.display());
                removed.push(path.display().to_string());
            }
        }
    }
    removed
}

/// Evaluate one proposal. `repo` is the Helix checkout to link
/// worktrees from; `rt_dir` carries the worktree root
/// (`self_improve/worktrees` per config) and the proposals file.
/// On `Ok`, the record is updated (status, promoted, reason, full
/// report JSON) and persisted via upsert. On `Err`, no evaluation
/// happened (cap, lock, worktree/commit failure) and the record is
/// untouched — the caller decides whether to retry.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_proposal(
    repo: &Path,
    rt_dir: &Path,
    cfg: &crate::self_improve::SelfImproveConfig,
    proposal: &mut ProposalRecord,
    diff_files: &[DiffFile],
    runner: &mut dyn TestRunner,
    now_ms: u64,
) -> Result<EvalReport, String> {
    let started = std::time::Instant::now();
    // Pre-eval rejections: no diff, or over the line cap. No worktree
    // is created and no test runs for either.
    if diff_files.is_empty() {
        return Err("no diff files to evaluate".to_string());
    }
    let new_lines: usize = diff_files
        .iter()
        .map(|f| f.content.lines().count())
        .sum();
    if new_lines > cfg.max_diff_lines {
        return Err(format!(
            "diff of {new_lines} lines exceeds the {}-line cap (rejected pre-eval)",
            cfg.max_diff_lines
        ));
    }
    let root = rt_dir.join(&cfg.worktree_dir);
    std::fs::create_dir_all(&root).map_err(|e| format!("worktree root create failed: {e}"))?;
    let _lock = EvalLock::acquire(&root)?;
    let slug = slugify_signature(&proposal.signature);
    let wt_name = format!("eval-{slug}-{now_ms}");
    let branch = format!("self-improve/{slug}-{now_ms}");
    let git = Git::new(repo.to_path_buf(), root.clone());
    let worktree = git.add_worktree(&wt_name, &branch)?;
    // Cleanup helper: remove worktree + branch, logging (never masking
    // the eval result). Runs on every exit path after creation.
    let cleanup = |git: &Git, name: &str, branch: &str| {
        if let Err(e) = git.remove_worktree(name) {
            eprintln!("self_improve: worktree cleanup failed for {name}: {e}");
        }
        if let Err(e) = git.delete_branch(branch) {
            eprintln!("self_improve: branch cleanup failed for {branch}: {e}");
        }
    };
    // Apply: read old lines first (for the stat), then write.
    let mut old_lines = 0usize;
    let mut paths: Vec<String> = Vec::new();
    let apply = (|| -> Result<(), String> {
        for f in diff_files {
            let target = resolve_diff_path(&worktree, &f.path)?;
            if target.exists() {
                let old = std::fs::read_to_string(&target)
                    .map_err(|e| format!("diff target read failed for {}: {e}", f.path))?;
                old_lines += old.lines().count();
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("diff parent create failed for {}: {e}", f.path))?;
            }
            std::fs::write(&target, &f.content)
                .map_err(|e| format!("diff write failed for {}: {e}", f.path))?;
            paths.push(f.path.clone());
        }
        Ok(())
    })();
    if let Err(e) = apply {
        cleanup(&git, &wt_name, &branch);
        return Err(e);
    }
    let stat = DiffStat {
        files_changed: diff_files.len(),
        new_lines,
        old_lines,
    };
    if let Err(e) = git
        .add_files(&worktree, &paths.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        .and_then(|_| git.commit(&worktree, &format!("self-improve eval: {}", proposal.signature)))
    {
        cleanup(&git, &wt_name, &branch);
        return Err(format!("eval commit failed: {e}"));
    }
    // Bounded subset: one cargo run per touched lib module (or the full
    // suite when no module filter derives).
    let mut filters: Vec<String> = diff_files
        .iter()
        .filter_map(|f| module_filter_for_path(&f.path))
        .collect();
    filters.sort();
    filters.dedup();
    let mut subset_results = Vec::new();
    if filters.is_empty() {
        subset_results.extend(runner.run_subset(""));
    } else {
        for filter in &filters {
            subset_results.extend(runner.run_subset(filter));
        }
    }
    let subset_passed = subset_results.iter().all(|r| r.passed);
    // D5 gate + promotion, in order: subset fail beats gate-closed
    // (the eval itself failed); subset pass + gate closed never
    // promotes and never runs the eval set; all green + gate open
    // promotes.
    let (passed, promoted, promotion_reason, eval_set_results) = if !subset_passed {
        (false, false, "subset-failed".to_string(), Vec::new())
    } else if !cfg.eval_set_approved {
        (true, false, "eval-set-not-approved".to_string(), Vec::new())
    } else {
        let eval_results = runner.run_eval_set();
        let eval_passed = eval_results.iter().all(|r| r.passed);
        if eval_passed {
            (true, true, "ok".to_string(), eval_results)
        } else {
            (false, false, "eval-set-failed".to_string(), eval_results)
        }
    };
    let report = EvalReport {
        passed,
        promoted,
        promotion_reason: promotion_reason.clone(),
        subset_results,
        eval_set_results,
        diff_stat: stat,
        runtime_ms: started.elapsed().as_millis() as u64,
    };
    proposal.status = if passed { "evaluated".to_string() } else { "eval-failed".to_string() };
    proposal.eval_passed = passed;
    proposal.promoted = promoted;
    proposal.promotion_reason = promotion_reason;
    proposal.eval_report_json =
        serde_json::to_value(&report).unwrap_or(serde_json::Value::Null);
    proposal.updated_at = now_ms;
    upsert_proposal(rt_dir, proposal)?;
    cleanup(&git, &wt_name, &branch);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake runner: canned results + call flags. Never runs cargo.
    struct FakeRunner {
        subset: Vec<TestResult>,
        eval_set: Vec<TestResult>,
        eval_set_called: bool,
        subset_called: bool,
    }

    impl FakeRunner {
        fn passing() -> Self {
            Self {
                subset: vec![TestResult { name: "a::b".to_string(), passed: true, detail: "ok".to_string() }],
                eval_set: vec![TestResult { name: "eval1".to_string(), passed: true, detail: "ok".to_string() }],
                eval_set_called: false,
                subset_called: false,
            }
        }

        fn failing_subset() -> Self {
            Self {
                subset: vec![TestResult { name: "a::b".to_string(), passed: false, detail: "FAILED".to_string() }],
                eval_set: vec![],
                eval_set_called: false,
                subset_called: false,
            }
        }
    }

    impl TestRunner for FakeRunner {
        fn run_subset(&mut self, _filter: &str) -> Vec<TestResult> {
            self.subset_called = true;
            self.subset.clone()
        }

        fn run_eval_set(&mut self) -> Vec<TestResult> {
            self.eval_set_called = true;
            self.eval_set.clone()
        }
    }

    fn git(args: &[&str], cwd: &Path) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_ASKPASS")
            .output()
            .expect("test git spawn");
        assert!(
            out.status.success(),
            "test git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&["init", "-b", "main"], &repo);
        git(&["config", "user.email", "t@t"], &repo);
        git(&["config", "user.name", "t"], &repo);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src").join("a.rs"), "line one\nline two\n").unwrap();
        git(&["add", "."], &repo);
        git(&["commit", "-m", "init"], &repo);
        let rt = dir.join("rt");
        std::fs::create_dir_all(&rt).unwrap();
        (dir, repo, rt)
    }

    fn proposal(sig: &str) -> ProposalRecord {
        ProposalRecord {
            problem_statement: "p".to_string(),
            trace: sig.to_string(),
            diff_description: "d".to_string(),
            proving_test: "t".to_string(),
            signature: sig.to_string(),
            status: "proposed".to_string(),
            created_at: 1_000,
            updated_at: 1_000,
            proposals_this_week: 0,
            open_for_signature: 0,
            eval_passed: false,
            promoted: false,
            promotion_reason: String::new(),
            eval_report_json: serde_json::Value::Null,
            notes: Vec::new(),
        }
    }

    fn cfg_gate_closed() -> crate::self_improve::SelfImproveConfig {
        // Defaults already gate-closed (eval_set_approved=false).
        crate::self_improve::SelfImproveConfig::default()
    }

    #[test]
    fn module_filter_derivation() {
        assert_eq!(
            module_filter_for_path("src/self_improve/budgets.rs").as_deref(),
            Some("self_improve::budgets")
        );
        assert_eq!(
            module_filter_for_path("src/cpu/mod.rs").as_deref(),
            Some("cpu")
        );
        assert_eq!(module_filter_for_path("src/main.rs"), None);
        assert_eq!(module_filter_for_path("Cargo.toml"), None);
        assert_eq!(module_filter_for_path("docs/x.md"), None);
        assert_eq!(module_filter_for_path("src/a/b/c.rs").as_deref(), Some("a::b::c"));
    }

    #[test]
    fn green_eval_end_to_end_with_correct_report_and_cleanup() {
        // GIVEN a fixture repo + a proposal with a small diff,
        // WHEN evaluated with a passing fake runner (gate closed),
        // THEN the report is correct, the record is updated + persisted,
        // and NO worktree dir or branch remains.
        let (dir, repo, rt) = fixture("helix-self-improve-eval-green");
        let cfg = cfg_gate_closed();
        let mut p = proposal("errors:logs/error_log.md");
        let diff = vec![DiffFile {
            path: "src/a.rs".to_string(),
            content: "line one\nline two\nline three\n".to_string(),
        }];
        let mut runner = FakeRunner::passing();
        let report =
            evaluate_proposal(&repo, &rt, &cfg, &mut p, &diff, &mut runner, 5_000).unwrap();
        assert!(report.passed);
        assert!(!report.promoted, "gate closed must never promote");
        assert_eq!(report.promotion_reason, "eval-set-not-approved");
        assert!(!runner.eval_set_called, "gate closed must not run the eval set");
        assert_eq!(report.diff_stat.files_changed, 1);
        assert_eq!(report.diff_stat.new_lines, 3);
        assert_eq!(report.diff_stat.old_lines, 2);
        assert_eq!(p.status, "evaluated");
        assert!(!p.promoted);
        // Worktree dir gone, branch gone, record persisted.
        let root = rt.join("self_improve/worktrees");
        let remaining: usize = std::fs::read_dir(&root)
            .map(|rd| rd.flatten().count())
            .unwrap_or(0);
        assert_eq!(remaining, 0, "no worktree dir may remain");
        let branches = git(&["branch", "--format=%(refname:short)"], &repo);
        assert!(!branches.contains("self-improve"), "eval branch must be deleted");
        let stored = crate::self_improve::worker::read_proposals(&rt).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, "evaluated");
        assert!(!stored[0].eval_report_json.is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_subset_produces_fail_report_and_cleanup() {
        // GIVEN a failing subset, WHEN evaluated, THEN the report fails
        // with reason subset-failed, the record is eval-failed (which
        // starts the 7-day cooldown), and cleanup still runs.
        let (dir, repo, rt) = fixture("helix-self-improve-eval-fail");
        let cfg = cfg_gate_closed();
        let mut p = proposal("errors:logs/error_log.md");
        let diff = vec![DiffFile {
            path: "src/a.rs".to_string(),
            content: "changed\n".to_string(),
        }];
        let mut runner = FakeRunner::failing_subset();
        let report =
            evaluate_proposal(&repo, &rt, &cfg, &mut p, &diff, &mut runner, 5_000).unwrap();
        assert!(!report.passed);
        assert!(!report.promoted);
        assert_eq!(report.promotion_reason, "subset-failed");
        assert_eq!(p.status, "eval-failed");
        let root = rt.join("self_improve/worktrees");
        assert_eq!(
            std::fs::read_dir(&root).map(|rd| rd.flatten().count()).unwrap_or(0),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gate_closed_never_promotes_even_on_green() {
        // GIVEN an all-green fake runner but the D5 gate closed,
        // WHEN evaluated, THEN promoted=false with reason
        // eval-set-not-approved and the eval set never runs.
        let (dir, repo, rt) = fixture("helix-self-improve-eval-gate");
        let cfg = cfg_gate_closed();
        assert!(!cfg.eval_set_approved);
        let mut p = proposal("errors:logs/error_log.md");
        let diff = vec![DiffFile {
            path: "src/a.rs".to_string(),
            content: "x\n".to_string(),
        }];
        let mut runner = FakeRunner::passing();
        let report =
            evaluate_proposal(&repo, &rt, &cfg, &mut p, &diff, &mut runner, 5_000).unwrap();
        assert!(report.passed);
        assert!(!report.promoted);
        assert_eq!(report.promotion_reason, "eval-set-not-approved");
        assert!(!runner.eval_set_called);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gate_open_promotes_on_all_green() {
        // GIVEN the D5 gate open and all-green runners, WHEN evaluated,
        // THEN promoted=true with reason ok and the eval set ran.
        let (dir, repo, rt) = fixture("helix-self-improve-eval-open");
        let mut cfg = cfg_gate_closed();
        cfg.eval_set_approved = true;
        let mut p = proposal("errors:logs/error_log.md");
        let diff = vec![DiffFile {
            path: "src/a.rs".to_string(),
            content: "x\n".to_string(),
        }];
        let mut runner = FakeRunner::passing();
        let report =
            evaluate_proposal(&repo, &rt, &cfg, &mut p, &diff, &mut runner, 5_000).unwrap();
        assert!(report.passed);
        assert!(report.promoted);
        assert_eq!(report.promotion_reason, "ok");
        assert!(runner.eval_set_called);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_over_cap_rejected_pre_eval() {
        // GIVEN a diff over 500 lines, WHEN evaluated, THEN it errors
        // BEFORE any worktree or test run (runner never called, no dir).
        let (dir, repo, rt) = fixture("helix-self-improve-eval-cap");
        let cfg = cfg_gate_closed();
        let mut p = proposal("errors:logs/error_log.md");
        let big = (0..501).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let diff = vec![DiffFile { path: "src/a.rs".to_string(), content: big }];
        let mut runner = FakeRunner::passing();
        let r = evaluate_proposal(&repo, &rt, &cfg, &mut p, &diff, &mut runner, 5_000);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("exceeds the 500-line cap"));
        assert!(!runner.subset_called, "no test may run for an over-cap diff");
        assert!(!rt.join("self_improve/worktrees").exists(), "no worktree may be created");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_one_worktree_enforced() {
        // GIVEN a live lock file (another eval in flight), WHEN a
        // second eval starts, THEN it is refused with no worktree.
        let (dir, repo, rt) = fixture("helix-self-improve-eval-lock");
        let cfg = cfg_gate_closed();
        let root = rt.join("self_improve/worktrees");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".eval.lock"), "pid=1 start_ms=1").unwrap();
        let mut p = proposal("errors:logs/error_log.md");
        let diff = vec![DiffFile { path: "src/a.rs".to_string(), content: "x\n".to_string() }];
        let mut runner = FakeRunner::passing();
        let r = evaluate_proposal(&repo, &rt, &cfg, &mut p, &diff, &mut runner, 5_000);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("already running"));
        assert!(!runner.subset_called);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn orphan_sweep_removes_only_stale_dirs() {
        // GIVEN a worktree root with two fresh dirs, WHEN swept with a
        // clock 25h ahead (both count as stale), THEN both are removed;
        // and a fresh dir swept with the current clock survives.
        let dir = std::env::temp_dir().join("helix-self-improve-eval-sweep");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("worktrees");
        std::fs::create_dir_all(root.join("old-wt")).unwrap();
        std::fs::create_dir_all(root.join("new-wt")).unwrap();
        let removed = sweep_stale_worktrees(
            &root,
            crate::utils::now_ms() + 25 * 60 * 60 * 1000,
            24 * 60 * 60 * 1000,
        );
        assert_eq!(removed.len(), 2, "both dirs are stale against a +25h clock");
        assert!(!root.join("old-wt").exists());
        assert!(!root.join("new-wt").exists());
        std::fs::create_dir_all(root.join("fresh-wt")).unwrap();
        let removed =
            sweep_stale_worktrees(&root, crate::utils::now_ms(), 24 * 60 * 60 * 1000);
        assert!(removed.is_empty(), "a fresh dir must survive");
        assert!(root.join("fresh-wt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_path_escapes_rejected() {
        // GIVEN a diff escaping the worktree, WHEN evaluated, THEN it
        // errors and no file is written outside (and cleanup runs).
        let (dir, repo, rt) = fixture("helix-self-improve-eval-escape");
        let cfg = cfg_gate_closed();
        let mut p = proposal("errors:logs/error_log.md");
        let diff = vec![DiffFile {
            path: "../evil.txt".to_string(),
            content: "evil\n".to_string(),
        }];
        let mut runner = FakeRunner::passing();
        let r = evaluate_proposal(&repo, &rt, &cfg, &mut p, &diff, &mut runner, 5_000);
        assert!(r.is_err());
        assert!(!dir.join("evil.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
