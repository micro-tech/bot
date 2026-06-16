# Task Audit Report — Tasks 1–59

**Date:** 2026-04-24
**Author:** AI Assistant (Claude Sonnet 4.6) — triggered by user "Cobble"
**Project:** Grok-CLI (`H:/GitHub/bot`)
**Scope:** Verify every task 1–59 against actual source code in `src/`

---

## Method

Each task was cross-checked by:
1. Reading the relevant source file(s) directly
2. Searching for key function names, structs, and log calls via grep
3. Running `cargo check` to confirm compilation status

Verdicts:
- ✅ **DONE** — fully implemented and confirmed in code
- ⚠️ **IN_PROGRESS** — partially implemented; skeleton or stub exists but feature is incomplete
- ❌ **PENDING** — no implementation found; marked done in task list incorrectly
- ⏸️ **DEFERRED** — intentionally reserved/skipped slot

---

## Results by Task

### Task 1 — LLM Tool Calling (Ollama Functions)
**Status in file:** `done` → **Verdict: ✅ DONE**

`src/io/ollama/mod.rs` contains a full agentic tool-call loop:
- Sends `tools` JSON array to Ollama `/api/chat`
- Checks `msg["tool_calls"]` in response
- Dispatches to `crate::tools::execute()`
- Pushes tool results back and loops up to 10 rounds
- `default_tools()` and `execute_tool()` wrappers present

---

### Task 2 — Vector Memory Search
**Status in file:** `done` → **Verdict: ⚠️ IN_PROGRESS**

`src/memory/vector.rs` has:
- `VectorMemory` struct with `add_fact()` and `search()` ✅
- Cosine similarity ranking ✅
- `dummy_embed()` — **returns `vec![text.len() as f32; 384]`** ❌

The embedding function is a placeholder. All strings of the same length produce
identical embeddings, making semantic search meaningless. A real embedding model
(e.g. sentence-transformers, fastembed, or an Ollama embedding endpoint) was
never wired in.

**Action needed:** Replace `dummy_embed()` with a real embedding call.

---

### Task 3 — Docker & Systemd Deploy
**Status in file:** `done` → **Verdict: ❌ PENDING**

Searched entire project for `Dockerfile*` and `*.service` — **zero matches**.
Neither a Dockerfile nor a systemd unit file exists anywhere in the repository.

**Action needed:** Create `Dockerfile` and `bot.service` in project root.

---

### Task 4 — Web Auth & Polish
**Status in file:** `done` → **Verdict: ❌ PENDING**

`src/io/web_server/mod.rs` has WebSocket + HTTPS (self-signed TLS via `rcgen`)
and CORS, but a grep for `jwt`, `JWT`, `jsonwebtoken`, `bearer`, and `token`
across all web server files returned **zero hits**. No auth middleware or token
validation exists.

**Action needed:** Add `jsonwebtoken` crate and implement JWT middleware on the
axum router.

---

### Task 5 — Planning Loop (Reflection/Decomp)
**Status in file:** `done` → **Verdict: ⚠️ IN_PROGRESS**

No `src/planning/` directory exists. Planning-related code in `src/cpu/`:
- `instructions.rs` defines `PlanNextSteps` and `ReflectOnLastStep` variants ✅
- `scheduler.rs` routes to `PlanNextSteps` in `AgentMode::Planning` ✅
- `executor.rs` handles both variants with only `debug!(…)` and `NodeResult::None` ❌

The HyEvo `reflection.rs` / `engine.rs` provide reflection infrastructure but
it is not wired into the CPU planning path. No decomposition or multi-step
planning loop is actually executed.

**Action needed:** Implement real planning logic in `executor.rs` — decompose
the goal, generate subtasks, and loop execution.

---

### Task 6 — Add Logging to `src/cpu/executor.rs`
**Status in file:** `done` → **Verdict: ✅ DONE**

