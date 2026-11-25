//! Loop drafting path (task 362, Phase 2).
//!
//! The worker returns prose only (`diff_description`), and the
//! evaluator skips records with no diff files — so today the eval arm
//! never fires. This module adds the missing code-drafting step
//! between intake and evaluation: a narrow drafting role turns an
//! intaked proposal into whole-file `DiffFile`s, handler-side.
//!
//! Triggers, in order (the drafter is NOT self-directed):
//! 1. An eval finding / ledger signal first — a failing test name or
//!    metric key traceable under the R3 rule (the worker's `trace`).
//!    The drafting arm only drafts proposals the worker already
//!    intaked from such a signal.
//! 2. A user request — John asks for a change; the proposal is
//!    intaked from his words and drafted the same way.
//! 3. A self-detected gap is the LONG-TERM trigger, not the first:
//!    the loop does not yet go looking for gaps on its own.
//!
//! The drafter is a read-only investigator like the worker: it reads
//! repo files, logs, and notes, and returns code as text. It writes
//! NOTHING itself — `attach_draft` (handler-side) is the only writer,
//! and it enforces the 500-line cap before upserting. The drafting
//! role uses the SAME 8 read-only tools as the loop worker
//! (`LOOP_WORKER_TOOLS`); the allowlist is NOT widened for drafting,
//! and the same fail-closed filter applies (a zero match refuses the
//! spawn rather than falling back to the full toolset).
//!
//! Draft shape: whole-file `DiffFile`s — the evaluator's contract.
//! The drafter returns one `--- file: <path> ---` block per file with
//! the FULL new content of that file. `parse_draft_response` validates
//! every path with the same rules as the evaluator (`resolve_diff_path`:
//! relative, no `..`, no absolute, never inside `.git`) — and the
//! evaluator re-validates at apply time (defense in depth).
//!
//! Delivery is unchanged: a drafted proposal goes through the worktree
//! evaluator (bounded subset + approved eval set, 500-line cap, audit)
//! and the PR payload via `phase3_gate`, target PreRelese only, John's
//! merge tap. Drafting needs Phase 2 to evaluate, Phase 3 to deliver,
//! and the loop actually running (dep 363).

use crate::self_improve::evaluator::{DiffFile, path_segments};
use crate::self_improve::worker::{ProposalRecord, upsert_proposal};
use std::path::{Path, PathBuf};

/// Persona file content, baked in from the repo source of truth
/// (`agents/loop-drafter.md`). The handler seeds this into the runtime
/// persona dir when missing; John's edits to the seeded copy always win.
pub const LOOP_DRAFTER_PERSONA: &str = include_str!("../../agents/loop-drafter.md");

/// The fail-closed tool definitions for a `loop-drafter` spawn: the
/// persona mechanism's output, intersected with the loop allowlist
/// (the same 8 read-only tools as the loop worker — NOT widened).
pub fn loop_drafter_tool_definitions() -> Result<serde_json::Value, String> {
    let defs = crate::persona::subagent_tool_definitions("loop-drafter");
    crate::self_improve::worker::filter_loop_tools(&defs, "loop-drafter")
}

/// Ensure the loop-drafter persona file exists in the runtime persona
/// dir (`<rt>/agents/loop-drafter.md`). Seeds the shipped content when
/// missing; NEVER overwrites an existing file (John's edits win).
/// This is runtime-dir state seeding (like routines.json), not a
/// live-tree write — the runtime dir is not the repo checkout.
pub fn ensure_loop_drafter_persona(rt_dir: &Path) -> Result<PathBuf, String> {
    let path = rt_dir.join("agents").join("loop-drafter.md");
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("persona dir create failed: {e}"))?;
    }
    std::fs::write(&path, LOOP_DRAFTER_PERSONA).map_err(|e| format!("persona seed failed: {e}"))?;
    Ok(path)
}

