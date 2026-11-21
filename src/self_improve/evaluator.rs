//! Worktree evaluator (task 287, Phase 2).
//!
//! Evaluates one proposal in a DISPOSABLE worktree: apply its diff,
//! commit it there, run the bounded test subset for the touched
//! modules plus (when the D5 gate is open) the approved eval set, and
//! report. The live tree is never touched — every git op runs through
//! the task-286 wrapper with cwd pinned to the worktree, and the
//! worktree dir is removed afterwards whether the eval passes or
//! fails. The BRANCH is removed too — EXCEPT when the proposal is
//! promoted (task 364): a promoted branch is preserved, its name
//! recorded on the proposal as `eval_branch`, so the Phase 3 delivery
//! path can push it. A non-promoted branch is deleted with the
//! worktree.
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
    /// Point the runner at the worktree dir before running tests.
    /// `evaluate_proposal` calls this right after creating the
    /// worktree (task 325): without it, `RealRunner` would run cargo
    /// in whatever directory it was constructed with, not the
    /// patched worktree. `FakeRunner` records the path for tests.
    fn set_work_dir(&mut self, dir: &Path);
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
    fn set_work_dir(&mut self, dir: &Path) {
        self.work_dir = dir.to_path_buf();
    }

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

/// Env vars a model-authored eval cargo run may see (H10).
/// Everything else — including GEMINI_API_KEY — is dropped by
/// `env_clear()`. Only vars present in this process's env are
/// passed through; absent ones are not invented.
const EVAL_ENV_ALLOWLIST: [&str; 3] = ["PATH", "CARGO_HOME", "RUSTUP_HOME"];

