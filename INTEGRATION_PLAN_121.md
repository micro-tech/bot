# Task 121 Integration Plan — Reasoning Engine + HyEvo + Manifest Evolution

**Goal**: Make the large amount of currently dead_code infrastructure (ReasoningEngine, HyEvo, OllamaLlm, manifest evolution) actually active inside the Cpu loop.

**Status**: In Progress (Aggressive mode) — Execution started 2025, moved to core integration 2026

**Current Focus**: 121.5 (Dead code cleanup — SystemManifest, SocketConfig, etc.)

---

## Current State (Discovered)

- `Cpu<L>` already has `HyEvoIntegration<L>` and calls `run_hyevo_cycle()` every 10 ticks.
- `ReasoningEngine` is fully implemented but **never instantiated**.
- `OllamaLlm` exists in `src/io/ollama/mod.rs::llm` but trait implementations are incomplete.
- Manifest evolution and self-repair routines exist but are weakly triggered.
- Main entry point (`main.rs`) is heavily bus-driven and does not yet construct a `Cpu` instance in the visible flow.

---

## Subtask Breakdown & Plan

### 121.1 Wire ReasoningEngine into Cpu heartbeat and bus handlers

**Changes needed**:
- Add `reasoning: Option<ReasoningEngine>` field to `Cpu<L>`.
- In `Cpu::new()`, optionally create a `ReasoningEngine` (or always create one).
- In `handle_heartbeat()`: periodically call reasoning methods (start, propose hypotheses from user goals, create plans).
- In `handle_bus_message()`: route `user_input` and error messages into the reasoning engine.
- Expose reasoning trace/summary via bus when requested.

**Files**:
- `src/cpu/mod.rs`
- `src/reasoning/engine.rs` (minor extensions if needed)

---

### 121.2 Enable HyEvo by default and connect OllamaLlm

**Changes needed**:
- Implement missing traits on `OllamaLlm` (`ReflectionLlm`, `LlmInterface`).
- Ensure `HyEvoIntegration` is seeded with at least one initial `WorkflowGenome` on `Cpu` creation.
- Add a manifest toggle (`"Run HyEvo cycle"`) that is respected.
- Make HyEvo run by default (remove any feature flag that disables it).

**Files**:
- `src/io/ollama/mod.rs` (the `llm` submodule)
- `src/cpu/mod.rs`
- `src/hy_evo/integration.rs` (seeding logic)

---

### 121.3 Activate manifest evolution and self-repair paths

**Changes needed**:
- Make `evolve_manifest()` and `self_repair()` more proactive.
- Allow manifest content to control:
  - Whether HyEvo runs
  - Whether ReasoningEngine is active
  - Frequency of evolution cycles
- Wire `nightly_maintenance()` to also trigger manifest evolution when appropriate.

**Files**:
- `src/cpu/mod.rs`
- `src/config/manifest.rs`

---

### 121.4 Add integration tests

**Changes needed**:
- Create or expand tests that exercise:
  - Full `Cpu` heartbeat with Reasoning + HyEvo
  - Manifest evolution path
  - OllamaLlm trait usage
- Verify that reasoning traces and evolution logs appear.

**Files**:
- `src/cpu/mod.rs` (or new `tests/integration_121.rs`)
- Possibly `src/hy_evo/tests.rs`

---

### 121.5 Reduce dead_code warnings to minimal level

**Strategy**:
- After wiring the above, many `#[allow(dead_code)]` and unused modules will become live.
- Run `cargo check` repeatedly and remove unnecessary allows.
- Target: < 20 warnings total.

**Files**: Whole crate (focus on `src/reasoning/`, `src/hy_evo/`, `src/cpu/`)

---

## Execution Order (Aggressive)

1. **Quick foundation** (121.2 partial)
   - Make `OllamaLlm` implement required traits.
   - Seed HyEvo on Cpu creation.

2. **Core wiring** (121.1)
   - Add `ReasoningEngine` to `Cpu`.
   - Hook into heartbeat and bus.

3. **Activation & control** (121.3)
   - Strengthen manifest-driven behavior.

4. **Tests + cleanup** (121.4 + 121.5)
   - Add tests.
   - Drive warning count down.

---

## Risks & Mitigations

- **Trait mismatch** on `OllamaLlm` → May require small adapter or trait adjustments.
- **Bus vs direct coupling** → ReasoningEngine currently takes a `Bus`; Cpu already has one.
- **Performance** → HyEvo + Reasoning on every heartbeat could be heavy; keep cycles throttled.
- **Manifest safety** → Evolution already has a "no deletions" check — keep it.

---

## Tracking

- Created:  (during Task 121 planning)
- Backup of original task_list: `.zed/task_list.json.bak`
- Plan owner: Agent (Grok-assisted)

---

**Next Action**: Begin aggressive execution starting with 121.2 (OllamaLlm traits + HyEvo seeding).