`use log::debug;` present. `debug!()` on every branch, `error!()` on failures,
and `log_to_file()` calls throughout.

---

### Task 7 — Add Logging to `src/cpu/instructions.rs`
**Status in file:** `done` → **Verdict: ⚠️ IN_PROGRESS**

Only **one** `log::debug!` call found in `from_message()`. All enum and struct
definitions have doc comments saying "add logging" but no actual log calls follow.

**Action needed:** Add `debug!()` / `info!()` calls throughout `instructions.rs`,
particularly in instruction dispatch and state transitions.

---

### Task 8 — Add Logging to `src/cpu/interrupts.rs`
**Status in file:** `done` → **Verdict: ✅ DONE**

`use log::debug;` present. `debug!()` on poll entry, success, and failure paths,
plus `log_to_file()`.

---

### Task 9 — Add Logging to `src/cpu/mod.rs`
**Status in file:** `done` → **Verdict: ✅ DONE**

`use log::debug;` present. Extensive `debug!()`, `error!()`, and `log_to_file()`
calls throughout all methods.

---

### Task 10 — Add Logging to `src/cpu/scheduler.rs`
**Status in file:** `done` → **Verdict: ✅ DONE**

`use log::debug;` present. `debug!()` at entry and exit, `log_to_file()` calls.

---

### Task 11 — Add Logging to `src/cpu/state.rs`
**Status in file:** `done` → **Verdict: ✅ DONE**

`use log::debug;` present. `debug!()` in `set_mode()`, `log_to_file()` in
`set_mode()`, `bump_step()`, and `record_error()`.

---

### Task 12 — Create `hy_evo/` Module
**Status in file:** `done` → **Verdict: ✅ DONE**

`src/hy_evo/` exists with all required files:
`crossover.rs`, `engine.rs`, `genome.rs`, `integration.rs`, `mod.rs`,
`mutation.rs`, `node.rs`, `reflection.rs`, `scoring.rs`, `workflow.rs`

---

### Task 13 — Define Node Enum
**Status in file:** `done` → **Verdict: ✅ DONE**

`src/hy_evo/node.rs` contains full `Node` enum with 7 variants:
`Skill`, `Llm`, `Code`, `MemoryRead`, `MemoryWrite`, `BusPublish`, `Conditional`
plus `ConditionNode`, `NodeResult`, and `NodeMetadata` structs.

---

### Task 14 — Define Workflow Struct
**Status in file:** `done` → **Verdict: ✅ DONE**

`src/hy_evo/workflow.rs` has `Workflow` struct with `id: Uuid`,
`ordered_nodes: Vec<(NodeMetadata, Node)>`, `genome: WorkflowGenome` fields,
`from_genome()` with cycle detection and topological sort, and `execute_node()`.

---

### Task 15 — Implement Serialization
**Status in file:** `done` → **Verdict: ✅ DONE**

`Serialize` + `Deserialize` derived on genome and workflow types. JSON
export/import is available.

---

### Task 16 — Implement Node-Level Mutations
**Status in file:** `done` → **Verdict: ⚠️ IN_PROGRESS**

`src/hy_evo/mutation.rs` has:
- `mutate_add_node()` ✅
- `mutate_remove_node()` ✅
- `mutate_node_params()` (in-place param mutation) ✅
- `mutate_reorder_nodes()` ✅
- `replace_node()` as a discrete named operation ❌ — absent

**Action needed:** Add explicit `replace_node(genome, index, new_node)` function.

---

### Task 17 — Implement Workflow-Level Mutations
**Status in file:** `done` → **Verdict: ✅ DONE**

Edge shuffle, subgraph insert/remove, and metadata mutation all implemented in
`mutation.rs`.

---

### Task 18 — Add Mutation Probability Config
**Status in file:** `done` → **Verdict: ✅ DONE**

`MutationConfig` with configurable weights loaded from config; default values
for `add_node`, `remove_node`, `reorder`, `param_mutation` all present.

