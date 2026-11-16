# Helix Self-Skills Contract (tasks 361–366)

What a skill is, how the loop drafts one, and what must hold before
John merges it. This is the single doc that ties the self-skills
workstream together; the code comments in `src/skills/` and
`src/self_improve/` are the authoritative detail.

## 361 — What a skill is

A skill is a **named entry in a `SkillRegistry`**: a name maps to a
closure that takes JSON parameters and returns a `NodeResult`.
`SkillRegistry::call(name, params)` runs it; an unregistered name
returns `NodeResult::Error`. The registry is instantiated in
`main.rs` and handed to HyEvo as its `SkillInterface`.

To add a new skill:
1. Write the skill (struct + methods, or a plain function) in its own
   module under `src/skills/`.
2. Register a closure in `SkillRegistry::new()` — the closure is the
   skill's public surface; keep state behind an `Arc<Mutex<...>>`
   if the skill is stateful.
3. If an LLM should be able to call it, add a tool definition in
   `src/tools/mod.rs` (or the owning tool module) that maps the tool's
   params onto `SkillRegistry::call`.
4. Add unit tests in the skill's module AND a registry-level test
   (registered name + a representative call returns a non-error).
5. Document it here and, once it is exercised live, add an eval to
   the approved eval set.

Reconciled dead code (361):
- `BayesianSkill` (`src/skills/bayesian.rs`) is now wired as the
  `bayes` registry skill (closure maps the `command` string param
  onto `BayesianSkill::handle`). It was zero-usage before this.
- `SkillMarketplace` (`marketplace.rs`) is metadata-only (publish /
  discover / install of `SkillMetadata`); it loads no code, is never
  instantiated, and is NOT the skill system.
- `sandbox.rs` is a 5-line placeholder with no real sandbox (no WASM,
  no isolation). Do not claim a sandbox exists. A full sandbox is a
  prerequisite for any self-deployment — see 366.

## 362 — Loop drafting path

The worker returns prose only (`diff_description`); the evaluator
skips records with no diff files. The drafting step closes that gap:

1. The worker intakes a proposal from a signal.
2. The **loop-drafter** role (narrow, read-only, same 8 tools as the
   loop worker — `repo_read`, `repo_glob`, `repo_grep`, `read_log`,
   `read_note`, `list_notes`, `system_status`, `scan_directory`;
   the allowlist is NOT widened) reads the relevant files and returns
   whole-file contents in `--- file: <path> ---` blocks.
3. `attach_draft` (handler-side) enforces the 500-line cap and
   upserts the `DiffFile`s onto the record.
4. The evaluator runs the bounded subset + approved eval set in a
   disposable worktree; the audit runs; the PR payload goes through
   `phase3_gate`, target PreRelese only, John's merge tap.

Triggers, in order:
1. **An eval finding / ledger signal first** — a failing test name or
   metric key traceable under the R3 rule (the worker's `trace`).
2. **A user request** — John asks for a change; the proposal is
   intaked from his words and drafted the same way.
3. **A self-detected gap is the long-term trigger, not the first** —
   the loop does not yet go looking for gaps on its own.

The drafter writes nothing itself. Draft paths are validated with
the evaluator's rules (relative, no `..`, no absolute, never inside
`.git`) and re-validated at apply time.

## 363 — Phase advancement (whose step is whose)

The loop is stuck at phase 0 and has had no first real cycle. The
order, and who does each step:

