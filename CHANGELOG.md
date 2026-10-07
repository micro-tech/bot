# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased] — 2026-10-04

**Author:** AI Assistant (Claude Sonnet 4.6) — triggered by user "Cobble"

### Fixed

- **Gemini TLS failure** (`src/io/llm_gemini/mod.rs`) — reqwest was compiled
  with `rustls-tls-manual-roots-no-provider` (0.12) / `rustls-no-provider`
  (0.13), which starts with an empty CA trust store. Every HTTPS connection to
  the Gemini API silently failed at TLS handshake. Fixed by upgrading to
  reqwest 0.13 where `rustls-no-provider` bundles Mozilla root certs
  automatically.

- **Misleading error label** (`src/io/llm_gemini/mod.rs`) — `reqwest`'s
  `is_connect()` fires for TCP refused, DNS failures, _and_ TLS cert errors.
  The handler was labelling all three as `"connection refused / DNS failure"`.
  Error classification now inspects the error string to distinguish TLS errors,
  DNS failures, and TCP errors separately.

- **Missing `router` module** (`src/main.rs`, `src/lib.rs`) — `src/router/`
  existed on disk but was not declared in either crate root. Added `mod router;`
  to both `main.rs` (binary crate root) and `lib.rs` (library crate root).

- **Missing `plan_tree` and `memory_continuity` modules** (`src/agents/mod.rs`)
  — both files existed but were not declared. Added `pub mod plan_tree;` and
  `pub mod memory_continuity;`.

- **Missing crates** (`Cargo.toml`) — `regex` and `notify` used in
  `src/router/complexity.rs` and `src/router/config.rs` respectively but not
  declared as dependencies. Added `regex = "1"` and `notify = "6"`.

- **reqwest feature name change** (`Cargo.toml`) — after version bump to 0.13.5
  the feature `rustls-tls-manual-roots-no-provider` no longer exists. Updated
  to the correct 0.13 name: `rustls-no-provider`.

- **`helix.service` wrong user/path** (`helix.service`) — hardcoded `User=helix`
  and `WorkingDirectory=/opt/helix` did not match the actual deployment user
  `cobble`. Updated to `User=cobble` / `WorkingDirectory=/home/cobble/helix`
  and added `EnvironmentFile` so `.env` is loaded by the service.

### Fixed (warnings)

- Removed unused imports: `std::path::Path` (okf/registry.rs),
  `std::collections::HashMap` (agents/plan_tree.rs),
  `serde_json::Value` (agents/memory_continuity.rs),
  `std::time::Duration` (router/config.rs),
  `LLMBackend / OverrideCommand / OverrideTier / TelemetryCollector`
  (router/integration.rs).
- Prefixed intentionally unused variables with `_` in cpu/mod.rs,
  router/health.rs, router/telemetry.rs, agents/runtime_trace.rs,
  okf/fetcher.rs, tools/mod.rs.
- Removed unnecessary `mut` on `ctx` in router/integration.rs.

### Documentation

- Rewrote `docs/configuration.md` to match the actual `config.toml` structure
  (all sections: ollama array, gemini, web, logging, heartbeat, ollama_keepalive,
  reasoning, helix.okf).
- Added `docs/deployment.md` covering install, update workflow, branch strategy,
  directory layout, and troubleshooting.
- Updated `docs/docker_systemd.md` to reference the installer binary instead of
  manual service file copy.
- Updated `docs/logging.md` with correct server paths and web UI viewer info.
- Updated `readme.md` quick-start and removed broken doc links.

---

## [Unreleased]

### Fixed — Port tests/*.rs to the current crate name; unmask follow-on failures (task 211, 2026-10-05)

- All `bot::` → `helix::` across `tests/*.rs` and `tests/scenarios/*.rs`
  (18 E0433s gone; `cargo check --tests` clean). The port unmasked several
  never-compiled, never-run tests with real bugs — all fixed minimally:
- `tests/integration_harness.rs`: the bus uses blocking `std::sync::mpsc`;
  the harness spawned its listener counter with `tokio::spawn` on the
  single-threaded `#[tokio::test]` runtime, wedging the executor in a
  blocking `recv()` (hung the whole binary). Now `spawn_blocking`, and the
  subscription is taken before spawning so the blocking thread holds only
  the `Receiver` — dropping the `TestContext` ends the `recv()` loop instead
  of an `Arc<Bus>` cycle keeping it alive forever. Two scenario tests now use
  `publish_and_wait` (their raw `bus.publish` never incremented the metrics
  counter their assertions checked); removed unused `anyhow::Result` import.
- `tests/planner_v2_tests.rs`: the test-local `Dummy` never implemented
  `Planner`, so `adapter.decide()` couldn't compile. Hoisted one `Dummy`
  with a message-driven `Planner` impl (`use tool:` → `ToolCall`,
  `final answer:` → `FinalAnswer`) matching the assertions' contract.
- `src/okf/api.rs` (`okf_traces`): **real production bug found via a flaky
  test.** `tokio::fs::File` pipelines writes — `write_all().await` returning
  `Ok` only means bytes were accepted into the pipeline, not written. The
  handler returned 202 before the JSONL line was visible (proven with a
  minimal repro: `write_all` Ok + immediate `read_to_string` → empty).
  Now `flush().await`s after the write, so 202 means the line landed.
  (Also gave each OKF self-fetch test a unique trace file — they shared one
  per-PID file while running in parallel.)
- `tests/runtime_end_to_end_test.rs`: `SimplePlanner`'s hardcoded `noop`
  tool was never registered in `crate::tools` → supervisor `FatalError` →
  halt at step 0. Test now uses a local `StatusPlanner` calling the real
  read-only `system_status` tool (same 3-steps-then-answer shape).
- `src/agents/runtime_loop.rs`: `FinalAnswer` branch now records the
  terminal step — previously `break` skipped the bottom-of-loop
  `record_step`, leaving finished runs at step 0 while the runtime trace
  had the step. (Error/Replan branches left as-is; out of scope.)
- Doctests: `src/tools/email_tools.rs` module doc `.env` block marked
  `text` (was compiled as Rust); `src/io/ollama/mod.rs`
  `check_ollama_health` example marked `ignore` (top-level `.await`).
- `tests/reflection_planner_adapter_test.rs`: removed 5 imports unused by
  its `assert!(true)` stub.
- Full suite: lib 265 passed ×2, integration 12+3+3+1+2+1+1+1+1, doctests —
  **0 failures everywhere**; no new warnings from these changes.

### Fixed — Warning cleanup: 8 new dead-code warnings from the mega-batch (task 210, 2026-10-05)

- `src/acp/agent.rs`: removed the unused `SimplePlanner` import the module
  docs already disclaimed.
- `src/agents/runtime_loop.rs`: **wired the rule layer instead of silencing
  it** (per the repair plan). `run()` now validates every `ToolCall`
  decision via `RuleLayer::validate` before execution — hallucinated tools,
  malformed args, and runaway consecutive-call loops are rejected into the
  transcript (recoverable) instead of executing or halting the run.
  Rejections consume step budget so a stuck planner can't spin forever.
  `AcpPlanner`'s allowlist fixed to `["system_status"]` (was empty — would
  have rejected its own grounding call).
