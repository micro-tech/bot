# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased]

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

| Task | Title | Was | Now | Reason |
|------|-------|-----|-----|--------|
| 2 | Vector Memory Search | `done` | `in_progress` | `dummy_embed()` placeholder — no real embeddings |
| 3 | Docker & Systemd Deploy | `done` | `pending` | No `Dockerfile` or `.service` file exists |
| 4 | Web Auth & Polish | `done` | `pending` | Zero JWT / auth code in web server |
| 5 | Planning Loop | `done` | `in_progress` | Executor stubs only — no real planning logic |
| 7 | Logging instructions.rs | `done` | `in_progress` | Only 1 log call found; rest are comments |
| 16 | Node-level Mutations | `done` | `in_progress` | `replace_node()` function absent |
| 31 | Workflow Execution in CPU | `pending` | `done` | Fully implemented in `cpu/` + `integration.rs` |
| 47 | Memory-Aware Prompting | `done` | `in_progress` | Subtasks 47.2 (vector) & 47.3 (episodic) not wired |
| 54 | Agent Personality Profiles | `done` | `in_progress` | Single hardcoded `"neutral"` string, no profiles |

#### Final tally for tasks 1–59

| Verdict | Count | IDs |
|---------|-------|-----|
| ✅ done | 39 | 1,6,8,9,10,11,12,13,14,15,17,18,19,20,21,22,23,24,25,26,27,31,41,42,43,44,45,46,48,49,50,51,52,53,55,56,57,58,59 |
| ⚠️ in_progress | 6 | 2, 5, 7, 16, 47, 54 |
| ❌ pending | 13 | 3, 4, 28, 29, 30, 32, 33, 34, 35, 36, 37, 38, 39 |
| ⏸️ deferred | 1 | 40 |

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

| Old ID | New ID | Task Title |
|--------|--------|------------|
| 31 | 1 | LLM Tool Calling (Ollama Functions) |
| 70 | 40 | Reserved / Removed Task (stub) |
| 85 | 55 | Self-Repair Routines |
| 90 | 60 | Design Reasoning Protocol Layer (RPL) Architecture |
| 105 | 75 | End-to-End Reasoning Engine CPU Integration Tests |

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
  `POST /api/generate` (empty prompt, `num_predict: 0`, `stream: false`) at bot
  startup to force Ollama to load the model into GPU/CPU memory before the first
  real user query, eliminating cold-start latency.

- **`spawn_keepalive_task(base_url, model, interval_secs)`** — spawns a
  long-lived background Tokio task that fires a heartbeat `POST /api/generate`
  (with `keep_alive: "1h"`) every N seconds (default: 240 s, below Ollama's
  default 5-minute eviction timeout).  The task never panics.

- **`read_preload_flag()`** / **`read_keep_alive_secs()`** — env-var config
  helpers with safe defaults.

- **Network resilience (Starlink policy)**: every HTTP call is wrapped in
  `tokio::time::timeout` and retried up to 3 times with exponential-backoff
  delays (2 s → 4 s → 8 s, capped at 30 s).  Failures are warnings only —
  the bot never crashes on keepalive errors.

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
  schema array.  Both the Ollama agentic loop and the CPU `SkillRegistry` now
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
  result preview.  The web UI renders these with a distinct gold ⚙ style and
  dark-background `<code>` for the args.

#### `SkillRegistry` fully populated

- All 9 tools registered by name; each closure delegates to
  `crate::tools::execute(name, params)` wrapped in `NodeResult::Text(...)`.
- New `pub fn list_names() -> Vec<&str>` method returns a sorted list of all
  registered skill names.

#### Slash commands in the chat UI

Seven slash commands handled directly in `web_server` without hitting an LLM:

| Command | Action |
|---|---|
| `/status` | `system_status` tool |
| `/tools` | `list_tools` tool |
| `/notes` | `list_notes` tool |
| `/note <title>` | `read_note` tool |
| `/beliefs` | `get_beliefs` tool |
| `/set key=value` | `set_belief` tool |
| `/log [file]` | `read_log` tool (defaults to `logs/chat_log.md`) |
| `/help` | Lists all slash commands |

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
  - Chat messages show sender class (`you`, `bot`, `error-msg`, `warning-msg`) for
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