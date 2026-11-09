# Harness Review: helix (Claude)

**Date:** 2026-11-09
**Reviewer:** Claude Opus 5.5 (Zed Agent)
**Branch / commit:** `PreRelese` @ `2d2557d`
**Scope:** everything used to test, evaluate and drive the agent:
- `tests/` (including `scenarios/` and `evals.rs`) and `evals/`
- in-source `#[cfg(test)]` modules
- `__tests__/` and `jest.config.js`
- `scripts/`, `push.sh`, `build.rs`, `.cargo/config.toml`
- `.zed/task_list.json`, and CI (there is none)

Companion document: `code_review-claude.md`.

---

## Executive Summary

The project has a lot of unit-test coverage (about 692 tests in `src/`), and some of the test engineering is thoughtful. Examples: the shared `test_env_lock()`, port-0 binding in the OKF test, and a real OpenSSH interop test. **The harness around those tests is not trustworthy yet:**

1. **Zero tests run on Windows today.** The lib test target fails to compile (Unix-only `PermissionsExt` in `src/chat_command/mod.rs:1218`), and the lib itself fails to compile (`ct_eq`, `src/io/web_server/mod.rs:506`).
2. **There is no CI**, so the broken build went unnoticed through a commit marked done.
3. **`.zed/task_list.json` contains unresolved merge-conflict markers** and is not valid JSON, so the task runner workflow in AGENTS.md cannot load it.
4. **Several evals and integration tests are placeholders** that cannot fail.
5. **One eval resets the live spend ledger** when run in the deploy directory.

**Harness score: 4.5/10.** The building blocks are good, but nothing currently enforces or runs them.

---

## Numbers

