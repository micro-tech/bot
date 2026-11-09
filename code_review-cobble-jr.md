# Helix (bot) — Code Review (cobble-jr)

**Reviewer:** Cobble jr — independent pass
**Date:** 2026-11-09
**Scope:** `src/` (303 files, ~49.8k LOC), `tests/` (18 files), `Cargo.toml` / `Cargo.lock`, build/clippy
**Base:** `origin/PreRelese` 7de0884 ("Add Claude and Grok code and harness reviews")
**Method:** Static reading of the security-critical and high-traffic modules (web server WS auth, chat-command MCP handler, self-improvement loop, CPU/bus hot paths) + `cargo clippy --lib` + targeted tests. This is a fresh, independent review. It does not reuse the conclusions in the existing review files; where my findings agree or disagree with them, I say so.

> **Context:** PreRelese just advanced 4a3c1b5 → e4dec24 → 2d2557d → 7de0884. The new commits carry a web_server auth module split, a task 313 auth-gate update, 37 dep updates, the MCP handler (task 319), a WS auth extract, and the Claude + Grok review docs. This review is of the current PreRelese state including those changes.

---

## Legend

| Symbol | Severity                      |
| ------ | ----------------------------- |
| 🔴     | Critical — fix before release |
| 🟠     | High — address soon           |
| 🟡     | Medium                        |
| 🔵     | Low / polish                  |
| ✅     | Good / strong area            |

---

## 0. Executive Summary

**🔴 The library does not compile at PreRelese 7de0884.** The auth-module split in e4dec24 removed the `use subtle::ConstantTimeEq;` import from `src/io/web_server/mod.rs` but left duplicate auth functions in that file that still call `.ct_eq()` — so `cargo check --lib`, `cargo clippy --lib`, and `cargo test --lib` all fail with E0599. This is a hard build break on PreRelese, introduced by the latest commits. It is the first thing to fix, before any of the findings below.

Beyond the build break, the security work from the last two weeks **holds**. The task 313 WS auth gate is real and tested, the auth module extraction is clean, the MCP handler is fail-closed and well-documented, and the 37 dep updates landed without breaking the build. The web server's production code is `unwrap()`-free, which is rare and good for a network-facing module.

The live problems are not in the new code — they are in the **unfinished business underneath it** and in **one file the last merge broke**:

1. 🔴 **The lib does not compile** (e4dec24 auth split — see §1).
2. 🟠 `.zed/task_list.json` — the crew's source of truth — has **unresolved git merge conflict markers** committed on PreRelese. It is invalid JSON and unparseable.
3. 🟠 Tasks **314, 315, 316 are all still pending** (unbuilt): dead log cleanup, the Cpu mutex held across whole routines, and hot-path file I/O on every log line and bus message.
4. 🟠 The **restart signal is dead in production**, so the self-improvement loop's "restarts detected" trigger never fires.
5. 🟠 The **retired `gemini-2.0-flash` is still the default model** (`src/main.rs:656`).

**Top 4 things to fix:**