---

### Task 19 — Implement Single-Point Crossover
**Status in file:** `done` → **Verdict: ✅ DONE**

`single_point()` in `crossover.rs` — prefix-A + suffix-B split. ✅

---

### Task 20 — Implement Multi-Point Crossover
**Status in file:** `done` → **Verdict: ✅ DONE**

`two_point()` in `crossover.rs` — prefix-A + middle-B + suffix-A. Bonus
`uniform()` crossover also present. `CrossoverEngine::crossover()` selects
probabilistically among all three. ✅

---

### Task 21 — Validate Resulting Workflow
**Status in file:** `done` → **Verdict: ✅ DONE**

`from_genome()` in `workflow.rs` performs cycle detection and topological sort,
rejecting invalid graphs. ✅

---

### Tasks 22–24 — Define & Implement Scoring
**Status in file:** `done` → **Verdict: ✅ DONE**

`src/hy_evo/scoring.rs` — `ScoringEngine::score()` computes a weighted
composite from `latency_ms`, `llm_calls`, `errors`, `success` flag, and
optional `user_score`. `ScoringWeights` is configurable. Score is persisted
back to `genome.score`. ✅

---

### Tasks 25–27 — Implement Reflection
**Status in file:** `done` → **Verdict: ✅ DONE**

`src/hy_evo/reflection.rs` — `ReflectionLlm` trait with `reflect()` and
`evolve_code()`, `ReflectionEngine<L>` struct, reflection results stored as
`ReflectionRecord` with timestamp and bullet-point suggestions. ✅

---

### Task 28 — Implement Evolution Cycle
**Status in file:** `pending` → **Verdict: ⚠️ IN_PROGRESS (confirmed pending)**

`engine.rs::evolve_once()` exists but only calls the LLM for text feedback.
The real genetic steps — **select top N → mutate → crossover → score → replace
worst** — are never executed. `MutationEngine` and `CrossoverEngine` exist but
are not called from the engine.

---

### Task 29 — Add Population Manager
**Status in file:** `pending` → **Verdict: ⚠️ IN_PROGRESS (confirmed pending)**

`HyEvoEngine` holds `population: Vec<WorkflowGenome>` and `best_workflow()`
returns the highest-scored entry. No keep-best-K, no cull-worst-K, and no
replace-worst logic. Population grows unbounded.

---

### Task 30 — Add Workflow Registry
**Status in file:** `pending` → **Verdict: ❌ MISSING (confirmed pending)**

No registry struct, no input-type tagging, and no "retrieve best for input
type" function anywhere in `hy_evo/`.

---

### Task 31 — Add Workflow Execution to CPU
**Status in file:** `pending` → **Verdict: ✅ DONE** ← *status corrected*

`CpuExecutorImpl::execute_workflow()` is fully implemented in `src/cpu/`:
builds a `WorkflowContext`, iterates `workflow.ordered_nodes`, dispatches each
node (skill / LLM / memory / bus), accumulates `ExecutionMetrics`, and returns
them. `integration.rs::run_and_evolve()` completes the pipeline.

---

### Task 32 — Add Workflow Selection
**Status in file:** `pending` → **Verdict: ❌ MISSING (confirmed pending)**

`run_hyevo_cycle()` always picks `best_workflow()` by score. No input-type
inspection, no context-based routing, and no multi-workflow selection logic.

---

### Task 33 — Add Workflow Evolution Triggers
**Status in file:** `pending` → **Verdict: ⚠️ IN_PROGRESS (confirmed pending)**

- **Periodic trigger** ✅ — `handle_heartbeat()` calls `run_hyevo_cycle()` every 10 ticks
- **After N executions** ❌ — no counter
- **After failure** ❌ — `metrics.errors` is collected but never triggers evolution

---

### Tasks 34–37 — Tests (Genome, Mutation/Crossover, Scoring, CPU Integration)
**Status in file:** `pending` → **Verdict: ❌ MISSING (all confirmed pending)**

