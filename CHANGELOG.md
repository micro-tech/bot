# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased]

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