1. 🔴 **Fix the build break** — delete the duplicate auth functions from `mod.rs` and rewire callers to `auth::…` (completing the intended split), or re-add the `subtle` import as a band-aid. See §1.
2. 🟠 Resolve the merge conflict in `.zed/task_list.json` and re-validate it parses (it is the task system's input).
3. 🟠 Build 314 + 316 → 315 (the Nov 7 review's recommended order — log cleanup and hot-path I/O first, then the mutex).
4. 🟠 Fix the restart signal (write the boot marker to a file under the runtime logs dir, or point `count_restarts` at the journal) so the loop's restart trigger is live.

---

## 1. Build & Tooling Health — 🔴 THE LIB DOES NOT COMPILE

- `cargo check --lib` → **FAILS** — `error[E0599]: no method named 'ct_eq' found for reference '&[u8]'` at `src/io/web_server/mod.rs:506`.
- `cargo clippy --lib` → **FAILS** — same E0599 (it is a rustc error, not a lint).
- `cargo test --lib` → **cannot run** — the lib does not compile, so no test binary is produced.
- `cargo check --bin helix` → [see Verification; the bin depends on the lib, so it is blocked by the same error].

**Root cause — the e4dec24 auth-module split is half-done:**

- e4dec24 ("Update task_list.json and split web_server auth module") removed `use subtle::ConstantTimeEq;` from `src/io/web_server/mod.rs` and added `mod auth;`.
- But it did **not** remove the duplicate auth functions still sitting in `mod.rs`:
  - `constant_time_token_eq` at line 505 (calls `.ct_eq()`)
  - `http_token_ok` at line 512
  - `require_web_token` at line 525
  - `require_okf_write_token` at line 549
- And it did **not** rewire any caller to the new module — there is not a single `auth::` reference in `mod.rs`. The `auth.rs` module is declared but nothing uses it.
- At 4a3c1b5 the import was present (line 29), so the duplicates compiled. Removing the import while leaving the duplicates broke the build.
- The WS arms call `constant_time_token_eq` (line 1071), which resolves to the mod.rs duplicate — the one that does not compile.

**Fix (do the intended split, not a band-aid):** delete the four duplicate functions from `mod.rs` and rewire their callers to `auth::constant_time_token_eq`, `auth::http_token_ok`, `auth::require_web_token`, and `auth::require_okf_write_token`. The band-aid alternative — re-adding `use subtle::ConstantTimeEq;` to `mod.rs` — restores the build but leaves the duplication that the split was meant to remove. Either way, verify with `cargo check --lib` + `cargo test --lib` before merging.

**Other build notes (from the clippy run, before it hit the error):**

- `build.rs:43,69` — two `collapsible_if` warnings in the build script.
- `src/router/config.rs:182` — empty line after a doc comment.
- `src/io/web_server/auth.rs:96` — unused variable `v` in `redacted_config_str` (the `if let Some((k, v))` binding — `v` is unused because the redacted branch does not use it).
- Edition 2024, `rust-version = "1.99.0"` in `Cargo.toml`. Current stable Rust is 1.99.1 (1.100.0 lands Nov 12, 2026) — the pin is one patch behind, not a problem.
- 726 `#[test]` / `#[tokio::test]` attributes across `src/` + `tests/` (counted by grep; the suite itself could not run because the lib does not compile).
- **Known caveat (not a regression):** a `cargo check --workspace` failure on the installer bin is the pre-existing task 262 `include_str!` issue (`src/bin/installer.rs:273-274` bakes in `../../config.toml` and `../../.grok/docs/system_manifest_root.md`, neither tracked). A fresh-checkout installer build fails on those two lines; that is task 262, still pending.

---

## 2. The task list is broken (new) 🟠

**`.zed/task_list.json` contains unresolved merge conflict markers, committed on PreRelese.**

- `<<<<<<< HEAD` at line 3797, `=======` at line 3907, `>>>>>>> 4a3c1b58be48d8c335e48e81f30c6506a672187c` at line 3933.
- The file fails to parse as JSON (`json.decoder.JSONDecodeError: Expecting property name enclosed in double quotes: line 3797`).
- The conflict sits inside task 317's entry, between its `"results"` field and task 318. It came in with the 2d2557d merge (e4dec24 ↔ 4a3c1b5).

This is the file the crew's task system reads. Any tooling that parses it — the task runner, a Ranger dispatch, a status audit — breaks on it. It also means the task statuses below were read by grep, not by a parser, and the file cannot be trusted as a single source of truth until the conflict is resolved and the file re-validated.

**Fix:** resolve the conflict (keep the intended 317 results + the 318 entry), then validate with a JSON parse before committing. Add a CI check or a pre-commit hook that parses `.zed/task_list.json` so a conflicted task list can never land again.

---

## 3. Security ✅ (the 313 gate holds)

The task 313 WS auth gate is **real and verified by reading the code**:

- All **seven mutating WS arms are gated** with the same check — `if !state.auth_token.is_empty() && !authed { reject }` — in `src/io/web_server/mod.rs`:
  - `routine_toggle`, `routine_create`, `routine_update`, `routine_delete`
  - `config_save`, `manifest_save`
  - `slash_cmd` (the sharp one — its dispatcher can send support email, read customer drafts, and apply personas)
- `chat` is deliberately **ungated** — a product decision by John, not an oversight. The comment says so explicitly.
- The gate is enforced per-connection (`authed` flag set only by a successful `{"type":"auth","token":"..."}` message, compared with constant-time equality). Rejection replies go on the per-connection channel, never broadcast.
- There is a real test driving the actual `/ws` endpoint enumerating the mutating arms (`ws_mutating_arms_require_auth`, `src/io/web_server/mod.rs:3785`).

**The auth module extraction is clean** (`src/io/web_server/auth.rs`, 150 lines, extracted from the monolith):

- `constant_time_token_eq` uses `subtle::ConstantTimeEq`.
- `http_token_ok` accepts `Authorization: Bearer <token>` or `X-Web-Token: <token>`.
- `require_web_token` middleware returns 401 when a token is set and the request lacks it.
- Redaction (`redacted_config_str` / `is_secret_key`) preserves keys and redacts values, with tests.
- 🟡 One wart: `require_okf_write_token` is a **copy-paste duplicate** of `require_web_token` (identical bodies). The comment says "can be separate token in future" — right now it is dead duplication. Extract a shared helper or delete one before they drift.

**Remaining security notes:**

- 🟡 **Fail-open web auth is still the design** (empty token = endpoints open, with a loud startup warning). The live token is the example placeholder string — rotation is still pending, as is the Google OAuth client secret rotation. Both are John's call, not a code defect.
- 🔵 `constant_time_token_eq` returns early on length mismatch, leaking the token length. Standard practice and very low risk, but the length is the one thing it leaks.

---

## 4. The 314 / 315 / 316 trio is still open 🟠

All three are `pending` in the task list, and the code confirms none of them is fixed.

### 314 — Log cleanup is dead code

- `nightly_maintenance` exists at `src/cpu/mod.rs:1146` but has **zero callers** (grep across `src/` finds only the definition). Log cleanup never runs.
- The error log grows without bound except for the rotation helper `rotate_error_log_if_needed` (`src/utils.rs:80`), which is itself only called by the nightly error-log review — and the review's cadence depends on the routine being enabled and firing.
- At the observed ~54k chatter lines/day, the error log is a steady disk-growth line with no automatic bound.

### 315 — Cpu mutex held across whole routines

- `src/main.rs:403-404` — each routine spawn takes `cpu_clone.lock().await` and holds the guard for the entire `handle_routine_run` call.
- `src/main.rs:420-421` — the heartbeat task takes the same `cpu.lock().await` to run `handle_heartbeat`.
- A long routine (a slow LLM call, a hung MCP call, a big error-log review) starves the heartbeat for its whole duration. The heartbeat is the liveness signal; starving it is how a stuck routine looks like a dead service.
- Fix direction (per the Nov 7 review): narrow the lock to the state mutation, not the whole routine run — or split the routine's long work out from under the guard.

### 316 — Hot-path file I/O on every log line and bus message

- `src/utils.rs` — `error_log_path()` calls `fs::create_dir_all(&log_dir)` on **every invocation**, and `log_to_file` calls `error_log_path()` on **every log line**. That is a `create_dir_all` (syscall + metadata check) per log line, even though the directory already exists. Cache the path once (a `OnceLock`) instead.
- `src/bus/mod.rs:117-133` — `publish()` calls `log_transaction()` on **every message**, which opens `logs/bus_log.md` (relative path, CWD-dependent) and appends a line. Every bus message pays a file open + write.
- At ~54k error-log lines/day plus bus chatter, this is a constant background of syscalls and file opens. It is not catastrophic, but it is the kind of cost that shows up as latency tail on the routines that log most.

**Recommended order (unchanged from the Nov 7 review): 314 + 316 → 315.** The log cleanup and hot-path I/O are independent and small; the mutex rework is the riskier one and should come after.

---

## 5. The restart signal is dead in production 🟠

- `count_restarts` (`src/self_improve/metrics.rs:223`) counts occurrences of the marker `"Helix is running..."` in files under `<runtime>/logs/`.
- `main.rs:139` only `println!("Helix is running...")` — stdout. In production the service's stdout goes to the systemd journal, **not** to a file under `<runtime>/logs/`.
- `log_to_file` writes to `~/.helix/logs/error_log.md` (HOME-based, `src/utils.rs:15`), not to `<runtime>/logs/`. The runtime logs dir is dead on the write side.
- Net effect: `restart_count` is **always 0 in production**. The self-improvement loop's "restarts detected" trigger (`routine_self_improve`, `src/cpu/mod.rs:538`) never fires.

**Fix direction:** either have `main.rs` write the boot marker to a file under the runtime logs dir at startup, or point `count_restarts` at the journal / the HOME log path. One line of startup code closes it.

**Related (🟡): the web UI error view is blind for the same reason.** The web UI reads `<runtime>/logs/error_log.md`, but writes go to `~/.helix/logs/error_log.md`. The error view shows nothing (or stale content). The loop's own error signal is **not** affected — `check_error_logs` (`src/cpu/mod.rs:956`) checks **both** the runtime path and the HOME path, so the loop reads the live file. Only the restart input and the web UI view are on the dead path.

---

## 6. Retired Gemini model is still the default 🟠

- `src/main.rs:656` — the Gemini listener defaults to `"gemini-2.0-flash"` when `GEMINI_MODEL` is unset.
- `gemini-2.0-flash` was shut down by Google June 1. The replacements are `gemini-3.6-flash` / `gemini-3.8-flash`.
- John said "I'll change it myself" (2026-10-02) and it is still his to change — but as of this review it is still the live default, so a Gemini listener start will fail or route to a dead model. Noting it here because it is the kind of default that bites silently at 3 AM.

---

## 7. Chat-command loop + MCP handler ✅

The chat-command machinery is in good shape:

- **MCP handler (task 319, `src/chat_command/mcp_handler.rs`) is well-documented and fail-closed.** Read-only allowlist per server (`google` → `gmail_search`/`gmail_read`/`calendar_list`; `chat` → `chat_read`/`chat_rooms`/`chat_health`; anything else → no read tools, fail closed). `to` is required (a missing `to` is a rejection, not a broadcast). Any `write:` scope is rejected at parse time. All three checks run **before** any call, and a failure posts a rejection naming the problem and executes nothing.
- **Documented limitation, not a bug:** the `time` budget bounds how long the tick *waits*, it does **not** kill the MCP server child. A runaway call keeps running in the background while the tick moves on (its result is dropped). The doc comment says so plainly. A real kill handle is a follow-up.
- **`to` addressing works** — verified live 2026-11-08 (drills (g)/(h) passed on both Helix and the app).
- **Queue is in-memory only** (`HandlerState.queues: HashMap<LlmBackend, VecDeque<QueuedTask>>`, `src/chat_command/task_handler.rs:232`). A Helix restart drops queued and running tasks. That is the known v1 limit, not a regression — but it is a durability gap worth naming: a restart during a queued LLM task silently loses it.
- **google-mcp is the weak link, not the plumbing.** The MCP drills (c)–(f) all passed; drill (a) (a real `gmail_search`) timed out at 60s and again at 120s on 2026-11-08. The plumbing is proven; the binary/OAuth on the R630 is the suspect. Still open, still John's R630 check.

---

## 8. Code quality notes 🟡 / 🔵

- 🟡 **`println!` in production code: 255 occurrences** across `src/` (excluding tests). The WS `chat` arm alone prints the prompt preview, bus routing, and publish confirmations to stdout. Production logging should go through `log::` so it is filterable and does not pollute the journal. Low risk, but it is noise that makes the real logs harder to read — and it is the same stdout that the restart marker is lost in (§5).
- 🟡 **The monoliths remain.** `src/io/web_server/mod.rs` is 4,074 lines; `src/cpu/mod.rs` is 2,819 lines; `src/main.rs` is 744 lines. The auth extraction (`auth.rs`) is a good start and task 318 (decompose `web_server/mod.rs`) is `in_progress`, but the bulk of the decomposition is still ahead. The auth split proves the pattern works — keep going in that direction.
- 🟡 **`agent-client-protocol` is 5.0.0; crates.io latest is 5.1.0.** One minor behind, days after the 37-dep update (task 317) landed. Low risk; fold the bump into the next dep pass rather than doing it alone.
- 🔵 **Task 262 (installer `include_str!`) still pending.** `src/bin/installer.rs:273-274` still bakes in `../../config.toml` and `../../.grok/docs/system_manifest_root.md`. The fresh-checkout installer build break persists. The 262.4 decision is still pending — noting it, not re-litigating it.
- 🔵 **Task 292 is unfiled.** There is no task 292 in `.zed/task_list.json` (grep finds none). The web UI routine-toggle bug is still a draft file, not a task.
- ✅ **Web server production code has zero `unwrap()` / `expect()`.** Lines 1–2553 of `src/io/web_server/mod.rs` contain neither. All 104 `unwrap()` occurrences in that file are inside the `#[cfg(test)]` module. For a network-facing module, that is excellent discipline.
- ✅ **`cpu/mod.rs` is reasonable on unwraps** (~37 in the first 2,700 lines, mostly benign) — not a finding, just a check.

---

## 9. What is good ✅

- The 313 WS auth gate holds, with a real test driving the real `/ws` endpoint.
- The auth module extraction is clean, tested, and the right shape for the task 318 decomposition.
- The MCP handler is fail-closed, well-documented, and honest about its one limitation (no kill handle).
- The `to` addressing and MCP rejection drills all passed live.
- The 37 dep updates landed cleanly (agent-client-protocol 5.0.0, russh 0.65.1, axum 0.8.9, reqwest 0.13.5).
- The self-improvement module's safety rules are stated as construction, not convention — it cannot merge, push, restart, write the live tree, or send mail, because no code path in it does those things.
- The error-log signal for the loop is live (both paths checked).
- Web server production code is `unwrap()`-free.

---

## 10. Verification

| Check | Result |
|---|---|
| `cargo check --lib` | **FAILS** — `error[E0599]: no method named 'ct_eq' found for reference '&[u8]'` at `src/io/web_server/mod.rs:506`. The lib does not compile at PreRelese 7de0884. |
| `cargo clippy --lib` | **FAILS** — same E0599 (rustc error, not a lint). 2 build-script warnings (`build.rs:43,69` collapsible `if`) and 2 lib warnings (`src/router/config.rs:182` empty line after doc comment; `src/io/web_server/auth.rs:96` unused variable `v`) were emitted before the error. |
| `cargo test --lib` (WS auth gate + MCP handler) | **Could not run** — no test binary is produced because the lib does not compile. The `ws_mutating_arms_require_auth` test exists (`src/io/web_server/mod.rs:3785`) but cannot execute until the build break is fixed. |
| `cargo check --bin helix` | **FAILS** — same E0599 (the bin depends on the lib). |
| Full test suite (726 tests) | **Not run** — blocked by the build break. |
| Toolchain | Rust 1.97.1 in the sandbox (two minors behind current stable 1.99.1). The E0599 is a real missing-trait-import error, not a toolchain-version artifact — it fails on any toolchain. |

The Quartermaster's last green (lib 635/0/4) was at 4a3c1b5, **before** the e4dec24 auth split. The break was introduced by e4dec24/2d2557d. No test numbers can be reported until the build is restored — that is itself the headline finding.

---

## 11. Findings index

| # | Severity | Finding | Location |
|---|----------|---------|----------|
| H0 | 🔴 Critical | **The lib does not compile** — e4dec24 auth split removed the `subtle` import but left duplicate auth fns in `mod.rs` that call `.ct_eq()`; `cargo check/clippy/test --lib` all fail with E0599 | `src/io/web_server/mod.rs:505,512,525,549` |
| H1 | 🟠 High | `.zed/task_list.json` has unresolved merge conflict markers (invalid JSON) | `.zed/task_list.json:3797,3907,3933` |
| H2 | 🟠 High | Tasks 314/315/316 still pending (dead log cleanup, Cpu mutex, hot-path I/O) | `src/cpu/mod.rs:1146`, `src/main.rs:403-404,420-421`, `src/utils.rs`, `src/bus/mod.rs:117` |
| H3 | 🟠 High | Restart signal dead in production (`count_restarts` always 0) | `src/self_improve/metrics.rs:223`, `src/main.rs:139` |
| H4 | 🟠 High | Retired `gemini-2.0-flash` still the default model | `src/main.rs:656` |
| M1 | 🟡 Medium | Web UI error view is blind (reads the dead runtime log path) | `src/io/web_server/mod.rs:148`, `src/utils.rs:15` |
| M2 | 🟡 Medium | 255 `println!` in production code | across `src/` |
| M3 | 🟡 Medium | `require_okf_write_token` is a copy-paste duplicate of `require_web_token` | `src/io/web_server/auth.rs` |
| M4 | 🟡 Medium | `agent-client-protocol` 5.0.0, latest is 5.1.0 | `Cargo.toml:35` |
| M5 | 🟡 Medium | Chat-command queue is in-memory only (restart drops tasks) | `src/chat_command/task_handler.rs:232` |
| M6 | 🟡 Medium | Monoliths remain (`web_server/mod.rs` 4,074 lines, `cpu/mod.rs` 2,819) | `src/io/web_server/mod.rs`, `src/cpu/mod.rs` |
| L1 | 🔵 Low | Task 262 installer `include_str!` still present | `src/bin/installer.rs:273-274` |
| L2 | 🔵 Low | Task 292 unfiled (no task 292 in the list) | `.zed/task_list.json` |
| L3 | 🔵 Low | `constant_time_token_eq` leaks token length (early return) | `src/io/web_server/auth.rs` |

**Verdict: 🔴 Fix the build break first — the lib does not compile.** Once the build is restored, the verdict on the rest is 🔧 fix-and-sign-off: the security work holds and the new code is good. The fixes after the build are the unfinished trio (314/315/316), the broken task list, the dead restart signal, and the retired model default.
