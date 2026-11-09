# Code Review: helix (Claude)

**Date:** 2026-11-09
**Reviewer:** Claude Opus 5.5 (Zed Agent)
**Branch / commit:** `PreRelese` @ `2d2557d`
**Scope:** `src/` (~55k lines incl. tests), `Cargo.toml`, `build.rs`
**Method:** `cargo clippy --all-targets` on Windows 11, plus a manual review split by subsystem. I spot-checked the highest-severity findings against the source. Line numbers are as of `2d2557d`.

---

## Executive Summary

Helix is ambitious and in many places carefully engineered. The git wrapper is a real allowlist. `resolve_inside` sandboxes paths properly. SQL is fully parameterised. Config defaults fail closed. Secret redaction is section-aware. However:

1. **The crate does not compile right now**, either as a library or as a test target on Windows.
2. **The web UI's WebSocket is the largest attack surface.** `chat` is never auth-gated, there is no `Origin` check, and the bundled UI never authenticates.
3. **Network resilience doesn't meet the Starlink requirement.** Retries give up after about 5 seconds. IMAP and git have no timeouts. Several paths have no retry at all.
4. **The Gemini API key goes in the URL query string**, so it leaks through reqwest error messages into logs and the UI.
5. **Blocking work runs inside the async runtime** in many places: the bus `recv()`, `tools::execute`, IMAP, and vector-memory embedding.
6. **Windows support is partial.** The shell tool is hard-wired to `/bin/sh`, the installer recurses through `sudo`, and the self-improve diff-path checks only split on `/`.

**Overall score: 6/10.** The design is good, but the current build state, the WebSocket exposure and the network fragility hold it back.

---

## Build & Clippy Results (Windows 11)

`cargo clippy --all-targets`:

| Kind | Location | Message |
|---|---|---|
| **error** (lib) | `src/io/web_server/mod.rs:506` | `no method named ct_eq` (missing `use subtle::ConstantTimeEq`) |
| **error** (test) | `src/chat_command/mod.rs:1218` | `cannot find unix in os` (`std::os::unix::fs::PermissionsExt`) |
| **error** (test) | `src/chat_command/mod.rs:1226` | `Permissions::from_mode` not found (Unix-only) |
| warning | `src/io/web_server/auth.rs:117` | unused import `axum::http::HeaderMap` |
| warning | `src/io/web_server/auth.rs:96` | unused variable `v` |
| warning | `src/chat_command/executor.rs:162` | unused variable `pid` |
| warning | `src/router/config.rs:182` | empty line after doc comment |
| warning | `build.rs:43`, `build.rs:69` | collapsible `if` |

**Root cause of the lib error:** the latest commit split auth into `web_server/auth.rs`, but `mod.rs` kept its own copy of the auth/redaction helpers, and that copy lost the `subtle` import. The two copies have already drifted, and the `auth.rs` versions are unused (hence the warnings). Delete the copy in `mod.rs` and `use auth::{...}`.

---

## Findings

Severity: **Critical**, **High**, **Medium**, **Low**.

### Critical

#### C1. Crate does not build
See the table above. Nothing ships or tests until this is fixed. Task 313 / the auth split is marked done in the task list, but it left the tree broken.

#### C2. Gemini API key leaks via URL into logs and the UI
- `src/io/llm_gemini/mod.rs:100`, `:545`, `:575`: `format!("{}/{}:generateContent?key={}", ..., api_key)`
- reqwest error strings include the full URL. `map_reqwest_err` (around `:497-524`) copies that text into the error, which is logged with `warn!` (around `:144`) and broadcast to `web_interface` as a retry warning. **Every Starlink drop can print the key.**
- **Fix:** send the key in the `x-goog-api-key` header, and call `e.without_url()` inside `map_reqwest_err`.

#### C3. Self-improve diff can overwrite the worktree's `.git` file (latent)
- `src/self_improve/evaluator.rs:222-230`, `resolve_diff_path`:
  ```rust
  if path.split('/').any(|seg| seg == "..") { ... }
  Ok(worktree.join(path))
  ```