A full grep for `#[cfg(test)]` across all `src/hy_evo/*.rs` and `src/cpu/*.rs`
returned **zero matches**. No `tests/` directory exists at the crate root.
No test modules exist in any of these files.

---

### Task 38 — Add Workflow Visualizer (Mermaid)
**Status in file:** `pending` → **Verdict: ❌ MISSING (confirmed pending)**

No `visualizer.rs` or Mermaid export exists in `src/hy_evo/`. Search for
"mermaid" in `src/` only hit `memory/manager.rs` (belief graph — unrelated).

---

### Task 39 — Add Evolution Logs
**Status in file:** `pending` → **Verdict: ❌ MISSING (confirmed pending)**

No `log::info!`, `log::debug!`, or `tracing::` calls anywhere in
`src/hy_evo/`. The only output is a bare `println!("[HyEvo Reflection] …")`
in `integration.rs`. Mutations, crossovers, scores, and winners are all silent.

---

### Task 40 — Reserved / Removed Task
**Status in file:** `deferred` → **Verdict: ⏸️ DEFERRED**

Stub inserted to maintain sequential ID space. No action required.

---

### Task 41 — Update License to Custom Non-Commercial
**Status in file:** `done` → **Verdict: ✅ DONE**

`LICENSE` exists with explicit "no commercial use without permission" clause.
`readme.md` has real project content and references the license.

---

### Task 42 — Add System Manifest Feature
**Status in file:** `done` → **Verdict: ✅ DONE**

`src/config/manifest.rs` has `SystemManifest` struct with `load()` and
`load_from_string()`. `system_manifest.md` exists with 5 real sections:
Daily Routines, Memory Policies, Error Policies, Reflection Rules, and
Safety Constraints.

---

### Task 43 — Fix Compilation Errors for Manifest Integration
**Status in file:** `done` → **Verdict: ✅ DONE**

`cargo check` completes with **0 errors**. 1 warning only (unused import in
`src/bin/test_ollama.rs`).

---

### Task 44 — Remove/Archive `hartbeat.md`
**Status in file:** `done` → **Verdict: ✅ DONE**

`hartbeat.md` does not exist at the repo root. `src/hartbeat/` is an empty
stub directory. Minor note: `PROJECT_LAYOUT.md` still references it — small
doc cleanup outstanding.

---

### Task 45 — Parse Manifest into Structured Sections
**Status in file:** `done` → **Verdict: ✅ DONE**

`parse_sections()` in `manifest.rs` splits `##`-headed Markdown into a
`HashMap<String, String>` and is called on every `load()`.

---

### Task 46 — Add Automatic Manifest Evolution
**Status in file:** `done` → **Verdict: ✅ DONE**

`evolve_manifest()` in `cpu/mod.rs` — uses the LLM to propose improvements,
calls `diff()`, rejects changes that delete content, and applies validated
updates. Lives in the CPU (requires LLM access), not `manifest.rs`.

---

### Task 47 — Add Memory-Aware Prompting
**Status in file:** `done` → **Verdict: ⚠️ IN_PROGRESS**

`enhance_prompt_with_memory()` in `cpu/mod.rs` injects **working memory**
(last 3 entries) and **beliefs** into every LLM call. ✅

However:
- **Subtask 47.2** — vector `search_facts()` in prompts ❌ — not called
- **Subtask 47.3** — episodic memory entries in prompts ❌ — not called

**Action needed:** Call `memory.vector.search_facts(prompt, 3)` and
`memory.episodic.recent(5)` inside `enhance_prompt_with_memory()`.

---

### Task 48 — Add Web UI Panel for the Manifest
**Status in file:** `done` → **Verdict: ✅ DONE** *(implementation differs from spec)*