/// The drafter brief for one proposal. The drafter returns whole-file
/// contents in `--- file: <path> ---` blocks and NOTHING else — no
/// introduction, no explanation, no closing note (trailing prose
/// would be written into the last file's content).
pub fn draft_prompt(proposal: &ProposalRecord) -> String {
    format!(
        "Helix self-improvement loop drafter (Phase 2, code drafting). \
         You are a read-only investigator: read the relevant repo files with your tools, then return the code. \
         You write no files yourself; the handler applies your draft to a disposable worktree and validates it.\n\n\
         Proposal:\n\
         - Problem: {}\n\
         - Trace: {}\n\
         - What would change: {}\n\
         - Proving test: {}\n\n\
         Return ONLY file blocks, one per file, in this exact format:\n\
         --- file: <relative path from repo root> ---\n\
         <the FULL new content of that file>\n\
         --- file: <next path> ---\n\
         <the FULL new content of that file>\n\n\
         Rules: whole files only (no unified diffs, no patches); paths are relative to the repo root, never absolute, \
         never containing `..`, never inside `.git`; keep the total draft under 500 lines; do not add secrets, \
         credentials, or tokens; do not touch sensitive paths (src/security.rs, shell tools, web server auth, \
         support-mail send paths). Return ONLY the file blocks.",
        proposal.problem_statement,
        proposal.trace,
        proposal.diff_description,
        proposal.proving_test,
    )
}

/// Validate a draft path with the same rules as the evaluator's
/// `resolve_diff_path`: relative, no `..` segments, no absolute path,
/// never inside a `.git` directory. Segments split on both `/` and
/// `\` (M27), so the Windows spellings are rejected too.
pub fn validate_draft_path(path: &str) -> Result<(), String> {
    if path.is_empty() || Path::new(path).is_absolute() {
        return Err(format!("rejected draft path: {path}"));
    }
    if path_segments(path).any(|seg| seg == "..") {
        return Err(format!("rejected draft path escaping repo: {path}"));
    }
    if path_segments(path).any(|seg| seg == ".git") {
        return Err(format!("rejected draft path inside .git: {path}"));
    }
    Ok(())
}

/// Parse a drafter response into whole-file `DiffFile`s. The response
/// must be ONLY `--- file: <path> ---` blocks: any text before the
/// first marker is an error (the drafter was told to return nothing
/// else), and every path is validated. A marker line is exactly
/// `--- file: <path> ---` (trailing whitespace tolerated).
pub fn parse_draft_response(text: &str) -> Result<Vec<DiffFile>, String> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(path) = line
            .trim()
            .strip_prefix("--- file: ")
            .and_then(|rest| rest.strip_suffix("---").map(|p| p.trim().to_string()))
        {
            if let Some(prev) = current_path.take() {
                validate_draft_path(&prev)?;
                files.push(DiffFile {
                    path: prev,
                    content: current_lines.join("\n"),
                });
                current_lines.clear();
            }
            if path.is_empty() {
                return Err("draft file marker has an empty path".to_string());
            }
            current_path = Some(path);
        } else if current_path.is_some() {
            current_lines.push(line.to_string());
        } else if !line.trim().is_empty() {
            return Err(format!(
                "draft response has text before the first file marker: {line}"
            ));
        }
    }
    if let Some(prev) = current_path.take() {
        validate_draft_path(&prev)?;
        files.push(DiffFile {
            path: prev,
            content: current_lines.join("\n"),
        });
    }
    if files.is_empty() {
        return Err("draft response contains no file blocks".to_string());
    }
    Ok(files)
}

/// Attach a parsed draft to a proposal record handler-side: enforce
/// the line cap, set `diff_files`, and upsert. Returns the file
/// count. The record must already exist (intaked); this never
/// creates a proposal.
pub fn attach_draft(
    rt_dir: &Path,
    record: &mut ProposalRecord,
    diff_files: Vec<DiffFile>,
    max_lines: usize,
    now_ms: u64,
) -> Result<usize, String> {
    if diff_files.is_empty() {
        return Err("draft produced no files".to_string());
    }
    let new_lines: usize = diff_files.iter().map(|f| f.content.lines().count()).sum();
    if new_lines > max_lines {
        return Err(format!(
            "draft of {new_lines} lines exceeds the {max_lines}-line cap (rejected pre-attach)"
        ));
    }
    record.diff_files = diff_files;
    record.updated_at = now_ms;
    upsert_proposal(rt_dir, record)?;
    Ok(record.diff_files.len())
}