- A `DiffFile { path: ".git", content: "gitdir: <repo>/.git" }` passes the check. The `git add`/`git commit` that follow then act on the **main repo's** index and HEAD.
- On Windows, `..\..\x` and `\Windows\x` also pass. A path starting with a backslash is not `is_absolute()` on Windows, so `join` resolves it against the drive root.
- This is latent because `evaluate_proposal` is only reached from tests today (Phase 0/1 only proposes). It must be fixed before Phase 2 is wired up.
- **Fix:** walk `Path::components()` and reject `ParentDir`, `RootDir`, `Prefix`, and any component equal to `.git` (case-insensitive), as `git::resolve_under_root` already does. Canonicalize after writing and assert the result is inside the worktree.

### High

#### H1. WebSocket `/ws`: `chat` is unauthenticated and there is no `Origin` check
- `src/io/web_server/mod.rs:1336`: the `"chat"` arm has no `authed` check, unlike `routine_delete` (`:1311`) and others.
- There is no `Origin` validation on upgrade (no matches for `Origin` in `mod.rs`). Once the user has accepted the self-signed cert, any web page can open `wss://localhost:8443/ws` (cross-site WebSocket hijacking).
- Chat replies are broadcast to every connected client, so an unauthenticated client can also read other people's chat.
- With no token configured, `config_save` and `slash_cmd` are fully open, and slash commands include `/support send`.
- **Fix:** reject upgrades whose `Origin` isn't this server. Require auth (a first message, or a token in the query string) before subscribing a client to broadcasts or accepting `chat`.

#### H2. Bundled UI never authenticates
- `src/io/web_server/static/index.html` never sends `{"type":"auth"}`. Once `[web] auth_token` is set, config saves, routine edits and slash commands from the UI always fail. In practice this pushes users to leave the token empty, which makes H1 worse.

#### H3. Retry policy can't survive real Starlink drops
- Ollama uses a fixed 2.5 s delay (`src/io/ollama/mod.rs:76`, `:529`), even though its docs say "exponential-ish". Gemini waits 2 s then 4 s (`src/io/llm_gemini/mod.rs:20`, `:165`). Both give up after about 5-6 s, while drops last 10-60 s.
- `call_gemini_direct` (`llm_gemini/mod.rs:535`), the OKF fetcher (`src/okf/fetcher.rs:74`, `:118`) and IMAP have no retry.
- Ironically, the only proper exponential backoff is in `io/ollama/keepalive.rs`, which is not declared as a module and never compiles (see L4).
- **Fix:** one shared `retry_with_backoff` helper: jittered exponential backoff, about 2 s rising to a 60 s cap, about 5 attempts. It should honour `Retry-After` on 429/503 and fail fast on other 4xx.

#### H4. Retries re-run the whole tool loop (duplicate side effects)
- `src/io/llm_gemini/mod.rs:107-110` and `src/io/ollama/mod.rs:443-444`: the retry wraps the multi-round tool loop.
- A failure in round 3 replays rounds 1-2, including `send_email` and `write_note`, bills Gemini again, and re-streams tokens the UI already showed.
- **Fix:** retry each HTTP call inside the loop, not the loop as a whole.

#### H5. IMAP and SSH2 connections have no timeouts
- `src/tools/email_tools.rs:285`: `TcpStream::connect((host, port))` has no `connect_timeout` and no read/write timeouts, so it can hang forever on a dropped link.
- `src/tools/project_scanner.rs:245` (ssh2) never calls `sess.set_timeout`.
- **Fix:** `connect_timeout(10s)`, `set_read_timeout`/`set_write_timeout(30s)`, and bounded retry.

#### H6. Blocking work inside the Tokio runtime
- `src/main.rs:384`, `:528`, `:617`, `:662`: blocking `std::sync::mpsc::recv()` inside `tokio::spawn`. Four listeners pin four worker threads for good, which can starve the runtime on a 4-core box.
- `tools::execute` (whose shell path busy-waits with `thread::sleep` for up to 3600 s, and whose IMAP path blocks) is called directly from `async fn`s (`io/ollama/mod.rs:818`, `llm_gemini/mod.rs:455`, `cpu/mod.rs:504`).
- Web handlers `support_send` and `slash_cmd` (around `web_server/mod.rs:2104`, `:1682`) call sync tools. `ops_restart` (`:2256`) uses blocking `Command::output()`.
- `memory/vector.rs` embedding blocks for up to 30 s via `recv_timeout` and is called from `async fn consolidate_memory_inner` (`cpu/mod.rs:818`).
- **Fix:** `tokio::task::spawn_blocking` + `tokio::time::timeout`, or move the bus to `tokio::sync::mpsc`.

