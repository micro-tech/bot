# Project Layout

This document outlines the directory structure and key modules of the bot project. The project is a modular Rust-based AI system for executing workflows, managing memory, skills, and evolutionary adaptations. It uses async runtime (Tokio) for concurrency and Serde for JSON handling.

## Root Files
- `Cargo.toml`: Dependencies and build configuration (e.g., tokio, serde, async-trait).
- `main.rs`: Entry point; initializes Executor, Memory, Skills, Hooks, and HyEvo integration.
- `PROJECT_LAYOUT.md`: This file.
- `flow_map.md`: High-level data and control flow diagrams.
- `README.md`: Project overview and setup instructions.

## `src/` (Source Code)
The core Rust modules. All crates are under `bot` crate.

### `cpu/` (Central Processing Unit - Execution Layer)
Handles instruction execution, scheduling, interrupts, and integration with other components.
- `mod.rs`: Exports public items (e.g., `Executor`, `Scheduler`).
- `executor.rs`: Main execution engine. Manages memory reads/writes, skill execution, hook phases, and belief updates. Integrates with HyEvo for dynamic workflows.
- `scheduler.rs`: Schedules CPU events (e.g., `CpuEvent`) and serializes them for memory/queueing.
- `interrupts.rs`: Polls interrupt bus for events (e.g., using `mpsc` channels).
- `instructions.rs`: Defines `CpuEvent` struct/enum (with Serde derives for JSON serialization).
- `interfaces.rs`: Traits like `MemoryInterface` (for read/write/update_belief), `SkillInterface`, `HookInterface`.

### `memory/` (Belief and Context Management)
Persistent and transient storage for beliefs, context, and state.
- `mod.rs`: Defines `MemoryHandle` (implements `MemoryInterface`). Includes `VecDeque<String>` for context joining and belief updates.
- Integrates with executor for async read/write operations.

### `skills/` (Modular Action Plugins)
Registry for executable skills (e.g., API calls, computations).
- `mod.rs`: Defines `SkillRegistry` and `SkillInterface` trait (with `run` method for async execution of skills with `HashMap<String, Value>` args).
- Skills are stored as boxed async functions; executed via executor.

### `hooks/` (Lifecycle Event Handlers)
Registry for phases (e.g., init, post-execution) with mutable state updates.
- `mod.rs`: Defines `HookRegistry` and `HookInterface` trait (with `run_phase` method for async execution on phases with `&mut ExecutionState`).
- Hooks run sequentially in executor phases.

### `hy_evo/` (Evolutionary Workflow Engine - NEW MODULE)
Adaptive system for evolving workflows (genomes) using LLMs for reflection and optimization. Integrates with executor to dynamically generate/execute skills, LLM calls, and code nodes. Requires `ReflectionLlm` trait for LLM providers (e.g., OpenAI-compatible).

**Purpose**: Enables self-improving behaviors by evolving workflows based on metrics (e.g., performance feedback). Seed initial genomes, evolve iteratively, and retrieve best workflows for executor.

**Key Files**:
- `mod.rs`: Exports public items (e.g., `HyEvoEngine`, `HyEvoIntegration`, `Workflow`).
- `engine.rs`: Core `HyEvoEngine<L: ReflectionLlm + Send + Sync>` struct. Methods: `seed(genome)`, `evolve_once(metrics)`, `best_workflow()`. Manages population of workflows and fitness evaluation.
- `integration.rs`: `HyEvoIntegration<L>` wrapper with `Arc<Mutex<HyEvoEngine<L>>>`. Methods: `new()`, `get_best_workflow()`, `seed()`, `evolve()`. Used in executor for locking and async evolution.
- `workflow.rs`: Defines `Node` enum (e.g., `Skill { name, params: HashMap<String, Value> }`, `Llm { model, prompt_template, params }`, `Code { function, params }`). `WorkflowContext` for execution. Methods: `execute_skill`, `execute_llm`, `execute_code` (convert `HashMap` params to `serde_json::Value`). `execute_workflow` matches on nodes and awaits results.
- `reflection.rs`: Defines `ReflectionLlm` trait (e.g., methods for prompting LLMs to reflect on/evolve code).

**Integration Notes**:
- Instantiate `HyEvoIntegration` in `main.rs` or executor init with an LLM (e.g., `L = OpenAILlm`).
- In `executor.rs`, call `integration.evolve(metrics).await` after executions to adapt workflows.
- Workflows output `Node`s that map to skills/hooks/LLM calls in executor.
- Dependencies: Add `rand` for genetic algorithms, `serde_json` for params serialization.

### Other Modules
- `bus/`: Message passing (e.g., `mpsc` channels for interrupts/events).
- `lib.rs`: Crate root; re-exports modules for `pub use`.

## `tests/` (Unit and Integration Tests)
- `executor.rs`: Tests for memory ops, skill runs, hook phases.
- `hy_evo.rs`: NEW - Tests for workflow evolution, node execution, integration with executor.
- `integration.rs`: End-to-end tests (e.g., seed -> evolve -> execute workflow).

## `examples/`
- `simple_bot.rs`: Basic executor with memory and skills.
- `evolving_workflow.rs`: NEW - Example using HyEvo to evolve a chat response workflow.

## Build and Run
- `cargo build`: Compile.
- `cargo test`: Run tests.
- `cargo run --example evolving_workflow`: Demo HyEvo.

## Future Plans
- Add more LLM providers for `ReflectionLlm`.
- Integrate HyEvo metrics with external feedback (e.g., user ratings).
- Visualize workflows with Graphviz in flow_map.md.