No REST `GET /api/manifest` or `POST /api/manifest` endpoints exist. However,
the manifest is fully served and saved via **WebSocket**:
- On connect: sends manifest file contents
- `manifest_save` WS message: writes to disk
- HTML has a full Manifest tab with textarea and Save button

Functionally complete. Either add REST endpoints to match the original spec
or update the task description to reflect the WS-based design.

---

### Task 49 — Add Belief Memory
**Status in file:** `done` → **Verdict: ✅ DONE**

`MemoryManager` in `manager.rs` has `beliefs: HashMap<String, Value>` with
`set_belief()`, `get_belief()`, and `get_all_beliefs()`. Full CRUD. ✅

---

### Task 50 — Manifest Diffing (Safe Updates Only)
**Status in file:** `done` → **Verdict: ✅ DONE**

`SystemManifest::diff()` in `manifest.rs` uses the `similar` crate's
`TextDiff::from_lines()` to produce a unified diff with 3-line context. ✅

---

### Task 51 — Belief Graph Visualization
**Status in file:** `done` → **Verdict: ✅ DONE**

`visualize_beliefs()` in `manager.rs` generates a `graph TD;` Mermaid diagram
from all belief key/value pairs. ✅

---

### Task 52 — Skill Arbitration
**Status in file:** `done` → **Verdict: ✅ DONE**

`arbitrate_skill()` in `cpu/mod.rs` calls the LLM with a skill list and
returns the best match. Note: skill list is currently hardcoded
(`"noop, send_email, read_log"`) rather than driven from the skill registry —
minor improvement opportunity.

---

### Task 53 — Nightly Maintenance Cycles
**Status in file:** `done` → **Verdict: ✅ DONE**

`nightly_maintenance()` in `cpu/mod.rs` — deletes logs older than 7 days,
backs up the manifest. Triggered at hour == 2 inside `handle_heartbeat()`. ✅

---

### Task 54 — Agent Personality Profiles
**Status in file:** `done` → **Verdict: ⚠️ IN_PROGRESS**

`personality: String` field exists on the `Cpu` struct (initialized to
`"neutral"`) and is injected into prompts via `enhance_prompt_with_memory()`.
However there is no multi-profile system, no profile loading from config, and
no switching mechanism. A single hardcoded string does not constitute
"profiles."

**Action needed:** Define personality profiles in `config.toml`, load them at
startup, and add a mechanism to switch between them (e.g. via slash command or
bus message).

---

### Task 55 — Self-Repair Routines
**Status in file:** `done` → **Verdict: ✅ DONE**

`self_repair()` in `cpu/mod.rs` — handles working-memory overflow truncation,
manifest integrity check, and stuck-state tick reset. Called every 100 ticks
inside `handle_heartbeat()`. ✅

---

### Task 56 — Context-Aware History Compression
**Status in file:** `done` → **Verdict: ✅ DONE**

`drain_oldest_chunk()` and `push_summary()` implemented (and tested) in
`memory/mod.rs`. `run_manifest_routines()` in `cpu/mod.rs` drains the 5 oldest
entries, calls `llm.summarize()`, and pushes the result back — real
LLM-driven compression. ✅

---

### Task 57 — Fix Chat Routing, Log Display, and LLM Selector
**Status in file:** `done` → **Verdict: ✅ DONE**

- WS handler routes `chat` to `gemini`, `ollama_{id}`, or first-backend
  fallback based on the `llm` field ✅
- LLM selector buttons built dynamically from `config.toml` backends + Gemini ✅
- Logs tab with live display ✅
- `llm_gemini/mod.rs` has retry/backoff ✅

---

### Task 58 — Tools and Skills System — Full Implementation
**Status in file:** `done` → **Verdict: ✅ DONE**

- `src/tools/mod.rs` — single `execute()` dispatcher covering 11 named tools ✅
- `src/tools/file_tools.rs` — `read_log`, `write_note`, `read_note`,
  `list_notes` with path-traversal guard ✅