#### H7. Cpu mutex held across slow network work
- `src/main.rs:402-404` → `src/cpu/mod.rs:465-486`: `routine_support_mail_watch` runs under the Cpu lock while it fetches mail and drafts up to 50 replies (300 s timeout each, `support_watch/drafter.rs:190`). The heartbeat (`main.rs:420`) stalls, and the minute-cron piles up waiters.
- **Fix:** do the network work outside the lock, and skip a run if the previous one is still in flight (`AtomicBool`).

#### H8. Shell denylist bypass via command chaining
- `src/tools/shell_security.rs:96-123`: `check_shell_argv0` only analyses `words[idx]` of the whole command, and `validate_shell_command` never splits on `;`, `&&`, `||`, `|`, `&` or newlines.
- `echo; rm -fr /` passes: argv0 is `echo;`, and the substring list has `rm -rf /` but not `-fr`. `curl x|/bin/bash` also passes.
- On Windows, `rd /s /q C:\`, `del /s`, `format c:` and `Remove-Item -Force -Recurse C:\` (flag order) are not caught.
- **Fix:** split into segments and run argv0 analysis on each, normalise flag order, and add Windows destructive-command patterns.

#### H9. Background children escape the shell timeout
- `src/tools/shell_tool.rs:268-289` and `src/chat_command/executor.rs:172-195`: with `sleep 99999 &`, `sh` exits, but the background child keeps the stdout pipe open, so `out_handle.join()` blocks forever.
- **Fix:** kill the process group after reaping (a Job Object on Windows) and join the drain threads with a deadline.

#### H10. Model-authored tests run with full privileges (latent)
- `src/self_improve/evaluator.rs:103-122`: `cargo test` in the worktree inherits the full environment (`GEMINI_API_KEY` etc. from `.env`) and network access, and has **no timeout**.
- A proposal can add `build.rs` or `.cargo/config.toml`.
- **Fix:** `env_clear()` + allowlist (`PATH`, `CARGO_HOME`, `RUSTUP_HOME`), a hard timeout with kill, and ideally a restricted user, job object or container.

#### H11. Audit sensitive-path check is trivially bypassed
- `src/self_improve/audit.rs:192`: `EXACT_PATHS.contains(&path)` compares raw strings, so `./src/security.rs`, `src//cpu/mod.rs` and `src\tools\shell_tool.rs` don't match.
- `src/self_improve/**` and `tests/evals.rs` are not protected, so the loop can edit its own gate and the evals it is graded on.
- **Fix:** normalise paths (components, `/` separators, lowercase on Windows). Protect `src/self_improve/`, `tests/`, `agents/`, `build.rs`, `Cargo.toml`, `.cargo/` and `.github/`.

#### H12. Byte-index string slicing panics on non-ASCII
- `src/acp/agent.rs:263` (`&s[..MAX]`), `src/acp/server.rs:243` (`&args[..80]`), `src/io/llm_gemini/mod.rs:94`, `src/cpu/mod.rs:1352`, `:1384`, `:1399`.
- `system_status` output contains `•`, `—` and `🖥️`. A panic in the A2A worker leaves the task stuck in `Working` forever, and in the Gemini listener it kills the listener for good.
- **Fix:** use the existing `crate::utils::truncate_str` everywhere.

#### H13. Spend cap not enforced on normal Gemini chat
- `check_api_allowed` is only called from `tools/subagent_tool.rs:280`/`:344`. A chat turn can make 10 rounds × 3 attempts = 30 billed calls with `cap_block_on_exceed = true`.
- **Fix:** check the cap before each `generateContent` round.

### Medium