1. **John's step, on the R630** — set `[self_improve]
   eval_set_approved = true` in the LIVE `/home/cobble/helix/config.toml`
   (never the web UI) and `sudo systemctl restart helix`. The eval
   set was approved Nov 10 (task 242); the flag is the missing piece —
   while false the evaluator never promotes (D5 gate). The
   `self-improve` routine must also be enabled for cycles to run.
2. **Automatic** — the first cycle runs at the next 03:30 daily
   cycle; the crew confirms it in the ledger. The one-week gate clock
   starts only from the first confirmed cycle.
3. **Gate 280** — one week gap-free ledger + John's review → John
   sets `phase = 1`.
4. **Gate 285** — two weeks nightly cycles, zero violations, one
   correct proposal → John sets `phase = 2`.
5. **Gate 289** — 5 evaluated proposals, cleanup verified, zero
   live-tree mods → John sets `phase = 3`.
6. **Gate 291** — `pr_opening` is a separate D1 runtime decision
   (John's explicit enable); 3 loop PRs on PR evidence alone.

Phase bumps are John's config edits only — the loop never writes
config. No code change was needed for this task: phase 0 already
collects the ledger, the D5 gate already blocks promotion while the
flag is false, and the gates are John's eyeball + elapsed time (per
`src/self_improve/mod.rs`).

Note on phase enforcement: the handler's phase gate is phase 0 vs
>= 1. Phase 2/3 distinctions are enforced by the D5 gate
(`eval_set_approved`) and `pr_opening`, not by phase-number checks
in the handler — the eval arm and the drafting arm run at any phase
>= 1, and the delivery arm runs only when `pr_opening` is true.

## 364 — Dedicated git-push

The loop pushes through ONE narrow op, `Git::push_branch` in
`src/self_improve/git.rs` — NOT the shell tool (the shell is
general-purpose with only a substring denylist and cannot enforce
branch/target rules). The op's exact shape:

```
push origin HEAD:refs/heads/self-improve/<slug>
```

- Push ONLY the evaluated/promoted branch, ONLY to `origin`.
- Branch name validated: must be `self-improve/<slug>` (safe chars,
  no `..`, non-empty slug). Never `--force`, never `--tags`, never
  another remote or refspec, never master or PreRelese as the push
  target (the PR target is PreRelese; the push target is always the
  feature branch).
- Uses the existing R630 SSH deploy key — no new secret, no
  credential helper, no HTTPS token.
- Gated by `phase3_gate` (promoted + eval green + clean audit +
  `eval_set_approved` + target PreRelese); while `pr_opening` is
  false, no push happens.

The evaluator preserves a promoted branch (the worktree dir is still
removed; a non-promoted branch is deleted) and records its name on
the proposal as `eval_branch`. The delivery path
(`deliver_promoted_proposal` in `pr.rs`, driven per-cycle by
`deliver_oldest_promoted_proposal`) attaches a temp worktree on the
preserved branch, pushes, removes the temp worktree, then queues the
PR payload. A failed push leaves the proposal `evaluated` and the
branch in place (retryable); the payload is only queued AFTER the
push succeeds. PR opening stays with the queue consumer (external —
Cobble jr via the GitHub connector today); the loop never calls
`merge_pull_request`.

## 365 — Context authoring

Same shape as skills. The loop drafts a new `OkfBundleManifest`
(id + version + knowledge entries with inline markdown or URLs +
tools + schemas), validates it with `OkfValidator`, and stores it in
`rt/self_improve/context_drafts.json` as "proposed" for John's tap.
It never pushes anything itself.

Delivery boundary — which path, stated plainly:
- A manifest pushed to the local librarian (`POST /okf/manifest`)
  is **in-memory only** and is overwritten by the next remote reload
  from the OKF server (.106) (auto_reload / webhook / poller all
  reload from the server).
- The **durable path is publishing the bundle to the OKF server**
  itself — that is John's step (or a server-side write), not the
  loop's. A draft delivered for his tap must say which path it takes.

An OKF tool entry is NOT a skill: `try_execute_okf_tool`
simulates (it returns metadata plus a "remote call simulated" line;
`TODO(real)` dispatch is unimplemented). An OKF tool entry describes
a remote tool; a skill is a `SkillRegistry` closure that runs.

## 366 — Review gate

The ordered minimum before John merges a self-authored skill:

1. Unit tests green (the bounded subset ran and passed).
2. Bounded subset green (the subset ran and the diff is within the
   500-line cap).
3. Approved eval set green (`cargo test --test evals`, D5 gate open).
4. Secrets / sensitive-path audit clean (task 288). Sensitive-path
   proposals are queue-only forever; John opens any PR himself.
5. John's read.
6. John's merge tap.

`review_gate_checklist` (`src/self_improve/review.rs`) computes the
machine checks and reports the human checks as `PendingHuman` — it
enforces nothing; enforcement stays in `phase3_gate`.

**A full sandbox IS a prerequisite for any self-deployment.** Until
one exists, every self-authored skill goes through John's merge tap.
No full sandbox is designed here — that is a separate follow-up
workstream, filed as task 368 in the task list (pending, high
priority). The loop never self-deploys by construction.

## 367 — Live drill (John's side)

After the path is built and the loop has advanced (deps 362, 364,
365, 366 + gate 285 passed), John runs the live drill. Not built
here; the task stays pending.