- `src/io/web_server/mod.rs`: `#[allow(dead_code)]` on the parse-schema
  mirror fields (`Config.helix/heartbeat`, `HelixConfig.name`,
  `OllamaConfig.url/model`, `WebConfig.port`, `HeartbeatConfig.interval_seconds`)
  with why-comments — the web server re-parses raw TOML for the fields it
  needs; canonical config access stays in main.rs. Noted: the bind port
  arrives via the `start_web_server()` argument, not the schema field.
- `src/tools/project_scanner.rs`: `walk_ssh_directory` (ssh-disabled
  fallback) silenced via `cfg_attr(not(test), allow(dead_code))` — its only
  caller is the `cfg(test)` fallback test, a check artifact.
- Tests: 2 new `runtime_loop` unit tests (hallucinated-tool rejection,
  consecutive-call loop stop) green; full suite 265 passed, 0 failed.
- PRE-EXISTING FINDING (not fixed, out of scope): `tests/integration_harness.rs`
  and the other `tests/*.rs` files still reference the old `bot::` crate name
  (18 E0433 errors, present on the base commit) — `cargo check --tests` /
  `cargo test --tests` cannot compile until they're ported to `helix::`.

### Added — OKF protocol v1 + in-Helix server completion (tasks 207–208, 2026-10-05)

- `PROTOCOL.md` (repo root): the canonical OKF v1 contract both clients and
  the server conform to — canonical `/okf/*` paths, auth/ETag/error
  conventions, client mapping table, migration notes, Ubuntu deploy runbook.
- In-Helix OKF server completed (no standalone needed): new v1 routes
  `GET /okf/manifest.json`, `GET /okf/bundles/{id}/manifest.json`,
  `GET /okf/knowledge/{kid}`, `POST /okf/traces` (202, JSONL append),
  `POST /okf/bundles/{bundle}/concepts` (201, inserts + persists).
  Librarian retains the loaded manifest; ETag/304 on manifest GETs;
  `/okf/health` now returns `"status": "ok"`.
- New `require_okf_write_token` middleware: write endpoints accept the web
  token OR the OKF `auth_token` Bearer <redacted> grok-cli sends; fail-open when neither
  is configured (house posture).
- `[helix.okf] trace_file` config (default `okf_traces.jsonl`); config.toml
  documents the v1 surface.
- Tests: 6 fetcher contract tests + 3 self-fetch integration tests green
  (Helix fetches from itself); full suite 263 passed, 0 failed.

### Added — Dead memory modules wired or deleted (task 206, 2026-10-05)

- `Cpu::run_unified_agent` now opens every agent run with
  `inject_long_term_context` (recent episodes, semantically related vector
  facts, belief count into the fresh `AgentState`) and closes it by recording
  the run's `ImportantOutcome` — plus a 0.9-confidence `LessonLearned` when the
  run failed — then `persist_memory_deltas` into the disk-backed
  `MemoryManager`. Every function in `memory_continuity.rs` now has a
  production caller (verified by grep).
- `src/agents/memory_integration.rs` deleted: its `commit_memory_deltas` was
  a 22-line log-and-drop stub whose real intent already exists as
  `persist_memory_deltas`; `trigger_memory_hooks` only logged. No callers,
  no loss.
- Module docstrings now describe actual behavior; the "Hermies/OpenClaw-style
  persistent identity" aspiration is gone.
- `run_unified_agent` / `run_agent` / `handle_agent_run` are `&mut` (memory
  mutation is the honest signature); 3 new continuity unit tests green.
  Suite 257 passed, 0 failed. Zero new warnings.

### Added — Nightly consolidation: the real short-term → long-term path (task 205, 2026-10-05)

- `consolidate_memory_inner` now returns `ConsolidationReport
  {chunks_drained, episodes_archived, facts_added}`. Each drained
  working-memory chunk is LLM-summarized, date-tagged `[YYYY-MM-DD]`, and
  persisted as typed continuity records — an `ImportantOutcome` and a
  `LessonLearned` per chunk — into episodic (`OUTCOME [goal] …`) AND vector
  memory (two facts: outcome + lesson), both disk-backed. `consolidated.md`
  stays as the human-readable morning surface.
- The `LessonLearned` / `ImportantOutcome` types finally have callers. Along
  the way discovered `src/agents/memory_continuity.rs` was never declared in
  `agents/mod.rs` (orphaned file, not just dead code) — declared it; the
  module compiles cleanly.
- Deleted the old dead-end path: `run_manifest_routines` (drain → summarize
  → `push_summary` back into working memory, summaries never reaching
  long-term storage) is gone, function and heartbeat call site, with a
  regression test asserting working memory never receives the summary back.
- The nightly report now carries the counts: "consolidated N
  working-memory chunks -> M episodes archived, K vector facts added".
- 205.3 verified without duplicating 201/202: `nightly_memory_consolidation`
  was already seeded in the cron registry at `daily 02:00` (inside the 1–7am
  window) and dispatched via `handle_routine_run`; added a registry test
  pinning that registration. Suite 254 passed, 0 failed. Zero new warnings.

### Added — Disk persistence + real embeddings (task 204, 2026-10-05)

- Episodic memory is disk-backed: every `record()` appends one JSON line to
  `episodes.jsonl` (fsync per record; low-frequency writes, no batching to
  lose). Reload replays the log, skips corrupt lines with a warning, keeps the
  in-memory window bounded while the file stays the full archive. Path
  overridable via `HELIX_EPISODES_FILE`.
- Vector memory is disk-backed: facts persist to `vector_facts.json` through a
  shared `durable_write_json` helper (temp file + fsync + atomic rename +
  directory fsync — a crash leaves old or new, never torn). Path overridable
  via `HELIX_VECTOR_FACTS_FILE`.
- Real embeddings via local Ollama: new `Embedder` trait + `OllamaEmbedder`
  posting to `{HELIX_EMBED_URL}/api/embed` (default John's desktop Ollama
  `192.168.1.196:11434`, `HELIX_EMBED_MODEL` default `nomic-embed-text`),
  10s/3s timeouts, 5-minute degraded backoff, one warning per outage.
- The dummy `text.len()` embedding vector is gone. When the embedder is
  degraded, search falls back to honest keyword scoring (terms > 1 char,
  score > 0, stable sort) and says so in the logs — never silent garbage.
- `MemoryManager::new` is now fully disk-backed (beliefs + facts + episodes);
  `search_facts` is `&mut`; the `memory_continuity` call site updated.
  New tests: semantic ranking (fails under dummy vectors), keyword
  degradation, unreachable-endpoint backoff, disk round-trips, corrupt-file
  tolerance. Suite 253 passed, 0 failed. Zero new warnings.

### Added — Unified beliefs: single source of truth (task 203, 2026-10-05)

- Killed the beliefs split-brain: `beliefs.json` is now the one store.
  `src/memory/manager.rs` owns `BELIEFS_FILE` plus `load/save_beliefs[_from/_to]`
  helpers (one pretty-JSON serialization); `MemoryManager::new` loads it at
  construction (missing/corrupt -> empty + warning, never panics);
  `set_belief` is write-through; new `refresh_beliefs()`.
- `Cpu::enhance_prompt_with_memory` is now `&mut` and refreshes beliefs from
  disk before injection, so beliefs written via the `set_belief` tool always
  reach prompts.