| # | Location | Issue | Fix |
|---|---|---|---|
| M1 | `web_server/mod.rs:1012-1013` + `bus/mod.rs:88-94` | Each WS connection spawns an OS thread and a bus subscriber that is never removed. Subscribers grow with every reconnect (worse on Starlink), and with N tabs every message is duplicated N times. | One subscriber at startup, fan out via `broadcast`, add `Bus::unsubscribe`. |
| M2 | `web_server/mod.rs:190` | Config parse failure → `unwrap_or_else` fallback silently drops `auth_token` (auth off) and sets `tls_enabled: true`, contradicting the serde default. | Log the error. Read `[web]` from a `toml::Value` or make the tables `#[serde(default)]`. |
| M3 | `security.rs:142` | Content containing `</untrusted-data>` closes the prompt-injection wrapper early. | Escape or neutralise the delimiters inside content. |
| M4 | `security.rs:83`, `:87` | Shingles only cover the first 2000 chars. Eviction from a plain `HashSet` removes shingles still needed by a newer duplicate. | Raise or remove the cap, and reference-count shingles. |
| M5 | `static/index.html:1571` | Stored XSS: `supportEsc` doesn't escape `\`, so a crafted inbound-email `to` breaks out of the inline `onclick`. `d.id` is not escaped. | Use `addEventListener` + `dataset`, with no inline handlers. |
| M6 | `static/index.html:1630` | WS URL hardcodes `:8443`. | Use `location.host`. |
| M7 | `web_server/mod.rs:2172` | 10 MB voice limit is unreachable because axum's default 2 MB body limit applies first. | `DefaultBodyLimit::max(10 MiB)` on that route. |
| M8 | `web_server/mod.rs:315`, `:2311` | Agent card advertises `127.0.0.1` / `0.0.0.0`, but the cert only covers `localhost`. | Add IP SANs, and make the public URL configurable. |
| M9 | `tools/shell_tool.rs:231` | `Command::new("/bin/sh")` makes the shell tool unusable on Windows 11. | `cfg(windows)` branch using `cmd /C` or `powershell -NoProfile -Command` + Job Object. |
| M10 | `tools/file_tools.rs:224-225` | `scan_directory` accepts any absolute path with unbounded depth and follows symlinks, so it can list the whole filesystem. | `resolve_inside`, clamp depth, `symlink_metadata`. |
| M11 | `mcp_client/mod.rs:246`, `manager.rs:111` | Global MCP mutex held across `call_tool` (up to 120 s). `list_all_tools` has no timeout. Dead servers are never reconnected, and an empty `defs_cache` is cached forever. | Per-server locks or clone `Peer`, timeouts, reconnect on transport error. |
| M12 | `chat_command/mcp_handler.rs:117-127` | On timeout, the worker thread leaks and keeps holding the MCP mutex. | Use `tokio::time::timeout` inside the bridge so the call is actually cancelled. |
| M13 | `chat_mcp_install.rs:104`, `:327` | A Windows path in a TOML basic string becomes an invalid `\U` escape and corrupts the config. The binary-name check ignores `.exe`. | TOML literal string or `toml` serialisation, plus `EXE_SUFFIX`. |
| M14 | `chat_mcp_install.rs:250`, `:278` | `git clone`/`pull`: no timeout, no retry, can prompt for credentials. | `GIT_TERMINAL_PROMPT=0`, `http.lowSpeedLimit/Time`, retry. |
| M15 | `chat_command/executor.rs:26` | Allowlisted `hostname` can set the hostname (as root). `cat`/`grep`/`tail` can read `.env`, `config.toml` and `~/.ssh`. `tail -f` hangs the tick. | Arg-less `hostname`, directory allowlist, reject `-f`. |
| M16 | `self_improve/budgets.rs:91` vs `worker.rs:173-179` | "Open" statuses differ: `evaluated` and `needs-human-review` aren't counted, so duplicates get through. | One shared `OPEN_STATUSES` constant. |
| M17 | `ssh/handler.rs:175`, `:201` | Session counter only decremented in `channel_close`. Abrupt disconnects leak slots until all sessions are refused. | Decrement in `Drop`. |
| M18 | `ssh/handler.rs:140`, `:63-71` | Failed-auth limiter counts successful logins, and the IP map never evicts. | Record only on reject, and prune entries. |
| M19 | `bin/installer.rs:25-38` | On Windows `id` fails, so the installer re-runs itself via `sudo`. That recurses with Win11 sudo, or panics without it. | `#[cfg(unix)]` or a clean error on Windows. |
| M20 | `bin/installer.rs:298`, `:465`, `:522` | `.env`/`key.pem` written as root with the default umask. Fixed `/tmp/helix_*.pem` paths. `.env` backups pile up. | Mode `0600`, private temp dir, prune backups. |
| M21 | `memory/vector.rs:204-220`, `:289-301` | No cap or dedupe. Each `add_fact` rewrites the whole pretty JSON (768-float vectors) and fsyncs it: O(n) per insert. | Cap/evict, append-only records. |
| M22 | `support_watch/state.rs:113-118` | Corrupt `state.json` → empty `seen_ids` → with `auto_send`, replies to all 50 fetched mails. | Quarantine the file, mark the current batch as seen without drafting, log loudly. |
| M23 | `router/spend.rs:69-70`, `:100-111`, `:141`, `:226`/`:269` | `thoughtsTokenCount` (billed) is ignored. Check and record are separate steps (a race lets parallel calls overshoot). The day key is UTC. Prices look stale (inference, not verified). | Count thinking tokens, reserve before the call, use a local-day key. |
| M24 | `cron/registry.rs:546` | `next_run_ms == None` (unparseable schedule) is treated as due on every tick. | Treat `None` as disabled. |
| M25 | `cron/registry.rs:163`, `:177` | `.single()` returns `None` on DST gaps/overlaps, so the seeded 02:00 daily job is skipped on spring-forward day. | `.earliest()` or roll forward to the next valid minute. |
| M26 | `cron/registry.rs:535` | Non-atomic rewrite every minute. A crash mid-write corrupts the file, and `load_or_seed` silently reseeds, losing user routines. | Temp file + rename (as `support_watch/state.rs` already does). |
| M27 | `io/ollama/mod.rs:1222`, `:1232` | HyEvo/planner client is `Client::new()` (no timeout) plus `.expect("No Ollama backends configured")`. | `shared_client()` and return an error. |
| M28 | `persona.rs:112-126` | Frontmatter offsets assume `\n`. `agents/loop-worker.md` is CRLF in this Windows checkout, so the body starts with a stray `-\r\n`. | `split_inclusive('\n')` with real lengths, or normalise line endings. |

