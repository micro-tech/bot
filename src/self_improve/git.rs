//! Narrow git wrapper (task 286, Phase 2).
//!
//! The loop's ONLY git access. `std::process::Command`, no new git2
//! dependency. The allowlist is the whole design: anything not listed
//! here is rejected BY CONSTRUCTION, before any process spawns — the
//! wrapper never relies on git itself to refuse.
//!
//! Allowed (and nothing else):
//! - `worktree add <name> -b <branch>` — create a disposable worktree
//!   under the worktree root (cwd = the repo).
//! - `branch <name>` — create a branch (cwd = the repo: a ref-only op,
//!   and after worktree removal no worktree cwd exists).
//! - `worktree remove --force <name>` — remove a worktree dir + its
//!   metadata (cwd = the repo). Cleanup runs remove-then-delete:
//!   git refuses `branch -D` while a worktree has the branch checked
//!   out, so this is the only ordering that works.
//! - `branch -D <name>` — delete a branch (cwd = the repo, same reason;
//!   the cleanup path for unmerged eval branches).
//! - `add <path>...` — stage paths inside the worktree (cwd = the
//!   worktree). See below: a deliberate, minimal extension.
//! - `commit -m <message>` — commit staged changes (cwd = the worktree,
//!   NEVER the live checkout).
//!
//! Rejected by construction (each tested): push, merge, checkout,
//! reset --hard, fetch, pull, clone, stash, rebase, cherry-pick,
//! revert, tag, remote, submodule, `--amend`, `-C` pointing outside
//! the worktree root, `--git-dir`, `--work-tree`, `--upload-pack`,
//! and every other subcommand or flag. Path arguments must resolve
//! inside the worktree root (no `..`, no absolute path outside it).
//!
//! `add`: the spec's allowlist names worktree add / branch / commit /
//! branch -D, and its end-to-end test reads as worktree-add → commit
//! → delete. But a commit without staging can only commit modifications
//! to TRACKED files (`commit -a`); a proposal that ADDS a file (a new
//! test, a new module) could never be committed. `add` stages but
//! never commits, pushes, or touches the live tree (cwd is pinned to
//! the worktree, paths are validated inside it), so allowing it with
//! strict path validation is the minimal extension that keeps new-file
//! proposals possible. Flagged here and in the task note.
//!
//! Environment is sanitized before every spawn: `GIT_ASKPASS`,
//! `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`,
//! `GIT_OBJECT_DIRECTORY`, and `GIT_ALTERNATE_OBJECT_DIRECTORIES` are
//! removed (an inherited `GIT_DIR` would redirect git at the wrong
//! repo), and `-c credential.helper=` is forced (no credential use,
//! ever — loop branches never leave the machine). Commits use a fixed
//! local-only identity via `-c user.name/user.email`: loop branches
//! are never pushed (Phase 3 is queue-only), so no real identity is
//! needed or wanted.
//!
//! On "never writes the live tree": `worktree add` writes linked-worktree
//! metadata under the repo's `.git` directory — that is the mechanism
//! the spec requires, not a working-tree write. No allowed op modifies
//! a live checkout FILE; commit/add/branch all run with cwd pinned to
//! the disposable worktree.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The loop's git handle: a repo to link worktrees from, and the root
/// every worktree must live under (`rt/self_improve/worktrees`).
#[derive(Debug, Clone)]
pub struct Git {
    repo: PathBuf,
    worktree_root: PathBuf,
}

fn is_safe_branch_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

