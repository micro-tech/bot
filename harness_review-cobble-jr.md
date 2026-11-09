# Helix (bot) — Harness Review: the self-improvement loop (cobble-jr)

**Reviewer:** Cobble jr — independent pass
**Date:** 2026-11-09
**Scope:** `src/self_improve/` (4,207 LOC, 8 modules), the routines that drive it (`src/cpu/mod.rs` — `routine_self_improve`, `routine_error_log_review`, the chat-command loop, the support watcher, routine scheduling), and the loop's inputs (error log, restart signal, Quartermaster summary, ledger)
**Companion:** `code_review-cobble-jr.md` (whole-crate review)
**Base:** `origin/PreRelese` 7de0884

> "Harness" here = the Helix autonomous outer loop — the self-improvement loop (tasks 277–291) plus the routines that feed and drive it. That is the Helix analogue to grok-cli's HOH (Harness-of-Harnesses). This review assesses what it **actually does** versus what it **claims to do**, whether running it is safe, whether its evaluation is real, and whether its inputs are actually live.

> **Note:** the companion code review found the lib does not compile at this base (e4dec24's auth split broke the build — E0599, see `code_review-cobble-jr.md` §1). That does not affect the *running* service on the R630 (it runs an older, working binary), but it does mean no loop test can execute and no loop change can be verified until the build is restored. The findings below are from static reading, not from running the loop.

---

## Legend

| Symbol | Severity     |
|--------|--------------|
| 🔴     | Critical     |
| 🟠     | High         |
| 🟡     | Medium       |
| 🔵     | Low / polish |
| ✅     | Good         |

---

## 0. Headline Finding

**The self-improvement loop is a well-built engine that is currently idling in the driveway: it is armed at phase 0 (collect-only), its evaluation path is unwired, and two of its three trigger inputs are dead or unconfirmed — so it can observe, but it cannot yet improve anything, and it cannot yet prove it observed anything.**

Concretely:

- **Phase 0 (collect-only)** is the live setting. The loop writes a metrics entry per cycle and proposes nothing. That is the correct cautious setting — but it means the loop's entire value right now is a log line.
- **`evaluate_proposal` is never called from production code.** The evaluator exists (754 lines, well-built) and its tests call it, but no routine, handler, or cycle step invokes it. Proposals can be intaked; they can never be evaluated or promoted.
- **The restart input is dead.** `count_restarts` always returns 0 in production (the boot marker only goes to stdout). The loop's "restarts detected" trigger never fires.
- **The first-cycle ledger confirmation is still unanswered.** The loop's one-week gate clock is tied to the first cycle after arming, and that cycle has not been confirmed in the ledger — so gates 280/285/289/291 cannot pass.
- **The error-log input IS live** — the one input that works. `check_error_logs` reads both the runtime path and `~/.helix/logs/error_log.md`, so the loop sees the real error log.

So: the loop runs, collects, and reports — but it does not improve the project, and two of the three signals that would wake it up are silent. That is a defensible *safe* state, but it is not yet a self-improving loop. The documentation and the module comments are honest about the phase gates being John's eyeball + elapsed time, which is to the loop's credit — it does not claim to have passed gates it has not passed.

---

## 1. What the harness actually does (traced)

`routine_self_improve` (`src/cpu/mod.rs:538`) runs on the `self-improve` routine schedule (3:30 AM cycle). Each cycle:

1. **Kill switches first.** It checks the loop's kill conditions and the `routines.json` enabled flag. If a kill switch trips, it logs the abort and returns. This ordering is correct — the off-switch is checked before any work.
2. **Collect (counts only).** It gathers:
   - Error-log findings via `check_error_logs()` — live, reads both candidate paths.
   - The Quartermaster summary via `parse_qm_summary` — depends on the QM run having written its summary file.
   - Support stats via `support_watch::status_snapshot()`.
   - Restart count via `count_restarts` — **dead in production (always 0)**.
3. **Phase gate.** If `cfg.phase == 0` (the live setting), it writes the ledger entry, logs `"self_improve: phase 0 collect-only: no proposal"`, and returns. Nothing is proposed, no worker is dispatched.
4. **Worker dispatch (phase > 0 only).** If there is a failing signal (`error_total > 0`, or `restart_count > 0`), it seeds the loop-worker persona and dispatches a subagent with a worker brief, bounded by the cycle deadline (`max_cycle_minutes`). On timeout, it abandons cleanly with no partial proposal intaked.
5. **Intake (phase > 0 only).** The worker report is parsed, the proposal is budget-gated (`check_proposal_allowed` — 3/week, 1/signature, cooldowns), and intaked into the proposals file.
6. **Evaluation: missing.** There is **no step 6**. Nothing in the cycle calls `evaluate_proposal`. The evaluator's own doc comment says the per-cycle "evaluate the oldest proposed record" wiring is a follow-up handler arm and "no task in 277–291 owns it."

Because the loop is at phase 0, steps 4–6 do not run in production at all. The live behaviour is steps 1–3: check kill switches, collect counts, write a ledger entry, log one line.

---

## 2. Safety ✅ (the strong part)

The safety design is the best part of this harness, and it is enforced by construction rather than by convention:

- **The loop cannot merge, push, restart, write the live tree, or send mail** — because no code path in `src/self_improve/` performs any of those actions. The module doc says so, and the code bears it out: the git wrapper (`git.rs`) is an allowlist (not a denylist), the evaluator works only in a disposable worktree that is removed afterwards, and the PR module only builds a payload (PR opening is behind the `pr_opening` flag, off).
- **Budgets are hard caps.** 3 proposals/week, 1 per signature, 60-minute cycle timeout, 500-line diff cap. The 500-line cap rejects pre-eval (no worktree created, no test run), so a runaway diff costs nothing.
- **Kill switches are checked first**, before any work, and the abort is logged.
- **The evaluator rejects path escape.** Diff application is whole-file writes under the worktree root, rejecting `..` and absolute paths. `git apply` is deliberately excluded from the git allowlist.
- **D5 gate.** While `[self_improve] eval_set_approved` is false (the task-242 eval set is still DRAFT), every proposal is marked `promoted=false` with reason `eval-set-not-approved` — even on a green subset. Promotion requires subset + full eval set green AND the gate open.
- **Max-1-worktree.** A lock file admits one live evaluation; a second is refused. A 24h orphan sweep removes stale worktree dirs.

**The autonomy grant is real and scoped.** Helix has `[shell] enabled = true` plus a write SSH deploy key on the R630, so the loop *could* be dangerous — but the loop module itself does not use that grant. The grant lives in the general shell/tool layer, not in the loop. The loop's safety comes from what it does not do, not from what it is prevented from doing. That is the right shape, but it means the safety argument depends on the loop never growing a code path that uses the shell grant — which is a review discipline, not a compile-time guarantee. Worth a periodic re-check, not a redesign.

---

## 3. The evaluation is real but unwired 🟠

This is the central harness finding, and it has two layers.

**Layer 1 — the evaluator is never invoked (H1).**

- `evaluate_proposal` (`src/self_improve/evaluator.rs:307`) is called only from its own tests (lines 577, 616, 644, 666). A grep across `src/` finds no production caller.
- The evaluator's doc comment is explicit: the per-cycle "evaluate the oldest proposed record" wiring is a follow-up handler arm and "no task in 277–291 owns it."
- Consequence: even if the loop were moved past phase 0 tomorrow, proposals would be intaked and then sit. Nothing would evaluate them, nothing would promote them, and no PR would ever be opened. The loop's output pipeline ends at intake.

**Layer 2 — the eval set is still DRAFT (H2).**

- Even if the wiring existed, the D5 gate would mark every proposal `promoted=false` with reason `eval-set-not-approved`, because the task-242 eval set is still DRAFT and `[self_improve] eval_set_approved` is false.
- So there are two independent blockers on promotion: no wiring *and* no approved eval set. Fixing only one still leaves the loop unable to promote.

**The evaluation itself, when it runs, is real — not synthetic.** That is worth stating clearly, because it is the opposite of the grok-cli HOH problem (where the score was hardcoded arithmetic). Here the evaluator applies the diff in a disposable worktree, commits it there, runs the bounded test subset for the touched modules plus the approved eval set, and reports pass/fail. That is a genuine evaluation. The problem is not that the evaluation is fake — it is that it never runs and, if it did, the gate would refuse to promote.

**Fix order:** wire the per-cycle evaluation arm first (file the task — it has no owner), then get the task-242 eval set approved. Either alone is insufficient.

---

## 4. The inputs: one live, one dead, one unconfirmed 🟠

| Input | Status | Evidence |
|-------|--------|----------|
| Error log | ✅ Live | `check_error_logs` reads both the runtime path and `~/.helix/logs/error_log.md`, so it sees the real ~54k-lines/day log. |
| Restart count | 🔴 Dead | `count_restarts` scans `<rt>/logs/` for the boot marker, but `main.rs` only `println!`s it to stdout (journal) and the runtime logs dir is dead on the write side. Always 0 in production. |
| Quartermaster summary | 🟡 Depends | `parse_qm_summary` reads the QM summary file. If the nightly QM run wrote it, the input is live; if the QM run was skipped or failed, the loop logs a parse error and continues with `qm_summary: None`. The loop degrades gracefully, but the input is only as reliable as the QM run. |
| Ledger (first cycle) | 🟡 Unconfirmed | The first-cycle ledger confirmation after arming is still unanswered. The one-week gate clock is tied to the first cycle, not to the merge — so the gates cannot pass until a first cycle is confirmed in the ledger. |

The restart input is the one to fix (one line of startup code — write the boot marker to a file under the runtime logs dir, or point `count_restarts` at the journal). The ledger confirmation is John's eyeball, not a code fix.

**Signal quality note (🟡):** the error log is ~54k lines/day, mostly routine chatter (the task 312 fix excluded the loop's own `routine_run` / `chat-command:` prefixes from the error *count*, but the lines are still in the file). The loop's worker brief is built from the worst file by error count. If the worst file is dominated by chatter rather than genuine failures, the worker gets a noisy brief. The `is_counted_error_line` filter exists and the 312 prefixes help, but the signal-to-noise of a 54k-line/day log is the loop's real input-quality risk — more so than any single missing input.

---

## 5. The routines that drive it

The loop does not run in isolation — it is driven by the routine system, and the routines have their own state.

- **Chat-command loop (`chat-command-watch`, every 1m).** Works. The `to` addressing and MCP rejection drills passed live. The queue is in-memory (a restart drops queued/running tasks) and the LLM path depends on the `.149` Ollama backend, which has been the weak link (timeouts on the 311 drills). The loop's own health does not depend on the chat-command loop, but the chat-command loop is the mechanism by which a human (or Cobble jr) would poke the loop — and it is the routine most likely to be the loop's first real user.
- **Support watcher.** Live, with `auto_send = true` in the live config (default false in code). The watcher is the loop's most active routine (30 min night / 5 min business / 1-min burst). The task 312 fix stopped the nightly spam-sends, and the 312.2 check passed. The watcher feeds `support_stats` into the loop's cycle inputs via `status_snapshot()`.
- **Routine scheduling / heartbeat.** The Cpu mutex issue (code review §4, task 315) is the harness-relevant one: a long routine holds the Cpu lock for its whole run, and the heartbeat task needs the same lock. If a routine stalls, the heartbeat stalls, and the loop's liveness signal goes quiet. The loop cannot distinguish "routine running long" from "service dead" while the heartbeat is starved.
- **The routine toggle bug (task 292, unfiled).** The web UI displayed routines as on while `routines.json` said off. The file is ground truth; the UI is not. For the loop, this matters because the loop reads `routines.json` to decide whether it is enabled — so the loop's own kill-switch check is on the correct source. But an operator looking at the UI could believe the loop is off when it is on (or vice versa). The toggle bug is a UI-truth problem, and the loop happens to read the right file.

---

## 6. What the loop claims vs what it does

| Claim (docs / module comments) | Reality (traced) |
|---|---|
| "Self-improving loop" | Phase 0 collect-only. It collects and logs. It does not improve anything yet. |
| "Proposes fixes as task-list-shaped records with test evidence" | True in the code path (phase > 0), but the phase gate means it never runs in production. |
| "The loop NEVER merges / pushes / restarts / writes live tree / sends mail" | True, by construction. The strongest claim, and it holds. |
| "Evaluates proposals in a disposable worktree" | True of the evaluator component, but it is never invoked from production. The wiring is missing. |
| "Phase gates are John's eyeball + elapsed time" | Honest. The gates are not code and the module never claims to pass them. |
| "Budgets: 3/week, 1/signature, 60 min, 500 lines" | True, enforced in `budgets.rs` and the evaluator. But untested end-to-end (no cycle has ever run past phase 0). |

The loop is honest about its gates and its safety, which is more than can be said for most autonomous-loop marketing. Its gap is not overclaiming — it is that the *wiring* between its well-built components is incomplete (no evaluation arm, no approved eval set, no confirmed first cycle, one dead input).

---

## 7. Recommendations (in order)

1. **🟠 File and wire the per-cycle evaluation arm.** Give the "evaluate the oldest proposed record" step an owner and a task. Without it, the evaluator is dead code in production. This is the single highest-leverage harness fix.
2. **🟠 Fix the restart signal.** One line of startup code (write the boot marker to a file under the runtime logs dir) makes `count_restarts` live and gives the loop its second trigger.
3. **🟠 Confirm the first cycle in the ledger.** John's eyeball — read the ledger after the next 3:30 AM cycle and confirm the entry exists. That starts the one-week gate clock. Until then, gates 280/285/289/291 cannot pass.
4. **🟡 Approve (or explicitly defer) the task-242 eval set.** Until it is approved, the D5 gate refuses every promotion even if the wiring exists. If John does not want to approve it yet, that is a valid choice — but then the loop's promotion path is knowingly closed, and that should be recorded rather than discovered later.
5. **🟡 Improve the error-log signal-to-noise.** The 54k-lines/day log is the loop's main input. The 312 prefix exclusions helped the count; consider whether the *brief* should be built from a filtered view (genuine recent errors only) rather than the worst file by raw count.
6. **🔵 Re-check the autonomy grant periodically.** The loop does not use the `[shell]` grant, but the grant exists on the same box. A periodic review (e.g. in the Quartermaster run) that the loop module still contains no shell/git-push/restart code path would catch drift.

---

## 8. Findings index

| # | Severity | Finding | Location |
|---|----------|---------|----------|
| H0 | 🔴 Critical | The lib does not compile (e4dec24 auth split) — no loop test can execute and no loop change can be verified until the build is restored | `src/io/web_server/mod.rs:505` (see code review §1) |
| H1 | 🟠 High | `evaluate_proposal` never called from production code — evaluation wiring missing, no owning task | `src/self_improve/evaluator.rs:307`, `src/cpu/mod.rs:538` |
| H2 | 🟠 High | Restart input dead in production (`count_restarts` always 0) | `src/self_improve/metrics.rs:223`, `src/main.rs:139` |
| H3 | 🟠 High | Phase 0 collect-only + first-cycle ledger unconfirmed — loop observes but cannot improve, gates cannot pass | `src/cpu/mod.rs:538`, ledger |
| H4 | 🟡 Medium | Eval set (task 242) still DRAFT — D5 gate refuses every promotion even if wiring existed | `src/self_improve/evaluator.rs` (D5 gate) |
| H5 | 🟡 Medium | Error-log signal-to-noise: 54k lines/day, brief built from worst file by raw count | `src/cpu/mod.rs:538`, `src/self_improve/worker.rs:111` |
| H6 | 🟡 Medium | Cpu mutex starves the heartbeat during long routines (loop's liveness signal goes quiet) | `src/main.rs:403-404,420-421` |
| L1 | 🔵 Low | Autonomy grant exists on the box but the loop does not use it — needs periodic re-check, not a redesign | `src/self_improve/mod.rs` |

**Verdict: 🔴 Fix the build break first, then 🔧 wire the evaluation before trusting the loop to improve anything.** The safety is real and the evaluator is genuine. But until the build is restored, the evaluation arm is wired, the eval set is approved, and the first cycle is confirmed in the ledger, the loop is an observer — a well-built one — not yet a self-improving system.