### Low

- **L1** `web_server/mod.rs:1408` and `main.rs:529`: `println!` of prompt and token previews breaks the Task 265 "log length only" policy. `main.rs:529` also fires once per streamed token.
- **L2** `a2a/tasks.rs`, `a2a_handler.rs:285`, `:318`: the task store and session map are never evicted, `run_prompt` has no timeout, and there is a cancel race between `spawn` and `set_worker`.
- **L3** `io/unix_socket.rs`: the whole file is `#[cfg(unix)]`, so its inner `#[cfg(windows)]` branches (`:162`, `:216`) are dead code. The ops endpoints rely on `/proc`, `setsid` and `systemctl`, so update/restart never work on Windows.
- **L4** Dead files never declared with `mod` (632 lines that never compile): `io/ollama/llm.rs`, `io/ollama/keepalive.rs`, `ollama_handler_test.rs`, `gemini_handler_test.rs`. `router/schedule.rs` is unused, evaluates hours in UTC, and can't match windows that wrap midnight.
- **L5** Default Gemini model is defined in three places that disagree: `main.rs:656` (`gemini-2.0-flash`), `llm_gemini/mod.rs:24` (`gemini-3.6-flash`), and the doc comment.
- **L6** `io/ollama/mod.rs:617`: `from_utf8_lossy` runs per network chunk, which garbles multi-byte characters split across chunks.
- **L7** `self_improve/git.rs:161-168`: `worktree add <p> <branch> X` (without `-b`) leaves `argv[4]` unchecked. `is_safe_branch_name` accepts `master`/`main` for `delete_branch`.
- **L8** `ssh/server.rs:149-156`: host key written and then `chmod`ed (a race), with no ACL on Windows. `:230-233`: a transient `accept()` error ends the listener for good.
- **L9** `hy_evo/workflow.rs:162-163`: `indegree[*to] += 1` panics on an out-of-range edge loaded from SQLite.
- **L10** `memory/episodic.rs:59`: reads the entire never-rotated `episodes.jsonl` at boot.
- **L11** `Cargo.toml`: `native-tls` is listed even though the comments say the goal is to avoid native-tls/OpenSSL, and both `ring` and `aws-lc-rs` (via `russh`) crypto backends are pulled in. `dotenv` is unmaintained (use `dotenvy`). `rust-version = "1.99.0"` should be checked against your toolchain.