- `src/tools/system_tools.rs` — `send_email`, `system_status`, `list_tools`,
  `get_beliefs`, `set_belief` ✅
- `src/tools/email_tools.rs` — outbox log ✅
- Slash commands (`/tools`, `/status`, `/beliefs`, `/set`, `/note`, `/help`) ✅

---

### Task 59 — Ollama Model Preload and Keep-Alive
**Status in file:** `done` → **Verdict: ✅ DONE**

`src/io/ollama/keepalive.rs`:
- `preload_model()` — cold-start preload, up to 3 retries with exponential
  backoff, Starlink-resilient ✅
- `spawn_keepalive_task()` — background loop, configurable interval via
  `OLLAMA_KEEP_ALIVE_SECS` env var ✅
- `OLLAMA_PRELOAD` flag to disable at startup ✅

---

## Summary Table

| ID | Title (abbreviated) | File Status | Code Verdict |
|----|---------------------|-------------|--------------|
| 1  | LLM Tool Calling | done | ✅ DONE |
| 2  | Vector Memory Search | ~~done~~ | ⚠️ IN_PROGRESS — dummy embeddings |
| 3  | Docker & Systemd Deploy | ~~done~~ | ❌ PENDING — zero files |
| 4  | Web Auth & Polish | ~~done~~ | ❌ PENDING — no JWT code |
| 5  | Planning Loop | ~~done~~ | ⚠️ IN_PROGRESS — stubs only |
| 6  | Logging executor.rs | done | ✅ DONE |
| 7  | Logging instructions.rs | ~~done~~ | ⚠️ IN_PROGRESS — 1 call only |
| 8  | Logging interrupts.rs | done | ✅ DONE |
| 9  | Logging mod.rs | done | ✅ DONE |
| 10 | Logging scheduler.rs | done | ✅ DONE |
| 11 | Logging state.rs | done | ✅ DONE |
| 12 | Create hy_evo/ module | done | ✅ DONE |
| 13 | Define Node enum | done | ✅ DONE |
| 14 | Define Workflow struct | done | ✅ DONE |
| 15 | Implement serialization | done | ✅ DONE |
| 16 | Node-level mutations | ~~done~~ | ⚠️ IN_PROGRESS — no replace_node |
| 17 | Workflow-level mutations | done | ✅ DONE |
| 18 | Mutation probability config | done | ✅ DONE |
| 19 | Single-point crossover | done | ✅ DONE |
| 20 | Multi-point crossover | done | ✅ DONE |
| 21 | Validate resulting workflow | done | ✅ DONE |
| 22 | Define scoring metrics | done | ✅ DONE |
| 23 | Implement scoring function | done | ✅ DONE |
| 24 | Add score persistence | done | ✅ DONE |
| 25 | Implement reflection node | done | ✅ DONE |
| 26 | Implement reflection cycle | done | ✅ DONE |
| 27 | Store reflection results | done | ✅ DONE |
| 28 | Implement evolution cycle | pending | ⚠️ IN_PROGRESS — LLM only, no GA loop |
| 29 | Add population manager | pending | ⚠️ IN_PROGRESS — no cull/keep-K |
| 30 | Add workflow registry | pending | ❌ MISSING |
| 31 | Workflow execution in CPU | ~~pending~~ | ✅ DONE — corrected |
| 32 | Add workflow selection | pending | ❌ MISSING |
| 33 | Evolution triggers | pending | ⚠️ IN_PROGRESS — periodic only |
| 34 | Unit tests for genome | pending | ❌ MISSING |
| 35 | Tests for mutation/crossover | pending | ❌ MISSING |
| 36 | Scoring tests | pending | ❌ MISSING |
| 37 | CPU integration tests | pending | ❌ MISSING |
| 38 | Workflow visualizer (Mermaid) | pending | ❌ MISSING |
| 39 | Evolution logs | pending | ❌ MISSING |
| 40 | Reserved / Removed Task | deferred | ⏸️ DEFERRED |
| 41 | Update License | done | ✅ DONE |
| 42 | Add System Manifest Feature | done | ✅ DONE |
| 43 | Fix Compilation Errors | done | ✅ DONE |
| 44 | Remove hartbeat.md | done | ✅ DONE |
| 45 | Parse Manifest Sections | done | ✅ DONE |
| 46 | Automatic Manifest Evolution | done | ✅ DONE |
| 47 | Memory-Aware Prompting | ~~done~~ | ⚠️ IN_PROGRESS — 47.2 & 47.3 missing |
| 48 | Web UI Panel for Manifest | done | ✅ DONE (via WebSocket, not REST) |
| 49 | Add Belief Memory | done | ✅ DONE |
| 50 | Manifest Diffing | done | ✅ DONE |
| 51 | Belief Graph Visualization | done | ✅ DONE |
| 52 | Skill Arbitration | done | ✅ DONE |
| 53 | Nightly Maintenance Cycles | done | ✅ DONE |
| 54 | Agent Personality Profiles | ~~done~~ | ⚠️ IN_PROGRESS — 1 hardcoded string |
| 55 | Self-Repair Routines | done | ✅ DONE |
| 56 | Context-Aware History Compression | done | ✅ DONE |
| 57 | Fix Chat Routing / LLM Selector | done | ✅ DONE |
| 58 | Tools and Skills System | done | ✅ DONE |
| 59 | Ollama Preload and Keep-Alive | done | ✅ DONE |