| Metric | Value |
|---|---|
| `#[test]` / `#[tokio::test]` in `src/` | 692 |
| Same, in `tests/` | 36 |
| Files in `src/` with `#[cfg(test)]` | 98 |
| `#[ignore]` tests | 4 (2 undocumented) |
| `env::set_var` / `remove_var` call sites in tests | about 41 |
| Tests runnable on Windows right now | **0** (build fails) |
| CI pipelines | **0** (no `.github/`) |
| Tasks in `task_list.json` (HEAD side of conflict) | 134: 98 done, 31 pending, 4 in progress, 1 deferred |
| Tasks with all 3 mandatory AGENTS.md subtasks | **0 / 134** |
| Tasks with no subtasks at all | 43 |
| `done` tasks with open subtasks | 7 (IDs 164, 165, 238, 255, 261, 308, 312) |
| Dangling dependencies | `154 → 153` (153 doesn't exist) |

The task counts were computed by resolving the merge conflict in memory only. Nothing was written to disk.

---

## Findings

### Critical

#### HC1. Test build broken on Windows
- `src/chat_command/mod.rs:1218` and `:1226`: `stub_ssh()` uses `std::os::unix::fs::PermissionsExt` / `Permissions::from_mode(0o755)` and writes a `#!/bin/sh` script. The enclosing `mod tests` is not `cfg(unix)`.
- On top of that, the lib doesn't build because `ct_eq` is not in scope (`src/io/web_server/mod.rs:506`).
- **Fix:** put `#[cfg(unix)]` on `stub_ssh` and on the `tick_asus_*` tests that use it, or add a `.cmd` stub under `cfg(windows)`. Fix the auth-helper duplication (see the code review, C1).

#### HC2. No CI
- There is no `.github/workflows/` or any other pipeline. Comments such as "skipped in CI" in `src/io/ollama/mod.rs` refer to CI that doesn't exist.
- **Fix:** add a GitHub Actions matrix (`windows-latest` and `ubuntu-latest`) that runs:
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test --all-targets`
  - a JSON-validity check on `.zed/task_list.json` (only if that file is committed; your personal `.gitignore` excludes `.zed/`)

#### HC3. `task_list.json` has merge-conflict markers
- `.zed/task_list.json:3797` (`<<<<<<< HEAD`), `:3907` (`=======`), `:3933` (`>>>>>>> 4a3c1b5`).
- The file is invalid JSON, so the task runner from AGENTS.md (load → filter pending → sort by priority) fails at step 1. The two sides differ in the status of about 2 tasks.
- **Fix:** resolve the conflict by hand, then add a validation step (see HM7).

#### HC4. Eval 8 resets the live spend ledger
- `tests/evals.rs:128` and `:141` call `spend::reset_for_tests()` without setting `HELIX_SPEND_FILE`. `save_state` then writes `./spend_state.json` in the crate root (`src/router/spend.rs:176-181`, `:213-217`, `:333-337`).
- That is the real agent's spend file (see `scripts/backup-state.sh`). Running evals in the deploy directory zeroes today's spend, which quietly resets the cap.
- `spend_state.json` is also not in `.gitignore`.
- **Fix:** set `HELIX_SPEND_FILE` to a temp path in the eval (under the shared lock), and make `state_path()` refuse the default path under `cfg(test)`. Add the file to `.gitignore`.

### High

#### HH1. Placeholder tests and evals that cannot fail
| Location | Problem |
|---|---|
| `tests/evals.rs:147-160` (Eval 9) | Builds a `json!` literal and asserts its own fields. |
| `tests/evals.rs:165-172` (Eval 10) | Empty body, only comments. |
| `tests/evals.rs:94-100` (Eval 6) | Passes vacuously when no persona is configured. |
| `tests/reflection_planner_adapter_test.rs:10` | `assert!(true);` |
| `tests/unix_socket_test.rs:9-23` | Placeholder on both Unix and Windows (`assert!(true)` at `:22`). |
| `tests/scenarios/llm_tests.rs`, `cpu_tests.rs` | Only check counters the harness itself increments. |
| `tests/integration_harness.rs:69` | TODO where CPU/HyEvo startup should happen. |

These inflate the pass count and give false confidence. **Task 242 ("Eval harness") is marked `done`**, but `evals/README.md:3` still says DRAFT.
- **Fix:** make Eval 9 drive the real bridge, and make Eval 10 build the real router and hit an ops route without a token (`tower::ServiceExt::oneshot`). Otherwise, delete them and reference the unit tests. Set task 242 back to `in_progress`.

#### HH2. `HELIX_SPEND_FILE` guarded by two different mutexes
- `src/router/spend.rs:352` has its own `TEST_LOCK`, while `src/io/llm_gemini/mod.rs:700-767` and `src/tools/subagent_tool.rs:762-805` use `subagent_tool::test_env_lock()`.
- Parallel tests can race on both the env var and the global `SPEND` tracker.
- **Fix:** one lock for all env-mutating tests, or add the `serial_test` crate with a named key.

#### HH3. Unlocked `unsafe remove_var` in Gemini tests
- `src/io/llm_gemini/mod.rs:590`, `:601`: the SAFETY comment claims "single-threaded test", but cargo runs tests on parallel threads. Under edition 2024 this is unsound alongside any other env reader.
- **Fix:** take `test_env_lock()`.

#### HH4. Network resilience is not tested
AGENTS.md requires timeout and retry handling for Starlink drops. No test simulates:
- a connect timeout
- a mid-stream disconnect
- 429/503 with `Retry-After`
- retry exhaustion

Given the production gaps (code review H3-H5), this is the most valuable test area to add.
- **Fix:** `mockito` (already a dev-dependency) with delayed responses, plus a TCP listener that accepts and then drops the connection. Assert the backoff schedule, the total deadline and the no-replay-of-tools property (code review H4).

#### HH5. `ssh_integration.rs` needs external tools and fixed ports but isn't ignored
- `tests/ssh_integration.rs:32`: `.expect` panics if `ssh-keygen` is missing.
- Ports 2229 and 2230 are hard-coded (`:45`, `:172`).
- **Fix:** bind port 0, skip gracefully when OpenSSH isn't on `PATH`, or `#[ignore = "needs OpenSSH client"]`.

#### HH6. Shell scripts have CRLF line endings and there is no `.gitattributes`
- `push.sh` and `scripts/backup-state.sh` have CRLF line endings on disk (confirmed with `file`). If either is copied (not cloned) to the Linux host, `set -euo pipefail\r` fails and the nightly backup dies.
- **Fix:** add a `.gitattributes` containing `*.sh text eol=lf`. This is the one justified exception to the CRLF rule in AGENTS.md. Consider `* text=auto` and `*.ps1 text eol=crlf` as well.

### Medium

- **HM1. Unix assumptions in tests.**
  - `src/tools/shell_tool.rs:379`, `:423-494`: live tests spawn `/bin/sh`, and `/etc` is not absolute on Windows.
  - `src/tools/file_tools.rs:514`: expects an "escapes" error for `/etc/passwd`, but Windows returns "Error reading".
  - `src/chat_mcp_install.rs:327`: binary name without `.exe`.
  - **Fix:** `cfg(unix)` gates, or Windows-specific expectations.
- **HM2. Hard-coded `/tmp` paths:** `src/router/spend.rs:362`, `src/persona.rs:734`, `:747`, `src/io/unix_cli/resume_transfer.rs:43`. On Windows these resolve to `I:\tmp\...`. **Fix:** `std::env::temp_dir()` or add `tempfile` as a dev-dependency.
- **HM3. Sleep-based synchronisation:** `tests/scenarios/bus_tests.rs:26`, `:48` and `tests/integration_harness.rs:94` sleep 30-50 ms and then assert. `a2a_handler.rs:463` polls for up to 10 s. Ollama tests assume port 19999 is closed (`src/io/ollama/mod.rs:1032-1081`). **Fix:** wait on channels with `recv_timeout`, and use a port the test binds and then drops.
- **HM4. Undocumented `#[ignore]`:** `src/io/ollama/llm_impl.rs:169`, `:179` need a live Ollama. **Fix:** `#[ignore = "needs Ollama at localhost:11434"]`.
- **HM5. No coverage of the WS `chat` auth path or `Origin` rejection.** The Task 313 tests cover CRUD and `slash_cmd`, but not `chat`, which is the unauthenticated arm (code review H1). Add negative tests once that is fixed.
- **HM6. Self-improve safety lacks adversarial tests.** No test feeds `resolve_diff_path` with `.git`, `..\..\x`, or `\Windows\x`, or feeds the audit with `./src/security.rs` or `src\cpu\mod.rs` (code review C3, H11). These are cheap, high-value table tests.
- **HM7. No task-list validator.** AGENTS.md asks for periodic validation (invalid or circular dependencies, missing fields, consistent status). Nothing does this, which is why HC3, the `154 → 153` dangling dependency, and the 0/134 mandatory-subtask compliance went unnoticed. **Fix:** a small `xtask`, `scripts/validate_tasks.ps1`, or a `#[test]` that parses the file and checks:
  - valid JSON
  - unique IDs
  - every dependency exists, and there are no cycles
  - status values are within the allowed set
  - every task has the 3 mandatory subtasks
  - every `done` task has no open subtasks
- **HM8. Tasks marked `done` with open subtasks** (164, 165, 238, 255, 261, 308, 312), and 66 of 98 `done` tasks have no test-related subtask at all.

### Low

- **HL1. Orphaned Jest setup:** `jest.config.js` and `__tests__/commands.test.ts` target `src/commands/*.ts`, a TypeScript Discord-bot stub that is unrelated to the Rust crate. There is no `package.json`, so `ts-jest` can't resolve. Delete it or move it into its own package.
- **HL2. Duplicate scripts:** `scripts/build_markmaps.ps1` and `scripts/build-markmaps.ps1` have drifted apart. They and `build.rs:28` reference `.grok/diagrams`, which is gitignored, so the freshness check is a no-op on a clean checkout.
- **HL3. `build.rs:17`** has `rerun-if-changed` on `config.toml`, the secrets file, so every config edit forces a rebuild. Confirm whether this is needed. Clippy also flags 2 collapsible `if`s in `build.rs`.
- **HL4. `.cargo/config.toml`** only sets env vars. Its comment admits `LIBSSH2_SYS_USE_PKG_CONFIG` is usually ineffective on Windows.
- **HL5. Untracked review artefacts** in the repo root (`code_review_grok-4.2.md`, `harness_review_grok-4.2.md`, a deleted `code-grok-cli-review.md`). Consider moving reviews to `.doc/` or `doc/AiSummary/`, as AGENTS.md suggests.

---

## Comparison with the earlier Grok 4.2 harness review

| Grok claim | Status |
|---|---|
| Unix-only test breaks Windows | **Confirmed** (HC1) |
| Missing mandatory subtasks | **Confirmed, and worse:** 0/134 compliant |
| `tests/evals.rs` incomplete or missing | **Partly wrong.** It exists (172 lines). Evals 1-5 and 7 are real and deterministic. Evals 6, 9 and 10 are vacuous. |
| No network-drop tests | **Confirmed** (HH4) |
| — | **New here:** no CI, merge-conflict markers in the task list, the eval resets the live spend ledger, the two-mutex env race, CRLF shell scripts, placeholder `assert!(true)` tests |

---

## Strengths

- **Shared env lock:** `subagent_tool::test_env_lock()` recovers from poisoning and is used across about 10 modules.
- **Real interop test:** `ssh_integration.rs` drives a real OpenSSH client and checks policy blocks and fail-closed behaviour. Its single-function design (to avoid `OnceLock` races) is documented at `:90`.
- **Port hygiene in OKF:** `okf_v1_self_fetch.rs` binds `127.0.0.1:0` and uses a per-process temp trace file.
- **Deadlock-aware harness:** `integration_harness.rs:50` uses `spawn_blocking` for the blocking `mpsc` receive, with a clear comment.
- **Deterministic evals:** Evals 1-5 and 7 need no LLM and pin real security and spend behaviour.
- **`push.sh` is defensive:** it restores the config with a `trap` and refuses to push if secrets are in unpushed commits.
- **Thoughtful eval philosophy** in `evals/README.md`: it separates behavioural evals for a self-modifying agent from unit tests.

---

## Recommended Fix Order

1. **HC1:** make `cargo test` compile on Windows.
2. **HC3:** resolve the `task_list.json` conflict.
3. **HC2:** add CI (Windows + Linux, clippy `-D warnings`) so 1 and 2 can't regress.
4. **HC4, HH2, HH3:** stop tests from touching live state, and unify env locking.
5. **HH1:** make the placeholder evals real, or remove them, and reopen task 242.
6. **HH4, HM5, HM6:** add Starlink-drop, WS-auth and self-improve adversarial tests.
7. **HM7:** task-list validator, then backfill the mandatory subtasks.
8. **HH5, HH6** and the remaining Medium/Low items.

---

*Generated by Claude Opus 5.5 (Zed Agent). Source of suggestions: AI. I checked the build failure, missing CI, merge markers, placeholder tests, test/ignore counts, CRLF scripts, the missing `.gitattributes` and `package.json`, and the eval spend reset directly. Task-list statistics come from an in-memory parse of the HEAD side of the conflict.*