---

## Maintainability

| File | Lines | Note |
|---|---|---|
| `src/io/web_server/mod.rs` | 4,074 | Duplicates the auth helpers in `auth.rs`. The WS handler is one giant `match`. Split it into `ws/`, `routes/`, `ops.rs`. |
| `src/io/web_server/static/index.html` | 3,213 | Inline JS + CSS + inline event handlers (the source of M5). Split into `.js`/`.css`. |
| `src/cpu/mod.rs` | 2,819 | About 1,880 lines of production code, 89 functions. `handle_bus_message` is about 200 lines and `routine_self_improve` about 170. Split into `routines/`, `bus_dispatch.rs`, `nightly.rs`. |
| `src/chat_command/mod.rs` | 1,863 | Contains the Unix-only test that breaks the Windows test build. |
| `src/io/ollama/mod.rs` | 1,335 | Has an inline `pub mod llm` that duplicates the dead `llm.rs`. |

**Repository hygiene:** `####agents.md` (odd filename), `hartbeat.md` (typo), and a Jest/TypeScript config that is unrelated to the Rust crate (see the harness review). `spend_state.json` is not in `.gitignore`.

---

## Strengths

- **Git wrapper** (`self_improve/git.rs`): argv-only allowlist with no shell, environment stripped, `credential.helper=` set, and every rejection tested.
- **Path sandbox** (`resolve_inside`): canonicalizes both sides and rejects NUL bytes, `..`, symlink escapes and `\\?\` prefixes. It fails closed.
- **Chat-command executor:** argv-only spawn, metacharacter screen, code-level allowlist, `stdin(null)`, and capped, lossy pipe draining.
- **Web auth middleware:** gated routes use `route_layer` sub-routers and constant-time comparison. Secrets go over a per-connection `direct_tx` channel, never broadcast.
- **Config redaction/restore:** section-aware, and refuses to save if a placeholder survives.
- **SQL:** every `sqlx` query is parameterised.
- **Fail-closed defaults:** loopback bind, no permissive CORS, self-improve kill switches checked first, queue-only PRs, key-only SSH.
- **Atomic writes and caps** in `support_watch` (seen-ID cap, retry limit).

---

## Recommended Fix Order

1. **C1:** get the build green (remove duplicate auth helpers; `#[cfg(unix)]` on `stub_ssh` and its tests).
2. **C2:** move the Gemini key to a header and strip URLs from errors.
3. **H1/H2:** gate WS `chat`, add the `Origin` check, and make the UI authenticate.
4. **H3/H4/H5:** shared backoff helper, per-request retry, timeouts on IMAP/SSH2/git. This is the Starlink requirement.
5. **H6/H7:** move blocking work off the runtime and out of the Cpu lock.
6. **H8/H9/H12/H13:** shell segmenting, timeout escape, UTF-8 slicing, spend cap.
7. **C3/H10/H11:** harden self-improve before Phase 2 is enabled.
8. Medium/Low items, then the god-module splits.

Run `cargo clippy --all-targets -- -D warnings` after each step.

---

*Generated by Claude Opus 5.5 (Zed Agent). Source of suggestions: AI. I verified the build/clippy output and the C1-C3, H1, H2, H8 and H13 call-site claims directly. Other line references come from subsystem review passes; re-confirm them before editing, as lines will shift.*