/// Hard timeout for a single evaluator cargo invocation (H10).
/// A fresh worktree's first build takes minutes, so the cap is
/// generous — but a hung cargo must not stall the loop forever.
/// 30 minutes fits inside the default 60-minute cycle budget
/// (`max_cycle_minutes`), leaving room for the second cargo run.
/// The timeout kills the direct cargo child only; orphaned
/// grandchildren are H9's scope (process-group kill).
const EVAL_CARGO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Strip an eval command down to the H10 allowlist: clear the
/// inherited environment, then pass through only PATH/CARGO_HOME/
/// RUSTUP_HOME when present in this process's env. Call before
/// spawning any cargo the loop runs for a model-authored proposal.
fn harden_eval_command(cmd: &mut std::process::Command) {
    cmd.env_clear();
    for key in EVAL_ENV_ALLOWLIST {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
}

/// Run a command, capturing stdout/stderr, with a hard timeout (H10).
/// Spawns with piped output, drains both pipes on reader threads
/// (so a large output can never deadlock on a full pipe buffer),
/// polls `try_wait`, and on deadline kills the child, reaps it, and
/// returns a `TimedOut` error. The child is the direct process
/// only (see EVAL_CARGO_TIMEOUT note re H9). The reader drains use
/// a bounded wait so orphaned grandchildren holding the pipes open
/// cannot hang the caller either.
fn run_command_with_timeout(
    mut cmd: std::process::Command,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (tx_out, rx_out) = std::sync::mpsc::channel();
    let (tx_err, rx_err) = std::sync::mpsc::channel();
    if let Some(mut pipe) = stdout_pipe {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = tx_out.send(buf);
        });
    } else {
        let _ = tx_out.send(Vec::new());
    }
    if let Some(mut pipe) = stderr_pipe {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = tx_err.send(buf);
        });
    } else {
        let _ = tx_err.send(Vec::new());
    }
    // Bounded drain: orphaned grandchildren can hold the pipes
    // open after the child dies; never wait forever for them.
    let drain = |rx: std::sync::mpsc::Receiver<Vec<u8>>| -> Vec<u8> {
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or_default()
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(status) => {
                let stdout = drain(rx_out);
                let stderr = drain(rx_err);
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = drain(rx_out);
                    let _ = drain(rx_err);
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("command timed out after {timeout:?} (killed)"),
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}

fn run_cargo_parsed(mut cmd: std::process::Command, label: &str) -> Vec<TestResult> {
    // H10: the eval cargo run gets a clean env and a hard timeout.
    harden_eval_command(&mut cmd);
    let out = match run_command_with_timeout(cmd, EVAL_CARGO_TIMEOUT) {
        Ok(o) => o,
        Err(e) => {
            let detail = if e.kind() == std::io::ErrorKind::TimedOut {
                format!("cargo timed out after {EVAL_CARGO_TIMEOUT:?} (killed)")
            } else {
                format!("cargo spawn failed: {e}")
            };
            return vec![TestResult {
                name: label.to_string(),
                passed: false,
                detail,
            }];
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

/// Split a diff path into segments on both `/` and `\` (M27).
/// On Windows `\` is a separator, so a segment check that only
/// splits on `/` misses `..\secret` and `.git\config`. On Linux
/// `\` is a literal filename character, so treating it as a
/// separator here is fail-closed: a file literally named `..\x`
/// is rejected, and no legitimate proposal names one.
pub(crate) fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split(|c| c == '/' || c == '\\')
}

/// Validate a diff path: relative, no `..` segments, no absolute path,
/// and never inside a `.git` directory (a diff writing into the
/// worktree's `.git` could corrupt or replace the repository itself).
/// Segments split on both `/` and `\` (M27), so the Windows
/// spellings `..\secret` and `.git\config` are rejected too.
/// Returns the resolved path under the worktree root.
fn resolve_diff_path(worktree: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || Path::new(path).is_absolute() {
        return Err(format!("rejected diff path: {path}"));
    }
    if path_segments(path).any(|seg| seg == "..") {
        return Err(format!("rejected diff path escaping worktree: {path}"));
    }
    // Task 331 (C2): a diff must never touch `.git` — at the root
    // (`.git/config`) or nested (`sub/.git/config`). No legitimate
    // source file lives inside a `.git` directory.
    if path_segments(path).any(|seg| seg == ".git") {
        return Err(format!("rejected diff path inside .git: {path}"));
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
    // Task 364: record the branch on the proposal now — the Phase 3
    // delivery path reads it to push a promoted branch. It is set
    // even when the eval later fails (the branch is then deleted with
    // the worktree, and the record is not the delivery path).
    proposal.eval_branch = branch.clone();
    // Task 325: point the runner at the fresh worktree so cargo runs
    // against the PATCHED code. Before this, RealRunner kept whatever
    // work_dir it was constructed with (nothing set it), so a
    // production evaluation would have tested the wrong tree.
    runner.set_work_dir(&worktree);
    // Cleanup helper: remove the worktree, and the branch unless the
    // eval promoted (task 364: a promoted branch is preserved for
    // the Phase 3 push). Logs failures, never masks the eval result.
    // Runs on every exit path after creation.
    let cleanup = |git: &Git, name: &str, branch: &str, keep_branch: bool| {
        if let Err(e) = git.remove_worktree(name) {
            eprintln!("self_improve: worktree cleanup failed for {name}: {e}");
        }
        if !keep_branch {
            if let Err(e) = git.delete_branch(branch) {
                eprintln!("self_improve: branch cleanup failed for {branch}: {e}");
            }
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
        cleanup(&git, &wt_name, &branch, false);
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
        cleanup(&git, &wt_name, &branch, false);
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
    // Task 364: the persisted record names the branch only when it
    // survives — a non-promoted branch is deleted in cleanup, so a
    // stale name on the record would point at nothing.
    if !promoted {
        proposal.eval_branch = String::new();
    }
    proposal.eval_report_json =
        serde_json::to_value(&report).unwrap_or(serde_json::Value::Null);
    proposal.updated_at = now_ms;
    upsert_proposal(rt_dir, proposal)?;
    // Task 364: a promoted branch survives for the Phase 3 push; the
    // worktree dir is still removed (the branch is a ref, not a dir).
    cleanup(&git, &wt_name, &branch, promoted);
    Ok(report)
}

/// Per-cycle evaluation arm (task 325, H1 — the single
/// highest-leverage harness fix).
///
/// Picks the oldest stored proposal with status "proposed" and
/// evaluates it via `evaluate_proposal`, using the caller's runner.
/// Returns `Ok(Some(report))` when a proposal was evaluated,
/// `Ok(None)` when there is nothing evaluable, and `Err` when the
/// evaluation itself failed (cap, lock, worktree/commit failure —
/// the record is untouched and the caller decides whether to retry).
///
/// Two skip cases, both logged, neither an error:
/// - the oldest "proposed" record carries no diff files. The worker
///   returns only a `diff_description` (prose), so today that is
///   every record; the arm becomes effective when a later phase
///   attaches real diffs. Skipping (rather than calling
///   `evaluate_proposal` with an empty diff, which would just error
///   "no diff files") keeps the cycle log honest.
/// - `repo` is not a git checkout (the worktree is created from it).
pub fn evaluate_oldest_proposal(
    repo: &Path,
    rt_dir: &Path,
    cfg: &crate::self_improve::SelfImproveConfig,
    runner: &mut dyn TestRunner,
    now_ms: u64,
) -> Result<Option<EvalReport>, String> {
    let mut proposals = crate::self_improve::worker::read_proposals(rt_dir)?;
    // Oldest "proposed" record first (by intake time).
    proposals.sort_by_key(|p| p.created_at);
    let Some(mut record) = proposals.into_iter().find(|p| p.status == "proposed") else {
        return Ok(None);
    };
    if record.diff_files.is_empty() {
        eprintln!(
            "self_improve: proposal {} has no diff files; skipping evaluation",
            record.signature
        );
        return Ok(None);
    }
    if !repo.join(".git").exists() {
        eprintln!(
            "self_improve: repo {} is not a git checkout; skipping evaluation",
            repo.display()
        );
        return Ok(None);
    }
    let diff = record.diff_files.clone();
    let report = evaluate_proposal(repo, rt_dir, cfg, &mut record, &diff, runner, now_ms)?;
    Ok(Some(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake runner: canned results + call flags. Never runs cargo.
    /// Records the work dir `evaluate_proposal` points it at (task
    /// 325: proves the runner is aimed at the fresh worktree).
    struct FakeRunner {
        subset: Vec<TestResult>,
        eval_set: Vec<TestResult>,
        eval_set_called: bool,
        subset_called: bool,
        work_dir: Option<PathBuf>,
    }

    impl FakeRunner {
        fn passing() -> Self {
            Self {
                subset: vec![TestResult { name: "a::b".to_string(), passed: true, detail: "ok".to_string() }],
                eval_set: vec![TestResult { name: "eval1".to_string(), passed: true, detail: "ok".to_string() }],
                eval_set_called: false,
                subset_called: false,
                work_dir: None,
            }
        }

        fn failing_subset() -> Self {
            Self {
                subset: vec![TestResult { name: "a::b".to_string(), passed: false, detail: "FAILED".to_string() }],
                eval_set: vec![],
                eval_set_called: false,
                subset_called: false,
                work_dir: None,
            }
        }
    }

    impl TestRunner for FakeRunner {
        fn set_work_dir(&mut self, dir: &Path) {
            self.work_dir = Some(dir.to_path_buf());
        }

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
            diff_files: Vec::new(),
            eval_branch: String::new(),
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
    fn diff_path_inside_git_is_rejected() {
        // Task 331 (C2): a diff must never write into `.git` — the
        // worktree's repository itself. `.gitignore` is a normal file
        // and must still be allowed.
        let wt = Path::new("/tmp/helix-eval-wt");
        assert!(resolve_diff_path(wt, ".git/config").is_err());
        assert!(resolve_diff_path(wt, ".git").is_err());
        assert!(resolve_diff_path(wt, "sub/.git/hooks/pre-commit").is_err());
        assert!(resolve_diff_path(wt, ".gitignore").is_ok());
        assert!(resolve_diff_path(wt, "src/a.rs").is_ok());
        assert_eq!(
            resolve_diff_path(wt, "src/a.rs").unwrap(),
            wt.join("src/a.rs")
        );
    }

    #[test]
    fn harden_eval_command_keeps_only_allowlisted_vars() {
        // H10: after hardening, the only explicitly-set env vars
        // are the allowlisted ones — a secret set on the command
        // before hardening is gone (env_clear dropped it).
        let mut cmd = std::process::Command::new("cargo");
        cmd.env("GEMINI_API_KEY", "fake-secret-for-test");
        cmd.env("SOME_RANDOM_VAR", "x");
        harden_eval_command(&mut cmd);
        let keys: Vec<String> = cmd
            .get_envs()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        assert!(
            !keys.iter().any(|k| k == "GEMINI_API_KEY"),
            "hardened command must not carry GEMINI_API_KEY, got {keys:?}"
        );
        assert!(
            !keys.iter().any(|k| k == "SOME_RANDOM_VAR"),
            "hardened command must not carry unrelated vars, got {keys:?}"
        );
        for key in &keys {
            assert!(
                EVAL_ENV_ALLOWLIST.contains(&key.as_str()),
                "hardened command carries non-allowlisted var {key}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn eval_cargo_env_is_allowlisted() {
        // H10 (behavioral): a hardened command run through the
        // eval runner sees ONLY the allowlisted env — a fake
        // secret set on the command never reaches the child.
        let mut cmd = std::process::Command::new("env");
        cmd.env("GEMINI_API_KEY", "fake-secret-for-test");
        harden_eval_command(&mut cmd);
        let out = run_command_with_timeout(cmd, std::time::Duration::from_secs(10))
            .expect("env must run");
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            !text.contains("fake-secret-for-test"),
            "child env must not contain the fake secret:\n{text}"
        );
        assert!(
            !text.contains("GEMINI_API_KEY"),
            "child env must not contain GEMINI_API_KEY:\n{text}"
        );
        let mut saw_path = false;
        for line in text.lines() {
            let key = line.split('=').next().unwrap_or("");
            assert!(
                EVAL_ENV_ALLOWLIST.contains(&key),
                "child env carries non-allowlisted var {key}:\n{text}"
            );
            if key == "PATH" {
                saw_path = true;
            }
        }
        assert!(saw_path, "child env must carry PATH:\n{text}");
    }

    #[test]
    #[cfg(unix)]
    fn eval_cargo_timeout_kills_hung_command() {
        // H10: a command that never exits is killed at the
        // deadline — the runner returns a timeout error promptly
        // instead of hanging with the child.
        let cmd = std::process::Command::new("sleep");
        let mut cmd = cmd;
        cmd.arg("30");
        let start = std::time::Instant::now();
        let err = run_command_with_timeout(cmd, std::time::Duration::from_secs(1))
            .expect_err("sleep 30 with a 1s timeout must time out");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "timeout must return promptly, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    #[cfg(unix)]
    fn eval_cargo_fast_command_returns_output() {
        // H10: the timeout runner's happy path — a fast command
        // returns its captured output unchanged.
        let mut cmd = std::process::Command::new("echo");
        cmd.arg("hello");
        let out = run_command_with_timeout(cmd, std::time::Duration::from_secs(10))
            .expect("echo must run");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    #[test]
    fn diff_path_adversarial_spellings() {
        // M27: adversarial table for resolve_diff_path (C2). The
        // backslash spellings are the Windows bypass: on Windows
        // `\` is a separator, so `..\secret` and `.git\config`
        // escape / enter .git unless the check splits on `\` too.
        // A redundant separator or a dot segment is not a bypass
        // and stays allowed; a lexical `..` that resolves inside
        // the tree is still rejected, fail-closed.
        let wt = Path::new("/tmp/helix-eval-wt");
        let rejected = [
            "..\\secret",
            "src\\..\\..\\secret",
            ".git\\config",
            "sub\\.git\\hooks\\pre-commit",
            "src/../src/security.rs",
            "..",
            "../secret",
        ];
        for path in rejected {
            assert!(
                resolve_diff_path(wt, path).is_err(),
                "adversarial path {path} must be rejected"
            );
        }
        let allowed = [
            "src\\a.rs",
            "./src/a.rs",
            "src//a.rs",
            "src/a.rs",
            ".gitignore",
        ];
        for path in allowed {
            assert!(
                resolve_diff_path(wt, path).is_ok(),
                "benign path {path} must be allowed"
            );
        }
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
    fn promoted_eval_preserves_branch_and_records_eval_branch() {
        // GIVEN the D5 gate open and all-green runners, WHEN evaluated,
        // THEN promoted=true, the branch SURVIVES (the Phase 3 push
        // needs it), the worktree dir is still removed, and the
        // record carries the branch name in eval_branch (task 364).
        // A non-promoted record must NOT name a branch (the branch
        // is deleted in cleanup, so a stale name would point at
        // nothing).
        let (dir, repo, rt) = fixture("helix-self-improve-eval-preserve");
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
        assert!(report.promoted);
        assert!(!p.eval_branch.is_empty());
        assert!(p.eval_branch.starts_with("self-improve/"));
        let branches = git(&["branch", "--format=%(refname:short)"], &repo);
        assert!(branches.contains(&p.eval_branch), "promoted branch must survive: {branches}");
        let root = rt.join("self_improve/worktrees");
        assert_eq!(
            std::fs::read_dir(&root).map(|rd| rd.flatten().count()).unwrap_or(0),
            0,
            "worktree dir must still be removed"
        );
        let stored = crate::self_improve::worker::read_proposals(&rt).unwrap();
        assert_eq!(stored[0].eval_branch, p.eval_branch);
        // Tidy: remove the preserved branch so the fixture dir is clean.
        git(&["branch", "-D", &p.eval_branch], &repo);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_promoted_eval_clears_eval_branch() {
        // GIVEN the D5 gate closed (no promotion), WHEN evaluated,
        // THEN the branch is deleted AND the record names no branch.
        let (dir, repo, rt) = fixture("helix-self-improve-eval-nopreserve");
        let cfg = cfg_gate_closed();
        let mut p = proposal("errors:logs/error_log.md");
        let diff = vec![DiffFile {
            path: "src/a.rs".to_string(),
            content: "x\n".to_string(),
        }];
        let mut runner = FakeRunner::passing();
        let report =
            evaluate_proposal(&repo, &rt, &cfg, &mut p, &diff, &mut runner, 5_000).unwrap();
        assert!(!report.promoted);
        assert!(p.eval_branch.is_empty());
        let branches = git(&["branch", "--format=%(refname:short)"], &repo);
        assert!(!branches.lines().any(|b| b.starts_with("self-improve/")));
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

    // ── Task 325 (H1): the per-cycle evaluation arm ──────────────────

    #[test]
    fn arm_evaluates_oldest_proposed_record_with_diff() {
        // GIVEN a stored "proposed" record carrying diff files, WHEN
        // the arm runs with a passing fake runner (gate closed), THEN
        // it evaluates the record (status "evaluated", report
        // persisted), and the runner was pointed at the fresh
        // worktree (task 325: set_work_dir wiring).
        let (dir, repo, rt) = fixture("helix-self-improve-arm-eval");
        let cfg = cfg_gate_closed();
        let mut p = proposal("errors:logs/error_log.md");
        p.diff_files = vec![DiffFile {
            path: "src/a.rs".to_string(),
            content: "line one\nline two\nline three\n".to_string(),
        }];
        upsert_proposal(&rt, &p).unwrap();
        let mut runner = FakeRunner::passing();
        let report = evaluate_oldest_proposal(&repo, &rt, &cfg, &mut runner, 5_000)
            .unwrap()
            .expect("arm must evaluate the record with a diff");
        assert!(report.passed);
        assert!(!report.promoted, "gate closed must never promote");
        assert_eq!(report.promotion_reason, "eval-set-not-approved");
        // The runner was aimed at the fresh worktree.
        let work_dir = runner
            .work_dir
            .expect("evaluate_proposal must set the runner work dir");
        assert!(
            work_dir.starts_with(rt.join("self_improve/worktrees")),
            "runner work dir must be under the worktree root: {}",
            work_dir.display()
        );
        // The record was updated + persisted by the evaluation.
        let stored = crate::self_improve::worker::read_proposals(&rt).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, "evaluated");
        assert!(!stored[0].eval_report_json.is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arm_skips_record_without_diff_files() {
        // GIVEN a stored "proposed" record with NO diff files (the
        // worker returns prose only), WHEN the arm runs, THEN it
        // returns None and the record stays "proposed" — the arm does
        // not call evaluate_proposal with an empty diff.
        let (dir, repo, rt) = fixture("helix-self-improve-arm-nodiff");
        let cfg = cfg_gate_closed();
        let p = proposal("errors:logs/error_log.md");
        assert!(p.diff_files.is_empty());
        upsert_proposal(&rt, &p).unwrap();
        let mut runner = FakeRunner::passing();
        let report = evaluate_oldest_proposal(&repo, &rt, &cfg, &mut runner, 5_000).unwrap();
        assert!(report.is_none(), "no diff means nothing to evaluate");
        assert!(!runner.subset_called, "no test run for a diff-less record");
        let stored = crate::self_improve::worker::read_proposals(&rt).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, "proposed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arm_returns_none_when_no_proposals() {
        // GIVEN no stored proposals at all, WHEN the arm runs, THEN
        // it returns None (nothing to evaluate, no error).
        let (dir, repo, rt) = fixture("helix-self-improve-arm-empty");
        let cfg = cfg_gate_closed();
        let mut runner = FakeRunner::passing();
        let report = evaluate_oldest_proposal(&repo, &rt, &cfg, &mut runner, 5_000).unwrap();
        assert!(report.is_none());
        assert!(!runner.subset_called);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