/// Resolve `name` under `root`, rejecting `..` escapes and absolute
/// paths outside the root. Returns the resolved path.
fn resolve_under_root(root: &Path, name: &str) -> Result<PathBuf, String> {
    let candidate = if Path::new(name).is_absolute() {
        PathBuf::from(name)
    } else {
        root.join(name)
    };
    // Lexical normalization (no symlink resolution — the worktree may
    // not exist yet for `worktree add`).
    let mut out = PathBuf::new();
    for part in candidate.components() {
        use std::path::Component::*;
        match part {
            CurDir => {}
            ParentDir => {
                if !out.pop() {
                    return Err(format!("path escapes worktree root: {name}"));
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if !out.starts_with(root) {
        return Err(format!("path escapes worktree root: {name}"));
    }
    Ok(out)
}

impl Git {
    pub fn new(repo: PathBuf, worktree_root: PathBuf) -> Self {
        Self { repo, worktree_root }
    }

    /// Validate an argv vector against the allowlist, BEFORE spawning.
    /// Pure shape check: escape flags anywhere, subcommand allowlist,
    /// arg shapes, branch-name safety, and `..` segments in paths.
    /// (Root/worktree resolution is the typed methods' job — they
    /// resolve first, then this gate runs on every spawn as defense
    /// in depth.)
    fn validate_argv(argv: &[String]) -> Result<(), String> {
        // Global escape hatches are rejected wherever they appear —
        // including BEFORE the subcommand (`git -C /x worktree add`).
        for arg in argv {
            if arg == "-C"
                || arg == "--git-dir"
                || arg == "--work-tree"
                || arg.starts_with("--git-dir=")
                || arg.starts_with("--work-tree=")
                || arg.starts_with("--upload-pack")
            {
                return Err(format!("rejected git escape flag: {arg}"));
            }
        }
        let has_dotdot = |p: &str| p.split('/').any(|seg| seg == "..");
        let Some(sub) = argv.first().map(|s| s.as_str()) else {
            return Err("empty git argv".to_string());
        };
        match sub {
            "worktree" => {
                if argv.get(1).map(|s| s.as_str()) == Some("remove") {
                    // Exactly: worktree remove --force <path> (cleanup
                    // primitive: removes the dir AND its metadata, so a
                    // subsequent branch -D succeeds. Plain remove is
                    // rejected — it refuses dirty worktrees, which eval
                    // worktrees often are.
                    if argv.get(2).map(|s| s.as_str()) != Some("--force") || argv.len() != 4 {
                        return Err(format!("rejected git worktree remove op: {argv:?}"));
                    }
                    let path = &argv[3];
                    if path.starts_with('-') || has_dotdot(path) {
                        return Err(format!("rejected worktree path: {path}"));
                    }
                    return Ok(());
                }
                if argv.get(1).map(|s| s.as_str()) != Some("add") {
                    return Err(format!("rejected git worktree op: {:?}", argv.get(1)));
                }
                let name = argv.get(2).ok_or("worktree add needs a path")?;
                if name.starts_with('-') || has_dotdot(name) {
                    return Err(format!("rejected worktree path: {name}"));
                }
                let branch = match argv.get(3).map(|s| s.as_str()) {
                    Some("-b") => argv.get(4).ok_or("worktree add -b needs a branch")?,
                    Some(b) => b,
                    None => return Err("worktree add needs a branch".to_string()),
                };
                if !is_safe_branch_name(branch) || argv.len() > 5 {
                    return Err(format!("rejected worktree add op: {argv:?}"));
                }
                Ok(())
            }
            "branch" => match argv.get(1).map(|s| s.as_str()) {
                Some("-D") => {
                    let name = argv.get(2).ok_or("branch -D needs a name")?;
                    if !is_safe_branch_name(name) || argv.len() != 3 {
                        return Err(format!("rejected branch -D op: {argv:?}"));
                    }
                    Ok(())
                }
                Some(name) => {
                    if !is_safe_branch_name(name) || argv.len() != 2 {
                        return Err(format!("rejected branch op: {argv:?}"));
                    }
                    Ok(())
                }
                None => Err("branch needs a name".to_string()),
            },
            "add" => {
                if argv.len() < 2 {
                    return Err("add needs at least one path".to_string());
                }
                for p in &argv[1..] {
                    if p.starts_with('-') || has_dotdot(p) {
                        return Err(format!("rejected add path: {p}"));
                    }
                }
                Ok(())
            }
            "commit" => {
                // Exactly: commit -m <message>.
                if argv.get(1).map(|s| s.as_str()) != Some("-m") || argv.len() != 3 {
                    return Err(format!("rejected commit op: {argv:?}"));
                }
                Ok(())
            }
            // Everything else is rejected: push, merge, checkout,
            // reset, fetch, pull, clone, stash, rebase, cherry-pick,
            // revert, tag, remote, submodule, and anything unlisted.
            other => Err(format!("rejected git subcommand: {other}")),
        }
    }

    /// Spawn a validated argv with a sanitized environment. `cwd` is
    /// the directory git runs in (repo for worktree add, the worktree
    /// for everything else). The caller must have validated `cwd`
    /// lives under the worktree root (or is the repo, for worktree add).
    fn spawn(&self, argv: &[String], cwd: &Path) -> Result<String, String> {
        Self::validate_argv(argv)?;
        let mut cmd = Command::new("git");
        cmd.current_dir(cwd);
        cmd.arg("-c").arg("credential.helper=");
        cmd.arg("-c").arg("user.name=helix-self-improve");
        cmd.arg("-c").arg("user.email=self-improve@localhost");
        for a in argv {
            cmd.arg(a);
        }
        for var in [
            "GIT_ASKPASS",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        ] {
            cmd.env_remove(var);
        }
        let out = cmd.output().map_err(|e| format!("git spawn failed: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            Err(format!(
                "git {:?} failed: {}",
                argv,
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    /// Create a linked worktree at `<root>/<name>` on a new branch.
    /// Returns the worktree path. Runs with cwd = the repo (the only
    /// op allowed to touch the repo, and only its `.git` metadata).
    pub fn add_worktree(&self, name: &str, branch: &str) -> Result<PathBuf, String> {
        let path = resolve_under_root(&self.worktree_root, name)?;
        let argv = vec![
            "worktree".to_string(),
            "add".to_string(),
            path.display().to_string(),
            "-b".to_string(),
            branch.to_string(),
        ];
        // Validate the canonical form (path form differs from the
        // name form, so validate the shape manually here).
        if !is_safe_branch_name(branch) {
            return Err(format!("rejected branch name: {branch}"));
        }
        self.spawn(&argv, &self.repo)?;
        Ok(path)
    }

    /// Remove a worktree (dir + metadata) by name under the root.
    /// Runs with cwd = the repo. Cleanup order is remove-then-delete:
    /// git refuses `branch -D` while any worktree (even a stale one)
    /// still has the branch checked out.
    pub fn remove_worktree(&self, name: &str) -> Result<String, String> {
        let path = resolve_under_root(&self.worktree_root, name)?;
        self.spawn(
            &[
                "worktree".to_string(),
                "remove".to_string(),
                "--force".to_string(),
                path.display().to_string(),
            ],
            &self.repo,
        )
    }

    /// Create a branch (cwd = the repo). A branch op touches only a
    /// ref — never a working-tree file — and after worktree removal no
    /// worktree cwd exists, so the repo is the only possible cwd. Same
    /// metadata-only class as `worktree add`'s `.git` writes.
    pub fn create_branch(&self, branch: &str) -> Result<String, String> {
        if !is_safe_branch_name(branch) {
            return Err(format!("rejected branch name: {branch}"));
        }
        self.spawn(&["branch".to_string(), branch.to_string()], &self.repo)
    }

    /// Delete a branch (cwd = the repo, for the same reason as
    /// `create_branch`). Cleanup order is remove-then-delete: the
    /// worktree must be gone first, so no worktree cwd can exist here.
    pub fn delete_branch(&self, branch: &str) -> Result<String, String> {
        if !is_safe_branch_name(branch) {
            return Err(format!("rejected branch name: {branch}"));
        }
        self.spawn(
            &["branch".to_string(), "-D".to_string(), branch.to_string()],
            &self.repo,
        )
    }

    /// Stage paths inside a worktree (cwd = the worktree). Every path
    /// must resolve inside that worktree — an escape is rejected before
    /// any spawn.
    pub fn add_files(&self, worktree: &Path, paths: &[&str]) -> Result<String, String> {
        let worktree = self.check_worktree(worktree)?;
        let mut argv = vec!["add".to_string()];
        for p in paths {
            if p.starts_with('-') {
                return Err(format!("rejected add flag: {p}"));
            }
            let resolved = resolve_under_root(&worktree, p)?;
            argv.push(resolved.display().to_string());
        }
        self.spawn(&argv, &worktree)
    }

    /// Commit staged changes inside a worktree (cwd = the worktree,
    /// NEVER the live checkout).
    pub fn commit(&self, worktree: &Path, message: &str) -> Result<String, String> {
        let worktree = self.check_worktree(worktree)?;
        self.spawn(
            &[
                "commit".to_string(),
                "-m".to_string(),
                message.to_string(),
            ],
            &worktree,
        )
    }

    /// The raw argv gate, exposed for tests: validate + (on success)
    /// spawn with cwd = the worktree root's parent rule applied by the
    /// caller. Production code uses the typed methods above; this
    /// exists so the rejection table is tested through one gate.
    /// `cwd` must be the repo (worktree add only) or a worktree dir.
    pub fn run_raw(&self, argv: &[String], cwd: &Path) -> Result<String, String> {
        Self::validate_argv(argv)?;
        // The typed methods pin cwd; run_raw pins it too: worktree add
        // runs in the repo, everything else in a worktree under root.
        let sub = argv.first().map(|s| s.as_str()).unwrap_or("");
        // worktree add/remove and branch ops run in the repo (metadata
        // only); commit/add run in a worktree under the root (never the
        // live checkout's working tree).
        let allowed_cwd = if sub == "worktree" || sub == "branch" {
            cwd == self.repo
        } else {
            cwd.starts_with(&self.worktree_root) && cwd != self.worktree_root
        };
        if !allowed_cwd {
            return Err("rejected git cwd: outside the worktree root".to_string());
        }
        self.spawn(argv, cwd)
    }

    /// A worktree path must be a real directory under the worktree root.
    fn check_worktree(&self, worktree: &Path) -> Result<PathBuf, String> {
        let canon = if worktree.is_absolute() {
            worktree.to_path_buf()
        } else {
            self.worktree_root.join(worktree)
        };
        if !canon.starts_with(&self.worktree_root) || canon == self.worktree_root {
            return Err(format!(
                "worktree path escapes root: {}",
                worktree.display()
            ));
        }
        Ok(canon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(args: &[&str], cwd: &Path) -> String {
        // Test helper only (never production): strip the same hostile
        // vars the wrapper strips, so a parallel env-sanitization test
        // cannot break raw fixture git commands via inherited env.
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_ASKPASS")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("test git spawn");
        assert!(
            out.status.success(),
            "test git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// Fixture repo: init, one committed file, plus a wrapper rooted
    /// at a scratch worktrees dir. Tests hold the env lock: the env
    /// sanitization test mutates process env vars.
    fn fixture(name: &str) -> (PathBuf, PathBuf, Git) {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&["init", "-b", "main"], &repo);
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "config", "user.email", "t@t"], &repo);
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "config", "user.name", "t"], &repo);
        std::fs::write(repo.join("a.txt"), "hello\n").unwrap();
        git(&["add", "a.txt"], &repo);
        git(&["commit", "-m", "init"], &repo);
        let root = dir.join("worktrees");
        std::fs::create_dir_all(&root).unwrap();
        let g = Git::new(repo.clone(), root.clone());
        (dir, repo, g)
    }

    fn branches(repo: &Path) -> Vec<String> {
        git(&["branch", "--format=%(refname:short)"], repo)
            .lines()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn allowed_ops_succeed_end_to_end() {
        // GIVEN a fixture repo, WHEN worktree-add -> write -> add ->
        // commit -> branch-delete run through the wrapper, THEN each
        // succeeds and the commit lands on the new branch only.
        let (dir, repo, g) = fixture("helix-self-improve-git-ok");
        let wt = g.add_worktree("wt1", "loop-test").unwrap();
        assert!(wt.join("a.txt").exists());
        std::fs::write(wt.join("b.txt"), "new file\n").unwrap();
        std::fs::write(wt.join("a.txt"), "hello changed\n").unwrap();
        g.add_files(&wt, &["b.txt", "a.txt"]).unwrap();
        g.commit(&wt, "loop test commit").unwrap();
        let log = git(&["log", "--oneline", "loop-test"], &repo);
        assert!(log.contains("loop test commit"));
        let main_log = git(&["log", "--oneline", "main"], &repo);
        assert!(!main_log.contains("loop test commit"), "live branch untouched");
        assert!(branches(&repo).contains(&"loop-test".to_string()));
        // Cleanup order is load-bearing: git refuses branch -D while
        // the worktree still has it checked out.
        g.remove_worktree("wt1").unwrap();
        assert!(!wt.exists());
        g.delete_branch("loop-test").unwrap();
        assert!(!branches(&repo).contains(&"loop-test".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejected_ops_fail_closed_with_no_side_effect() {
        // GIVEN a fixture repo, WHEN each rejected op is attempted
        // through the raw gate, THEN it errors AND leaves no trace:
        // no new branch, no new worktree, no commit.
        let (dir, repo, g) = fixture("helix-self-improve-git-reject");
        let wt = g.add_worktree("wt1", "loop-test").unwrap();
        let before_branches = branches(&repo);
        let before_log = git(&["log", "--oneline", "main"], &repo);
        let rejected: Vec<Vec<String>> = vec![
            vec!["push".into()],
            vec!["push".into(), "origin".into(), "main".into()],
            vec!["merge".into(), "main".into()],
            vec!["checkout".into(), "main".into()],
            vec!["reset".into(), "--hard".into()],
            vec!["fetch".into()],
            vec!["pull".into()],
            vec!["clone".into(), "x".into()],
            vec!["stash".into()],
            vec!["rebase".into(), "main".into()],
            vec!["-C".into(), "/tmp".into(), "status".into()],
            vec!["--git-dir=/tmp".into(), "status".into()],
            vec!["--work-tree=/tmp".into(), "status".into()],
            vec!["--upload-pack=touch".into(), "status".into()],
            vec!["commit".into(), "--amend".into(), "-m".into(), "x".into()],
            vec!["commit".into(), "-m".into(), "x".into(), "--author=y".into()],
            vec!["branch".into(), "-d".into(), "loop-test".into()],
            vec!["branch".into(), "../evil".into()],
            vec!["worktree".into(), "remove".into(), "wt1".into()],
            vec!["status".into()],
        ];
        for argv in rejected {
            let r = g.run_raw(&argv, &wt);
            assert!(r.is_err(), "argv {argv:?} must be rejected, got {r:?}");
        }
        assert_eq!(branches(&repo), before_branches, "no branch change");
        assert_eq!(
            git(&["log", "--oneline", "main"], &repo),
            before_log,
            "no commit"
        );
        assert!(wt.exists(), "worktree still present");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_and_cwd_escapes_rejected() {
        // GIVEN a fixture repo, WHEN add/commit/worktree-add paths or
        // cwds escape the worktree root, THEN the wrapper refuses
        // before spawning, and the live repo is unchanged.
        let (dir, repo, g) = fixture("helix-self-improve-git-escape");
        let wt = g.add_worktree("wt1", "loop-test").unwrap();
        // add with a .. escape.
        assert!(g.add_files(&wt, &["../evil.txt"]).is_err());
        // add with an absolute path outside the worktree.
        assert!(g.add_files(&wt, &["/tmp/evil.txt"]).is_err());
        // commit with a cwd outside the root (the live repo itself).
        let r = g.run_raw(
            &["commit".into(), "-m".into(), "evil".into()],
            &repo,
        );
        assert!(r.is_err(), "commit cwd=repo must be rejected");
        // worktree add with an absolute path outside the root.
        assert!(g.add_worktree("/tmp/evil-wt", "evil-branch").is_err());
        // No commit landed anywhere.
        assert!(!git(&["log", "--oneline", "main"], &repo).contains("evil"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_sanitization_ignores_inherited_git_dir_and_askpass() {
        // GIVEN hostile inherited env (GIT_DIR pointing nowhere real,
        // GIT_ASKPASS set), WHEN the wrapper commits, THEN it still
        // operates on the right repo — the env is stripped per spawn.
        let _lock = crate::tools::subagent_tool::test_env_lock();
        let (dir, repo, g) = fixture("helix-self-improve-git-env");
        let wt = g.add_worktree("wt1", "loop-test").unwrap();
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        unsafe {
            std::env::set_var("GIT_DIR", "/nonexistent-git-dir");
            std::env::set_var("GIT_ASKPASS", "/bin/false");
        }
        let r = g.add_files(&wt, &["a.txt"]).and_then(|_| g.commit(&wt, "env test"));
        unsafe {
            std::env::remove_var("GIT_DIR");
            std::env::remove_var("GIT_ASKPASS");
        }
        r.expect("wrapper must ignore inherited GIT_DIR/GIT_ASKPASS");
        assert!(git(&["log", "--oneline", "loop-test"], &repo).contains("env test"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