---

## Tally

| Verdict | Count | Task IDs |
|---------|-------|----------|
| ✅ DONE | 39 | 1,6,8,9,10,11,12,13,14,15,17,18,19,20,21,22,23,24,25,26,27,31,41,42,43,44,45,46,48,49,50,51,52,53,55,56,57,58,59 |
| ⚠️ IN_PROGRESS | 6 | 2, 5, 7, 16, 47, 54 |
| ❌ PENDING | 13 | 3, 4, 28, 29, 30, 32, 33, 34, 35, 36, 37, 38, 39 |
| ⏸️ DEFERRED | 1 | 40 |

---

## Priority Action Items

### High Priority — Wrongly marked `done`, no code at all
1. **Task 3** — Create `Dockerfile` and `bot.service` systemd unit
2. **Task 4** — Implement JWT authentication middleware in `src/io/web_server/mod.rs`

### Medium Priority — Partial implementations needing completion
3. **Task 2** — Replace `dummy_embed()` in `vector.rs` with a real embedding call (Ollama `/api/embeddings` or fastembed)
4. **Task 5** — Implement real planning/decomposition logic in `cpu/executor.rs` for `PlanNextSteps`
5. **Task 47** — Add `search_facts()` and `episodic.recent()` calls into `enhance_prompt_with_memory()`
6. **Task 16** — Add explicit `replace_node()` function to `mutation.rs`
7. **Task 54** — Load personality profiles from `config.toml`; add switching mechanism

### Low Priority — Logging gap
8. **Task 7** — Add comprehensive logging throughout `cpu/instructions.rs`

### HyEvo Completion Block (tasks 28–39)
The HyEvo data structures and algorithms are solid. What is missing is the
wiring that makes the system self-evolve:
- **Task 28/29** — Wire `MutationEngine` + `CrossoverEngine` + `ScoringEngine` into `engine.rs::evolve_once()`; add population culling
- **Task 30** — Add `WorkflowRegistry` keyed by input type
- **Task 32** — Add context-aware workflow selection in CPU
- **Task 33** — Add after-N-executions and after-failure triggers
- **Tasks 34–37** — Write all missing `#[cfg(test)]` modules
- **Task 38** — Add Mermaid graph export for workflow genomes
- **Task 39** — Replace `println!` with `log::info!` / `log::debug!` throughout `src/hy_evo/`