- `get_beliefs`/`set_belief` tools delegate to the shared helpers (local
  `BELIEFS_FILE` const removed). 5 new tests green; suite 248 passed.

### Added — Nightly maintenance + heartbeat de-drift (task 202, 2026-10-05)

- hartbeat.md End-of-Day jobs are real now: `nightly_memory_consolidation`
  drains working memory, LLM-summarizes, and persists to
  `<runtime>/memory/consolidated.md` + episodic record (1–7am window-gated);
  `error_log_review` reports real findings (counts, recent lines) to the
  nightly report and fires a bus alert on errors.
- Nighttime code-write autonomy (granted 2026-10-04): all four guardrails
  enforced in code (window / logged-errors-only / `[shell] enabled` /
  nightly-report logging); armed path runs a bounded ReAct fix loop with
  test-verification and revert-on-red; propose-only otherwise.
- De-drift: `handle_heartbeat` is tick bookkeeping + real manifest routines
  only. Removed HyEvo cycles, reasoning-engine lifecycle, self-repair call,
  and the old 2am block from the heartbeat path; deleted `evolve_manifest`
  and the 'evolve constitution' branch. 10 new tests green.

### Added — Real cron registry (task 201, 2026-10-05)

- Deleted the dead `src/cron/cron_handler.rs` (never compiled, no `mod cron`;
  had a missing chrono import, `hour() % 1 == 0`, and a 5s `agent_run` spam
  loop). New `src/cron/registry.rs`: named `Routine`s with schedules
  (`daily HH:MM` local, `every <n>s|m|h|d`), enabled state, persisted as
  `routines.json` under the runtime dir.
- Scheduler task fires due jobs on their own schedules (60s housekeeping
  loop, no global 5s tick), publishing `routine_run` to the CPU over the bus.
  Seeded with hartbeat.md jobs: nightly memory consolidation (02:00, inside
  the 1–7am window) and error-log review (06:30).
- Routines WS API implements task 197's contract exactly: `routines_list` →
  `routines`, `routine_toggle` → `routine_status`; schedule served as display
  text ("Every day at 2:00 AM"). Registry shared between scheduler and web
  server; toggles persist without restart. 12 new unit tests green.

### Added — Heartbeat observability (task 200, 2026-10-05)

- Every beat writes `heartbeat.md` (tick, timestamp, uptime, mode, errors) to
  the runtime dir and appends to `logs/hartbeat_log.md` — the existing
  `/logs/hartbeat` route finally serves real content.
- New `heartbeat_status` WS request (direct reply with tick,
  `last_beat_ms_ago`, uptime, errors) plus a broadcast every 10th beat;
  `heartbeat_missed` bus alert when the beat file goes stale (> 3x interval)
  or missing, with a loud stderr log from the watchdog task.
- Log paths resolve through the loaded config's directory (task 188), never
  CWD-relative. Writer/reader share `format_heartbeat_md` /
  `parse_heartbeat_md` in utils; 6 new unit tests green.

### Added — Heartbeat wired: the bot ticks now (task 199, 2026-10-05)

- `run_helix` constructs the live `Cpu<OllamaLlm>` (memory, skills, Ollama
  router from `[[ollama]]` backends, HyEvo, reasoning config, manifest
  resolved via the runtime dir) and spawns a tokio tick task every
  `[heartbeat] interval_seconds` (default 300) driving `handle_heartbeat`.
  John's intent: the heartbeat is the drumbeat that keeps the bot busy moving
  through jobs, not a liveness ping.
- Deleted the dead duplicate `src/cpu/cpu.rs` and the uncalled
  `TimeScheduler` placeholder — exactly one tick path remains.
- Fix: `HyEvoIntegration` held a `std::sync::MutexGuard` across `.await`
  (rejected by `tokio::spawn`, latent executor deadlock) — converted to
  `tokio::sync::Mutex`.
- Tests: 3 new `cpu::heartbeat_tests` green; full suite 215/0.

### Added — Helix brand icon in web UI (task 209, 2026-10-05)

- John's chosen mark (`helix-icon-c.png`, bot-face helix) wired into the web UI:
  favicon via `<link rel="icon">` and a 38px brand mark beside the Helix title
  in the sidebar. Both inlined as base64 data URIs (favicon 48px, mark 96px —
  the UI is a single `include_str!` file with no static-asset routes).

### Added — SSH server into Helix (task 192, 2026-10-04)

- **SSH server** (russh 0.63.3, `aws-lc-rs` only backend): `src/ssh/` module
  (server + russh handler), `src/config/ssh.rs` (`[ssh]`: `enabled = false`
  default, `bind = "127.0.0.1"` default, `port = 2222`, `authorized_keys` /
  `host_key` defaulting under `<config-dir>/ssh/`, `max_sessions = 8`,
  `idle_timeout_secs = 900`, `auth_rate_limit_per_minute = 10`). Started
  from main.rs alongside the web server.
- **Threat model** (in code comments + `docs/ssh.md`): key-only auth (never
  passwords/keyboard-interactive); dedicated Helix-managed authorized_keys
  (compared by key material, comments ignored — never
  `~/.ssh/authorized_keys`); fail closed with no keys AND when `[shell]` is
  off; bind loopback by default, `0.0.0.0` warns loudly, bind only from the
  config file (no tool/chat surface — prompt-injection guard); per-IP auth
  rate limiting; attempts logged with fingerprint, never key material.
- **One shell path:** exec channels run through `run_shell` (task 191) via
  `spawn_blocking` — exit 124 timeout / 126 policy refusal. v1 is exec-only;
  `shell_request`/PTY rejected with a clear message (follow-up).
- **Verification:** 13 ssh unit tests + live OpenSSH interop (exec works,
  `rm -rf /` blocked over SSH proving one path, wrong key rejected,
  no-keys fail-closed without binding). Full suite 212 passed, 0 failed.
  Zero new warnings. Manual R630 check left for John.

### Added — Local shell tool `run_shell` (task 191, 2026-10-04)

- **New `run_shell` tool** (`src/tools/shell_tool.rs`, registered in
  `execute()` + `tool_definitions()`): runs commands via `/bin/sh -c` on the
  Helix server. Schema: `command` (required), `timeout_secs` (optional),
  `workdir` (optional).
- **Policy layers:** `[shell] enabled = false` default gate (new
  `src/config/shell.rs`: `enabled`, `default_timeout_secs = 60`,
  `workdir = "."`, `allow_absolute_paths = false`,
  `max_output_bytes = 32768`); the shared two-layer
  `validate_shell_command` from task 189 (argv[0] analysis + substring
  denylist — implemented once, reused, not copied); timeouts (1–3600s)
  SIGKILL the whole process group (`setpgid` + `killpg`, `libc 0.2` added);
  stdout/stderr drained on threads and truncated at the cap with notice;
  stdin is `/dev/null` (no TTY — interactive prompts fail fast); workdir
  confined under the base (absolute/`..` escapes rejected unless allowed).
- **Defense-in-depth** (as in grok-cli): the denylist sits behind the agent's
  judgment, never as the sole protection — documented in
  `docs/configuration.md`.
- **Unblocks task 192:** `shell_tool::is_enabled()` is the SSH server's
  fail-closed check.