/// Per-cycle drafting arm (task 362): draft the oldest "proposed"
/// record that still has no diff files, via the loop-drafter role.
/// Returns `Ok(Some(file count))` when a draft was attached,
/// `Ok(None)` when there is nothing draftable, and `Err` when the
/// draft failed (bad response, over-cap, dispatch failure — the
/// record stays diff-less and is retryable next cycle).
pub async fn draft_oldest_proposal(
    rt_dir: &Path,
    cfg: &crate::self_improve::SelfImproveConfig,
    now_ms: u64,
) -> Result<Option<usize>, String> {
    let mut proposals = crate::self_improve::worker::read_proposals(rt_dir)?;
    proposals.sort_by_key(|p| p.created_at);
    let Some(mut record) = proposals
        .into_iter()
        .find(|p| p.status == "proposed" && p.diff_files.is_empty())
    else {
        return Ok(None);
    };
    ensure_loop_drafter_persona(rt_dir)?;
    let prompt = draft_prompt(&record);
    let spec = crate::tools::subagent_tool::SpawnSpec {
        task: prompt,
        backend: cfg.draft_backend.clone(),
        role_hint: "loop-drafter".to_string(),
        timeout_secs: cfg.max_cycle_minutes.saturating_mul(60),
    };
    let report = crate::tools::subagent_tool::dispatch_spawn(&spec).await;
    let diff_files = parse_draft_response(&report)?;
    let count = attach_draft(rt_dir, &mut record, diff_files, cfg.max_diff_lines, now_ms)?;
    Ok(Some(count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::self_improve::worker::upsert_proposal;

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

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn draft_prompt_carries_proposal_and_format() {
        // GIVEN a proposal, WHEN the brief is built, THEN it carries
        // the problem/trace/description/test AND the file-block
        // format + the cap + the no-secrets rule.
        let p = proposal("errors:logs/error_log.md");
        let prompt = draft_prompt(&p);
        assert!(prompt.contains("errors:logs/error_log.md"));
        assert!(prompt.contains("--- file: <relative path from repo root> ---"));
        assert!(prompt.contains("500 lines"));
        assert!(prompt.contains("Return ONLY the file blocks"));
    }

    #[test]
    fn parse_single_file_block() {
        let text = "--- file: src/a.rs ---\nline one\nline two\n";
        let files = parse_draft_response(text).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].content, "line one\nline two");
    }

    #[test]
    fn parse_multiple_file_blocks() {
        let text = "--- file: src/a.rs ---\nnew a\n--- file: src/b.rs ---\nnew b\nline two\n";
        let files = parse_draft_response(text).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].content, "new a");
        assert_eq!(files[1].path, "src/b.rs");
        assert_eq!(files[1].content, "new b\nline two");
    }

    #[test]
    fn parse_rejects_leading_prose_and_empty_response() {
        // Text before the first marker is an error: it would otherwise
        // be silently dropped (or, worse, written into a file).
        assert!(parse_draft_response("Here is the fix:\n--- file: src/a.rs ---\nx\n").is_err());
        assert!(parse_draft_response("").is_err());
        assert!(parse_draft_response("just prose, no blocks\n").is_err());
        // An empty marker path is an error.
        assert!(parse_draft_response("--- file:  ---\nx\n").is_err());
    }

    #[test]
    fn parse_rejects_unsafe_paths() {
        // The same rules as the evaluator: no `..`, no absolute, no .git.
        assert!(parse_draft_response("--- file: ../evil.txt ---\nx\n").is_err());
        assert!(parse_draft_response("--- file: /tmp/evil.txt ---\nx\n").is_err());
        assert!(parse_draft_response("--- file: .git/config ---\nx\n").is_err());
        assert!(parse_draft_response("--- file: sub/.git/hooks/x ---\nx\n").is_err());
        // M27: backslash spellings are the Windows bypass — `\` is
        // a separator there, so `..\evil` and `.git\config` must
        // be rejected too.
        assert!(parse_draft_response("--- file: ..\\evil.txt ---\nx\n").is_err());
        assert!(parse_draft_response("--- file: .git\\config ---\nx\n").is_err());
        assert!(parse_draft_response("--- file: sub\\.git\\hooks\\x ---\nx\n").is_err());
        // A normal file and .gitignore are fine.
        assert!(parse_draft_response("--- file: src/a.rs ---\nx\n").is_ok());
        assert!(parse_draft_response("--- file: .gitignore ---\nx\n").is_ok());
    }

    #[test]
    fn attach_enforces_cap_and_persists() {
        // GIVEN a stored proposed record, WHEN a draft within the cap
        // is attached, THEN diff_files is set + persisted; WHEN a
        // draft over the cap is attached, THEN it is rejected and the
        // record is unchanged.
        let dir = tmpdir("helix-self-improve-draft-attach");
        let mut p = proposal("errors:logs/error_log.md");
        upsert_proposal(&dir, &p).unwrap();
        let small = vec![DiffFile {
            path: "src/a.rs".to_string(),
            content: "x\n".to_string(),
        }];
        let count = attach_draft(&dir, &mut p, small, 500, 2_000).unwrap();
        assert_eq!(count, 1);
        assert_eq!(p.diff_files.len(), 1);
        let stored = crate::self_improve::worker::read_proposals(&dir).unwrap();
        assert_eq!(stored[0].diff_files.len(), 1);
        // Over-cap: rejected, record unchanged.
        let big = (0..501)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let over = vec![DiffFile {
            path: "src/a.rs".to_string(),
            content: big,
        }];
        let r = attach_draft(&dir, &mut p, over, 500, 3_000);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("exceeds the 500-line cap"));
        let stored = crate::self_improve::worker::read_proposals(&dir).unwrap();
        assert_eq!(
            stored[0].diff_files.len(),
            1,
            "over-cap draft must not overwrite"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attach_rejects_empty_draft() {
        let dir = tmpdir("helix-self-improve-draft-empty");
        let mut p = proposal("errors:logs/error_log.md");
        let r = attach_draft(&dir, &mut p, Vec::new(), 500, 2_000);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("no files"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drafter_filter_fails_closed_and_excludes_spawn() {
        // GIVEN a definition list with only the 8 read-only tools,
        // WHEN filtered for the drafter, THEN all 8 survive; WHEN the
        // list is empty or carries spawn_subagent, THEN it fails
        // closed (never the full toolset) and spawn is never in the
        // result.
        let defs = serde_json::json!([
            {"function": {"name": "repo_read"}},
            {"function": {"name": "read_log"}},
            {"function": {"name": "spawn_subagent"}},
            {"function": {"name": "run_shell"}},
        ]);
        let filtered =
            crate::self_improve::worker::filter_loop_tools(&defs, "loop-drafter").unwrap();
        let names: Vec<&str> = filtered
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                d.get("function")
                    .unwrap()
                    .get("name")
                    .unwrap()
                    .as_str()
                    .unwrap()
            })
            .collect();
        assert_eq!(names, vec!["repo_read", "read_log"]);
        assert!(
            crate::self_improve::worker::filter_loop_tools(&serde_json::json!([]), "loop-drafter")
                .is_err()
        );
        assert!(
            crate::self_improve::worker::filter_loop_tools(
                &serde_json::json!([{"function": {"name": "spawn_subagent"}}]),
                "loop-drafter"
            )
            .is_err()
        );
    }

    #[test]
    fn drafter_persona_seeds_once_and_never_overwrites() {
        // GIVEN a fresh runtime dir, WHEN seeded, THEN the file holds
        // the shipped persona; WHEN seeded again after an edit, THEN
        // the edit wins (never overwritten).
        let dir = tmpdir("helix-self-improve-draft-persona");
        let path = ensure_loop_drafter_persona(&dir).unwrap();
        assert!(path.exists());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("loop drafter")
        );
        std::fs::write(&path, "john's edited drafter persona").unwrap();
        let path2 = ensure_loop_drafter_persona(&dir).unwrap();
        assert_eq!(path, path2);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "john's edited drafter persona"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