- **Verification:** 11 shell_tool tests + 2 config tests + 6 denylist tests;
  full suite 199 passed, 0 failed. Zero new warnings.

### Added — Real A2A v1.0 (task 190, 2026-10-04)

- **A2A JSON-RPC on the web server:** `GET /.well-known/agent-card.json`
  (public discovery) and `POST /a2a` (token-gated via the existing
  `require_web_token` middleware — 401 without/wrong token). Methods:
  `message/send`, `message/stream` (SSE), `tasks/get`, `tasks/cancel`, and
  `tasks/list` (Helix extension). Both v1.0 (`message/send`) and v0.3-era
  (`message:send`) spellings accepted; `contextId`/`sessionId` aliases
  accepted.
- **Task lifecycle** (`src/a2a/`): `submitted → working →
  completed | failed | canceled`, terminal states sticky, worker abort on
  cancel. Tasks execute on the same `HelixAcpAgent`/`RuntimeLoop` as ACP —
  one agent loop, two protocol surfaces. Completed tasks carry the agent's
  final message as an artifact plus full history. `max_steps` from `[acp]`.
- **Docs:** `docs/a2a.md` with curl examples.
- **Fix (touched from task 189):** `AgentRunResult.errored` was always true —
  `AgentState::halt()` sets `last_error` on *normal* completion too ("Terminal
  step reached"); it now keys on "failed" in the halt reason. Added
  `had_final_answer` (A2A uses it for the completed/failed decision).
- **Verification:** 15 a2a unit tests + 3 web-server route tests (real
  middleware, in-process) + live HTTP run against a standalone axum server:
  card 200, send→get→completed with a real agent artifact, v0.3 aliases,
  `-32601`/`-32001` errors, SSE `submitted→completed`, 401 unauthenticated.
  Full lib suite 186 passed, 0 failed. Zero new warnings.
- **Known pre-existing issue (not caused by this task):** the full Helix
  binary's tokio runtime starves on 2-core machines — `std::sync::mpsc`
  blocking `recv()` inside `tokio::spawn` bus subscribers (CPU forwarder,
  Gemini listener) parks all workers, so *every* web route hangs after
  connect. Unaffected on John's multi-core R630. Flagged for Surveyor 1.

### Changed — Web UI reworked to Grok-style three-column layout (tasks 193–198, 2026-10-04)

- **Layout:** the top-tab Control Panel is now a three-column app — left
  backend sidebar (~280px), center chat, right detail panel (~340px) — in a
  dark near-black theme (#0d1117). All CSS/JS stays inline in
  `src/io/web_server/static/index.html` (the only frontend file, baked via
  `include_str!`: rebuild after every HTML edit).
- **Sidebar:** backend rows built from the real `backends` WS message
  (`{id, label, kind}`); clicking selects the backend shown in the chat
  header and used as the `llm` field of outgoing chat messages. Status dots
  stay neutral — the payload carries no health data, so none is implied.
  Honest "No backends connected" placeholder on empty.
- **Chat:** Grok-style bubbles (user right, agent left), a NEW divider above
  the first post-load message, header with selected-backend label and a live
  connection dot (green/red across the 3s reconnect loop). The `extractText`
  anti-`[object Object]` guards are preserved verbatim.
- **Detail panel:** Details (read-only redacted-config summary — sections +
  key counts + full text, never unredacted — plus collapsible raw editor),
  Library (real `GET /okf/registry` with disabled/failure empty states),
  Computer (backends with kind labels), Logs (relocated `/logs/*` viewer).
- **Routines:** rows with iOS-style toggles built against the documented
  contract (`routines` / `routine_toggle` / `routine_status` messages). The
  backend exposes no routines API yet, so the section ships an honest empty
  state and sends nothing until a real `routines` message arrives.
- **Responsive:** below ~1100px the detail panel becomes a toggleable
  drawer; below ~800px the sidebar does too. Columns scroll independently;
  the chat input stays pinned.
- **Verification:** 57-check node harness driving the shipped JS with
  backend-shaped messages (all pass); `cargo build` clean, zero new
  warnings. Live browser smoke not run here — the dev server binds
  127.0.0.1:8443 but serves zero bytes in this sandbox (pre-request hang,
  unrelated to the HTML change); recommend a visual pass on the R630.
- **Flagged for Ranger 1:** (1) no `tools_list` WS message or `GET /tools`
  endpoint exists — the Computer tab needs `[{name, description}]`;
  (2) no routines backend API — needs a routines registry plus the four
  documented message types (pushing `routines` on WS connect would light up
  the UI with no further frontend changes).

### Fixed — Web editor and MCP client now use the resolved live config path (task 188, 2026-10-04)

- **Wrong-file saves fixed:** `main.rs` resolves the live config from
  `./config.toml` → `/etc/helix/config.toml` →
  `/usr/local/etc/helix/config.toml` (first-exists-wins), but the web config
  editor and the MCP client hardcoded a CWD-relative `"config.toml"`. Web
  saves landed in the wrong file and the `[mcp]` section was invisible
  whenever Helix ran off `/etc/helix/config.toml`. The resolved path is now
  threaded through: `start_web_server` takes a `config_path` argument stored
  in `AppState`, and both the redaction-restore merge-base read and the save
  write use it; `mcp_client` gained a set-once `set_config_path()` accessor
  called from `main.rs` right after config resolution.
- **No-config fallback:** when no config file exists anywhere, the web editor
  falls back to writing `./config.toml` (previous behavior) — never to a
  file literally named `"none"`. The MCP client falls back to `./config.toml`
  when no path was set (keeps unit tests and non-main entry points working).
- **Dual-deploy drift no longer silent:** the installer keeps deploying
  `config.toml`/`system_manifest.md` to both `<primary>/config.toml` and
  `/etc/helix/config.toml` (compatibility), with `<primary>` documented as
  canonical/live (service `WorkingDirectory`) and `/etc/helix` as a
  byte-identical fallback. New safeguards: (1) the installer backs up any
  existing deployed copy to `<name>.bak-<unix-epoch>` before overwriting and
  prints the backup path — reinstalls no longer destroy live web-UI edits;
  (2) the installer's verify step now checks the two copies are byte-identical
  and warns loudly on divergence; (3) `main.rs` logs a startup warning when
  both `./config.toml` and `/etc/helix/config.toml` exist and differ.
- **Tests:** 2 new unit tests for the MCP path loading
  (`load_config_from` round-trip at an arbitrary path; missing file yields the
  disabled default). `cargo test --lib`: 140 passed, 0 failed, 4 ignored
  (live-service tests). No new warnings introduced (all remaining warnings
  are on lines predating this change). Note: the `installer` bin previously
  failed to compile outside the dev machine on
  `include_str!("../../.grok/docs/system_manifest_root.md")` (gitignored,
  dev-machine-only); a local stub was created purely for compile verification
  and is not committed.
- **Manual verification still recommended on the server:** with only
  `/etc/helix/config.toml` present, confirm a web save lands there and
  `[mcp]` is picked up; with no config anywhere, confirm startup doesn't
  panic and a web save falls back to `./config.toml`.

### Fixed — Upstream build break, missing module declarations (task 187, 2026-10-04)

- **Build break repaired:** `cargo check`/`cargo test` failed with
  `unresolved import crate::router` and `crate::agents::plan_tree` — the
  `router/` and `agents/plan_tree.rs` modules existed on disk but were never
  wired into the module tree (declarations lost in the merge chain). Added
  `pub mod router;` to `src/lib.rs`, `pub mod plan_tree;` to
  `src/agents/mod.rs`, and `mod router;` to `src/main.rs` (the binary
  re-declares the module tree privately).
- **Restored missing dependencies:** the router subtree needs `regex 1`
  (complexity scoring) and `notify 8` (config hot-reload file watcher);
  both were absent from `Cargo.toml` — added.
- **Warning cleanup in newly compiled code:** removed 9 unused-import /
  unused-variable warnings surfaced when the router subtree first compiled
  (all in `src/router/*` and `src/agents/plan_tree.rs`; test-only imports
  moved into the `#[cfg(test)]` module). The Ranger 1 security fix bundle
  was not touched.
- **Result:** `cargo check --lib --bin helix --bin test_ollama` exits 0 with
  no new warnings; `cargo test --lib`: 138 passed, 0 failed, 4 ignored
  (ignored are live Ollama / live MCP tests needing external services).
  (Update 2026-10-04, task 188: the `installer` bin's pre-existing
  `include_str!` of the gitignored, dev-machine-only
  `.grok/docs/system_manifest_root.md` was worked around with a local,
  uncommitted stub purely for compile verification; the installer bin now
  builds and its config-deploy logic was hardened — see the task 188 entry.)

### Security — Code-reviewer follow-up fixes (2026-10-04)

- **Broadcast config leak closed (task 183 follow-up):** auth replies and the
  unredacted config are no longer sent over the broadcast channel (every WS
  client receives broadcast — one client's auth leaked the full config to
  all). Each connection now gets a dedicated per-connection channel; the
  forwarder task `select!`s on both broadcast and the direct channel.
- **Constant-time token comparison:** WS `auth` and HTTP bearer/X-Web-Token
  checks now use `subtle::ConstantTimeEq` instead of `==` (new direct
  dependency `subtle 2.6.1`).
- **Redactor hardened:** `is_secret_key` now also matches bare `key` and
  `credential` fragments, so hyphenated forms like `api-key` are redacted
  (over-redaction of e.g. "monkey" accepted as harmless).
- **Tool-pair orphans (task 179 follow-up):** `cap_history` now drops leading
  `tool`-role messages left without their assistant `tool_call` after the
  drain.
- **Panic-proof test cleanup:** `file_tools` traversal tests use a scope
  guard so probe files/symlinks are removed even when an assertion panics.

### Security — Code-review fixes (H9–H13, 2026-10-03)

**Web server hardening (tasks 183, 184):**
- `src/io/web_server/mod.rs`:
  - Server now binds `127.0.0.1` by default (new `[web] bind` option; was hardcoded `0.0.0.0`).
  - Removed `CorsLayer::permissive()` — the UI is served same-origin, so cross-origin access was pure attack surface.
  - New optional shared web auth token (`[web] auth_token`, or `HELIX_WEB_TOKEN` env var which overrides). When set:
    - WS clients must send `{"type":"auth","token":"..."}` before `config_save`/`manifest_save` are accepted (else rejected with an error status); the full config is pushed only after successful auth.
    - HTTP `POST /logs/chat/clear`, `/logs/error/clear`, `/okf/reload`, `/okf/reload/force`, `/okf/manifest` require `Authorization: Bearer <token>` or `X-Web-Token` (401 otherwise).
  - `/okf/webhook` intentionally left open (server-to-server; needs a shared webhook secret to gate — follow-up).
  - Loud startup warnings when no token is set, and when bound to a non-loopback address without a token.
  - WS clients now receive a *redacted* config rendering (secret-looking keys masked as `***REDACTED***`, line-based so comments/formatting survive); `config_save` restores untouched placeholders from the on-disk config before validating/writing and refuses the save if any placeholder survives unmatched.

**Path traversal (task 185):**
- `src/tools/file_tools.rs`: new `resolve_inside()` (canonicalize + prefix check). `read_log` is now constrained under `<cwd>/logs` (defeats `logs/../../config.toml` and symlink escapes); `repo_read`/`repo_grep` constrained to the repo root (defeats `../../.ssh/id_rsa`, absolute-path and symlink escapes). `repo_grep` verified as a no-FS-access placeholder — nothing to constrain. Note: `scan_directory` still accepts absolute paths (same bug class, out of scope — recommend a follow-up task).

### Performance — Code-review fixes (H1–H4, 2026-10-03)

- **H1 (task 177):** `src/io/ollama/mod.rs` — single process-wide `reqwest::Client` via `OnceLock` (`shared_client()`); health checks, model checks, and chat turns now share one connection pool instead of paying fresh TCP handshakes per request.
- **H2 (task 178):** health + model checks collapsed into one cached `/api/tags` probe (60s TTL, keyed by base URL); the 2–3 sequential round trips per turn are now one amortized request. Cache is invalidated when the retry loop exhausts so the next turn re-validates after a real failure.
- **H3 (task 179):** `call_ollama_tools` now enforces a sliding-window history cap (original user prompt + 20 most recent messages) and truncates each tool result to 8,000 chars (char-boundary safe) before feeding it back to the model — per-turn token burn is bounded.
- **H4 (task 180):** static Ollama tool schemas (~25 `json!` literals) built once via `OnceLock` in `src/tools/mod.rs`; `tool_definitions()` clones the cached vec and merges dynamic OKF defs on top per call.

### Fixed — Code-review correctness (H14, 2026-10-03)

- **H14 (task 186.3):** byte-slice panics on non-char-boundaries fixed via new `crate::utils::truncate_str`/`tail_str` helpers: Ollama `result_preview`, web-server chat previews, `read_log` tail truncation, and the same bug class in `src/main.rs` log previews.

### Deferred

- **Tasks 181, 182, 186.1, 186.2, 186.4 (MCP):** `src/mcp_client/` does not exist in this checkout (it arrived in merge `7751f3d`, ahead of `origin/PreRelese`). Findings were verified against `7751f3d` via `git show`; precise fixes are written into each task's details in `.zed/task_list.json`. Apply after pulling `origin/PreRelese`.

### Added — ReasoningEngine Integration (B1–B14)

**Author:** AI Assistant — triggered by user "Cobble"

Completed full integration of the `ReasoningEngine` into the `Cpu<L>`.

#### New Reasoning Features

- **Core reasoning loop** (`run_reasoning_cycle`): hypothesis → plan → execute with self-correction.
- **Lifecycle control**: `start_reasoning`, `stop_reasoning`, `reset_reasoning`, `change_goal`.
- **Observability**:
  - `reasoning_metrics()` — goal, phase, hypothesis count, plan steps, correction cycles.
  - `reasoning_health_check()` — progress detection.
  - `reasoning_status()` — combined paused + metrics + health snapshot.
  - `reasoning_summary()` and `reasoning_trace()` for debug/UI.
- **Pause/Resume**: `pause_reasoning()`, `resume_reasoning()`, `is_reasoning_paused()`.
- **Manual trigger**: `force_next_reasoning_step()`.
- **Bus integration**:
  - Periodic publishing of `reasoning_metrics` every 40 ticks.
  - `reasoning_command` messages (`pause`/`resume`/`reset`/`force_step`).
- **Self-healing**: Reasoning health is now checked inside `self_repair()` with optional auto-reset.
- **Heartbeat integration**: Auto-starts reasoning engine; runs cycles every 20 ticks (respecting pause flag).

#### Files Changed

- `src/cpu/mod.rs` — Added all reasoning control + observation methods.
- `src/cpu/state.rs` — Added `reasoning_paused` flag to `AgentState`.
- `src/reasoning/engine.rs` — Core engine (already present).

#### Documentation

- Added Reasoning API summary comment block in `cpu/mod.rs`.
- Updated `CHANGELOG.md`, `readme.md`, and `flow_map.md`.

**Result**: The agent now has a first-class, observable, controllable reasoning layer that can be driven from the bus or internally via heartbeat.

---

## [Unreleased]

### Audit — Code vs Task List Verification for Tasks 1–59 (2026-04-24)

**Author:** AI Assistant (Claude Sonnet 4.6) — triggered by user "Cobble"

Every task 1–59 was cross-checked against actual source code in `src/`. Task
statuses in `task_list.json` were corrected where the code did not match the
claimed status.

#### Status corrections applied to `task_list.json`

| Task | Title                      | Was       | Now           | Reason                                             |
| ---- | -------------------------- | --------- | ------------- | -------------------------------------------------- |
| 2    | Vector Memory Search       | `done`    | `in_progress` | `dummy_embed()` placeholder — no real embeddings   |
| 3    | Docker & Systemd Deploy    | `done`    | `pending`     | No `Dockerfile` or `.service` file exists          |
| 4    | Web Auth & Polish          | `done`    | `pending`     | Zero JWT / auth code in web server                 |
| 5    | Planning Loop              | `done`    | `in_progress` | Executor stubs only — no real planning logic       |
| 7    | Logging instructions.rs    | `done`    | `in_progress` | Only 1 log call found; rest are comments           |
| 16   | Node-level Mutations       | `done`    | `in_progress` | `replace_node()` function absent                   |
| 31   | Workflow Execution in CPU  | `pending` | `done`        | Fully implemented in `cpu/` + `integration.rs`     |
| 47   | Memory-Aware Prompting     | `done`    | `in_progress` | Subtasks 47.2 (vector) & 47.3 (episodic) not wired |
| 54   | Agent Personality Profiles | `done`    | `in_progress` | Single hardcoded `"neutral"` string, no profiles   |

#### Final tally for tasks 1–59

| Verdict        | Count | IDs                                                                                                              |
| -------------- | ----- | ---------------------------------------------------------------------------------------------------------------- |
| ✅ done        | 39    | 1,6,8,9,10,11,12,13,14,15,17,18,19,20,21,22,23,24,25,26,27,31,41,42,43,44,45,46,48,49,50,51,52,53,55,56,57,58,59 |
| ⚠️ in_progress | 6     | 2, 5, 7, 16, 47, 54                                                                                              |
| ❌ pending     | 13    | 3, 4, 28, 29, 30, 32, 33, 34, 35, 36, 37, 38, 39                                                                 |
| ⏸️ deferred    | 1     | 40                                                                                                               |

Full detail report saved to `Doc's/AiSummary/task_audit_1_59.md`.

---

### Fixed — `.zed/task_list.json` Full Renumber from ID 1 (2026-04-24)

**Author:** AI Assistant (Claude Sonnet 4.6) — triggered by user "Cobble"

#### Problem

The task list started at ID 31 (tasks 1–30 were from a prior phase and had
been removed). This caused Grok-CLI to miscalculate task positions when
navigating by sequential index, producing the wrong task for a given ID.

#### Changes Made

All 75 tasks were renumbered sequentially starting at **ID 1**. Every
reference throughout the file was updated accordingly:

- **75 top-level task `id` fields** renumbered (old 31–105 → new 1–75)
- **All top-level `dependencies` arrays** remapped to new IDs
  - 12 external dependency references to old tasks 1–30 (not in the file,
    all already `"done"`) were dropped cleanly
- **222 subtask `id` strings** remapped (e.g. `"90.1"` → `"60.1"`)
- **28 subtask `dependencies` groups** remapped

#### Key ID mapping (notable tasks)

| Old ID | New ID | Task Title                                         |
| ------ | ------ | -------------------------------------------------- |
| 31     | 1      | LLM Tool Calling (Ollama Functions)                |
| 70     | 40     | Reserved / Removed Task (stub)                     |
| 85     | 55     | Self-Repair Routines                               |
| 90     | 60     | Design Reasoning Protocol Layer (RPL) Architecture |
| 105    | 75     | End-to-End Reasoning Engine CPU Integration Tests  |

#### Result

- JSON validates cleanly — zero parse errors
- IDs 1–75, perfectly sequential, no gaps
- All subtask IDs and dependency references consistent and correct

---

### Fixed — `.zed/task_list.json` Audit and Structural Repair (2026-04-24)

**Author:** AI Assistant (Claude Sonnet 4.6) — triggered by user "Cobble"

#### Problem

Grok-CLI was confusing task IDs when navigating the task list. Asking for
"task 90" would execute a different task, and searching by title
`"Design Reasoning Protocol Layer (RPL) Architecture"` returned the wrong
task number (85 instead of 90).

#### Root Causes Found and Fixed

1. **10 wrong subtask dependency IDs in tasks 90–96** (critical)
   All subtask-level `dependencies` fields inside tasks 90 through 96 were
   offset by exactly 54, pointing to completely unrelated tasks:
   - `task 90.2` had `"dependencies": [36.1]` → fixed to `[90.1]`
   - `task 90.3` had `"dependencies": [36.2]` → fixed to `[90.2]`
   - `task 91.2` had `"dependencies": [37.1]` → fixed to `[91.1]`
   - `task 91.3` had `"dependencies": [37.2]` → fixed to `[91.2]`
   - `task 92.2` had `"dependencies": [38.1]` → fixed to `[92.1]`
   - `task 92.3` had `"dependencies": [38.2]` → fixed to `[92.2]`
   - `task 93.2` had `"dependencies": [39.1]` → fixed to `[93.1]`
   - `task 94.2` had `"dependencies": [40.1]` → fixed to `[94.1]`
   - `task 95.2` had `"dependencies": [41.1]` → fixed to `[95.1]`
   - `task 96.2` had `"dependencies": [42.1]` → fixed to `[96.1]`

2. **Inconsistent subtask ID types** (structural)
   Subtask IDs were a mix of JSON numbers (`31.1`, `90.2`) and strings
   (`"42.10"`, `"87.11"`). JSON numbers cannot distinguish `42.10` from
   `42.1` (they are equal as floats), making `.10+` subtasks unreliable.
   All 215 subtask IDs and 28 subtask dependency references were normalised
   to strings consistently.

3. **Missing task ID 70** (gap in sequence)
   Task 70 was absent, creating a gap between task 69 and task 71. A
   `"deferred"` stub entry was inserted to maintain sequential numbering
   and prevent any array-position-based tool from miscounting tasks.

#### Result

- JSON validates cleanly with no parse errors
- 75 tasks, IDs 31–105, no gaps
- All subtask IDs are strings
- All subtask dependencies reference the correct parent task

---

## [0.6.0] - 2026-05-14

### Added — Ollama Model Preload and Keep-Alive (Task #89)

**Author:** AI Assistant (Claude Sonnet 4.6) — triggered by user "Cobble"

#### New `src/io/ollama/keepalive.rs` module

Standalone keepalive module added to the Ollama IO sub-system:

- **`preload_model(base_url, model)`** — async fn that sends a dummy
  `POST /api/generate` (empty prompt, `num_predict: 0`, `stream: false`) at Helix
  startup to force Ollama to load the model into GPU/CPU memory before the first
  real user query, eliminating cold-start latency.

- **`spawn_keepalive_task(base_url, model, interval_secs)`** — spawns a
  long-lived background Tokio task that fires a heartbeat `POST /api/generate`
  (with `keep_alive: "1h"`) every N seconds (default: 240 s, below Ollama's
  default 5-minute eviction timeout). The task never panics.

- **`read_preload_flag()`** / **`read_keep_alive_secs()`** — env-var config
  helpers with safe defaults.

- **Network resilience (Starlink policy)**: every HTTP call is wrapped in
  `tokio::time::timeout` and retried up to 3 times with exponential-backoff
  delays (2 s → 4 s → 8 s, capped at 30 s). Failures are warnings only —
  Helix never crashes on keepalive errors.

- **10 unit tests** covering config-flag defaults, custom values, garbage input
  fallback, unreachable-host resilience, and zero-interval noop.

#### `src/main.rs` updated

- Fixed duplicate `log`/`tracing` import (compilation error).
- Added `preload_model()` call at startup, guarded by `OLLAMA_PRELOAD` env flag.
- Added `spawn_keepalive_task()` call to start the background heartbeat.
- Reads `OLLAMA_URL`, `OLLAMA_MODEL`, `OLLAMA_PRELOAD`, `OLLAMA_KEEP_ALIVE_SECS`
  from `.env`.

#### `Cargo.toml` updated

- Added `reqwest = { version = "0.11", features = ["json"] }`.
- Added `serde_json = "1.0"`.
- Added `"time"` to Tokio features (required for `tokio::time::sleep`).

#### `config.toml` updated

- Added `[ollama_keepalive]` section documenting `preload` and `interval_secs`.
- Added comments for all related `.env` overrides.

---

## [0.5.0] - 2026-04-12

### Added — Tools and Skills System (Task #88)

**Author:** AI Assistant (Claude Sonnet 4.6) — triggered by user "Cobble"

#### New `src/tools/` module — single source of truth

- **`src/tools/mod.rs`** — Central `execute(name, args) -> String` dispatcher
  and `tool_definitions() -> Value` that returns the Ollama-compatible JSON
  schema array. Both the Ollama agentic loop and the CPU `SkillRegistry` now
  delegate here, so adding a tool in one place automatically makes it available
  everywhere.

- **`src/tools/file_tools.rs`** — File-based tools:
  - `read_log(args)` — reads the tail (≤ 2 000 chars) of any `logs/` file;
    path-traversal guard rejects anything outside `logs/`.
  - `write_note(args)` — sanitises the title into a safe filename, creates
    `notes/`, writes `notes/<title>.md`.
  - `read_note(args)` — reads `notes/<title>.md`; on miss lists available notes.
  - `list_notes()` — scans `notes/` for `.md` files, returns sorted bullet list.

- **`src/tools/system_tools.rs`** — System and memory tools:
  - `send_email(args)` — logs intent and appends to `logs/email_outbox.md`;
    marked TODO for `lettre`/webhook SMTP integration.
  - `system_status()` — reports Unix timestamp, sizes of 3 log files, note
    count, and belief count.
  - `list_tools()` — human-readable catalogue of all tools plus slash commands.
  - `get_beliefs()` — reads and pretty-prints `beliefs.json`.
  - `set_belief(args)` — merges one key/value into `beliefs.json` and persists
    it across restarts.

#### Ollama tool-calling wired to shared tools

- `call_ollama_tools` now calls `crate::tools::execute` instead of the old
  hardcoded `execute_tool` match.
- `default_tools()` and `execute_tool()` are now thin wrappers over the shared
  module (kept for backward-compat with tests).
- `call_ollama` wrapper gains a `bus: &Arc<Bus>` parameter so tool executions
  can be published to the bus.
- **Tool-call events** — every time Ollama invokes a tool, a `tool_call` message
  is published to `"web_interface"` with the tool name, args, and a 200-char
  result preview. The web UI renders these with a distinct gold ⚙ style and
  dark-background `<code>` for the args.

#### `SkillRegistry` fully populated

- All 9 tools registered by name; each closure delegates to
  `crate::tools::execute(name, params)` wrapped in `NodeResult::Text(...)`.
- New `pub fn list_names() -> Vec<&str>` method returns a sorted list of all
  registered skill names.

#### Slash commands in the chat UI

Seven slash commands handled directly in `web_server` without hitting an LLM:

| Command          | Action                                           |
| ---------------- | ------------------------------------------------ |
| `/status`        | `system_status` tool                             |
| `/tools`         | `list_tools` tool                                |
| `/notes`         | `list_notes` tool                                |
| `/note <title>`  | `read_note` tool                                 |
| `/beliefs`       | `get_beliefs` tool                               |
| `/set key=value` | `set_belief` tool                                |
| `/log [file]`    | `read_log` tool (defaults to `logs/chat_log.md`) |
| `/help`          | Lists all slash commands                         |

The web client intercepts messages starting with `/` and sends
`{type: "slash_cmd", cmd}` over WebSocket; the server executes the tool
synchronously and publishes the result back to the chat.

#### CPU memory recording

`Cpu::handle_bus_message` now handles four message types:

- `user_input` — records `"user: <prompt>"` in working memory (existing).
- `chat_request` — records web-UI chat messages in working memory.
- `ollama_response` / `llm_output` — records bot replies (up to 500 chars) in
  working memory so the LLM has conversation context.
- `skill_request` — new: executes a named skill synchronously via
  `crate::tools::execute` and publishes the result to `"web_interface"` with
  type `ollama_response` and `"llm": "skill"`.

#### Test results

`cargo test` — **55 passed, 0 failed, 1 ignored** (live Ollama integration test).
13 new tests cover the tools module: security guard, filename sanitisation,
unknown-tool handling, schema shape, and no-panic guarantees for all tools.

---

## [0.4.0] - 2026-04-12

### Fixed — Chat Round-Trip, Live Logs, and Multi-LLM Routing (Task #87)

**Author:** AI Assistant (Claude Sonnet 4.6) — triggered by user "Cobble"

#### Critical Bug Fixes

- **`&msg.data[..50]` PANIC in `web_server` bus forwarder** — The bus→WebSocket
  bridge thread crashed silently whenever any log message payload was shorter than
  50 bytes (e.g. short startup logs). This killed the forwarder permanently,
  stopping ALL log output and ALL Ollama responses from reaching the UI.
  Fixed with `..50.min(msg.data.len())` guard in both `web_server/mod.rs` and
  `main.rs` log publisher.

- **Broken chat routing — messages silently dropped** — The CPU's
  `handle_bus_message` routed `user_input` to bus channel `"ollama_lan"`, but
  `main.rs` only subscribed a listener on `"ollama"`. Every chat message was
  published to a channel with no subscriber and vanished. Fixed by routing chat
  directly from `web_server` to `"ollama_{name}"` (or `"gemini"`), bypassing the
  CPU for web chat entirely.

- **Wrong Ollama prompt — full JSON blob sent to model** — `handle_ollama_message`
  was passing the raw `message.data` string (e.g.
  `{"type":"chat_request","prompt":"hello","correlation_id":123}`) directly as
  the user prompt to Ollama. The model received a JSON blob instead of the actual
  question. Fixed by extracting the `"prompt"` field from the JSON payload with a
  raw-string fallback.

- **Config struct mismatch in `web_server`** — The local `Config` struct in
  `web_server/mod.rs` had `ollama: OllamaConfig` (single struct) while
  `config.toml` uses `[[ollama]]` (TOML array). Any attempt to validate and save
  the config from the UI always failed with a misleading "Invalid TOML syntax"
  error. Fixed by updating the struct to `ollama: Vec<OllamaConfig>` with an added
  `name: String` field.

- **CPU `route_llm_request` used wrong bus channel names** — `LlmTarget::OllamaLan`
  mapped to `"ollama_lan"` and `LlmTarget::OllamaLocal` to `"ollama_local"`.
  Updated to `"ollama_server"` and `"ollama_local3090"` to match the `name` fields
  in `config.toml`.

#### Architecture Changes

- **Per-backend Ollama handlers in `main.rs`** — Replaced the single monolithic
  Ollama subscriber (listening on `"ollama"`) with a loop over `config.ollama`
  that spawns one independent handler per backend. Each handler subscribes to
  `"ollama_{name}"` (e.g. `"ollama_server"`, `"ollama_local3090"`), performs a
  startup health check, and uses the same sync→async bridge pattern for
  non-blocking concurrent dispatch.

- **`backend_name` parameter added to `handle_ollama_message`** — The response
  published to `"web_interface"` now carries `"llm": backend_name` so the UI can
  display which Ollama instance answered.

- **Dedicated Gemini bus subscriber in `main.rs`** — A new `tokio::spawn` block
  subscribes to the `"gemini"` bus channel and dispatches to
  `handle_gemini_bus_message`.

#### New Features

- **LLM selector in the web UI** — A dynamic row of pill-shaped buttons appears
  above the chat input. Buttons are populated at runtime from the `backends`
  WebSocket message sent on every connection. Currently shows:
  - **Ollama server** (maps to `"ollama_server"` bus channel)
  - **Ollama local3090** (maps to `"ollama_local3090"` bus channel)
  - **Gemini** (maps to `"gemini"` bus channel)

  Selecting a button sets `selectedLlm` in JS; every `sendChat()` call includes
  `llm: selectedLlm` in the WebSocket payload so the server routes to the correct
  backend.

- **Full `llm_gemini/mod.rs` rewrite** — Replaced the old synchronous,
  bus-unaware, hardcoded-key implementation with a production-ready async handler:
  - Reads `GEMINI_API_KEY` from environment (via `dotenvy` / `.env` file).
  - Uses `gemini-2.0-flash` model (upgraded from deprecated `gemini-pro`).
  - `reqwest::Client` with 10 s connect timeout and 60 s request timeout.
  - 3-attempt retry loop with **exponential back-off** (2 s → 4 s → 8 s) for
    Starlink / unstable-connection resilience.
  - Publishes responses directly to `"web_interface"` bus channel (and optionally
    `"cpu"` when a `correlation_id` is present).
  - Error and warning states published to the bus so the UI can show progress.

- **`dotenvy` integration** — Added `dotenvy = "0.15.7"` to `Cargo.toml` and
  `dotenvy::dotenv().ok()` at the top of `main()` so all secrets in `.env`
  (including `GEMINI_API_KEY`) are loaded automatically on startup.

- **`AppState.backends_json`** — The web server now parses `config.toml` at
  startup to build a JSON array of available LLM backends and stores it in
  `AppState`. On every new WebSocket connection the server sends a `backends`
  message; the JS client uses this to build the LLM selector buttons dynamically.

- **Dark-theme UI redesign** — Full CSS overhaul of `MAIN_HTML`:
  - Dark navy/teal colour scheme.
  - WebSocket connection status dot (green = connected, red = disconnected).
  - Log entries now include wall-clock timestamps.
  - **Clear** button on the Logs tab.
  - `WS_URL` uses `location.hostname` instead of hardcoded `localhost` so the UI
    works from any host on the LAN.
  - Smooth scroll on new chat messages and log entries.
  - Chat messages show sender class (`you`, `helix`, `error-msg`, `warning-msg`) for
    colour-coded display.

---

## [0.3.0] - 2025 (prior work)

### Added

- `[[ollama]]` array support in `config.toml` — multiple Ollama instances
  (`server` at `192.168.1.149`, `local3090` at `192.168.1.196`).
- `OllamaRouter` struct in `main.rs` for routing LLM requests across backends.
- `HyEvoIntegration` and `HyEvoEngine` for self-evolving behaviour.
- `SystemManifest` loaded from `system_manifest.md` at CPU startup.
- `TimeScheduler` for tick-based heartbeat (1 000 ms interval).
- Agentic tool-calling loop in `ollama/mod.rs` — model can call `read_log` and
  `send_email` tools, results fed back in a multi-turn loop (up to 10 rounds).
- `LlmTarget` enum (`OllamaLan`, `OllamaLocal`, `Gemini`, `Grok`) for CPU-level
  LLM routing via bus.
- `check_ollama_health`, `fetch_available_models`, `check_model_exists` helpers
  with timeout and retry.
- HTTPS web server using `axum-server` + `rcgen` self-signed certificates.
- Live log forwarding via `WebLogger` → mpsc → bus → WebSocket broadcast.
- Config and Manifest tabs with save-to-disk functionality.

### Fixed

- Duplicate `use log::{debug, error}` in `cpu/cpu.rs` (dead file, not compiled).

---

## [0.2.0] - 2025 (prior work)

### Added

- `Bus` message-routing system with sync `mpsc` channels and subscription model.
- `Cpu<L>` generic struct with `handle_bus_message`, `handle_heartbeat`,
  `execute_instruction`, `run_hyevo_cycle`.
- `MemoryManager` (working memory + beliefs + long-term store).
- `SkillRegistry` and `HookRegistry`.
- `BayesianClassifier` in `src/bayesian.rs`.
- Terminal I/O module.
- TLS certificate auto-generation.

---

## [0.1.0] - 2025 (initial)

### Added

- Initial project scaffold: `src/main.rs`, `src/bus/`, `src/cpu/`, `src/io/`,
  `src/memory/`, `src/skills/`, `src/utils.rs`.
- `config.toml` with `[bot]`, `[ollama]`, `[web]`, `[heartbeat]` sections.
- `system_manifest.md` system constitution.
- `.gitignore` excluding `/target/`, `.env`, `.zed/`, `*.pdb`, `Cargo.lock`.
- MIT `LICENSE`.
- `PROJECT_LAYOUT.md` and `flow_map.md` architecture documentation.
