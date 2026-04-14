# Project Layout

This document outlines the directory structure and key modules of the bot project. The project is a modular Rust-based AI system for executing workflows, managing memory, skills, evolutionary adaptations, and now includes web interface, LLM integration, heartbeat scheduling, agent-to-agent communication, cron jobs, MCP protocol, and more. It uses async runtime (Tokio) for concurrency and Serde for JSON handling.

## Root Files
- `Cargo.toml`: Dependencies and build configuration (e.g., tokio, serde, async-trait, toml, log).
- `config.toml`: Configuration file for bot name, Ollama settings, web port, heartbeat interval.
- `.env`: Environment variables (if any).
- `readme.md`: Project overview and setup instructions.
- `PROJECT_LAYOUT.md`: This file.
- `flow_map.md`: High-level data and control flow diagrams.
- `Grok.md`, `hartbeat.md`: Additional documentation.
- `Doc's/`: Documentation directory.
- `logs/`: Directory for log files.
- `certs/`: Certificates for HTTPS.
- `tooling/`: Tools and scripts.

## `src/` (Source Code)
The core Rust modules. The crate is a binary with main.rs.

- `main.rs`: Entry point; initializes bus, subsystems (memory manager with working/episodic/vector, skills, hooks, hy_evo), spawns web server, Ollama handler, CPU, time scheduler for heartbeat, and other handlers like a2a, cron, mcp.
- `utils.rs`: Utility functions, e.g., log_to_file.
- `bayesian.rs`: Bayesian reasoning module (likely for probabilistic inference).

### `a2a/` (Agent-to-Agent Communication)
- `a2a_handler.rs`: Handler for agent-to-agent interactions.

### `bin/` (Binary Utilities)
- `test_ollama.rs`: Test binary for Ollama.

### `bus/` (Message Bus)
- `mod.rs`: Defines `Bus` for pub-sub messaging with `mpsc` channels.
- `handler.rs`: Additional bus handling logic.

### `cpu/` (Central Processing Unit - Execution Layer)
Handles instruction execution, scheduling, interrupts, and integration with other components.
- `mod.rs`: Exports public items (e.g., `Cpu`, `CpuExecutor`, `TimeScheduler`).
- `cpu.rs`: Main `Cpu` struct, handles bus messages, user input, LLM requests.
- `executor.rs`: Main execution engine. Manages memory reads/writes, skill execution, hook phases, belief updates. Integrates with HyEvo.
- `scheduler.rs`: Schedules CPU events.
- `time_scheduler.rs`: Time-based scheduler for heartbeat.
- `interrupts.rs`: Polls interrupt bus for events.
- `instructions.rs`: Defines `Instruction` struct/enum.
- `interfaces.rs`: Traits like `MemoryInterface`, `SkillInterface`, `HookInterface`.
- `state.rs`: `AgentState` for CPU state.

### `cron/` (Cron Jobs)
- `cron_handler.rs`: Handler for scheduled tasks.

### `hartbeat/` (Heartbeat)
- (Empty or placeholder for heartbeat functionality)

### `hooks/` (Lifecycle Event Handlers)
Registry for phases (e.g., init, post-execution) with mutable state updates.
- `mod.rs`: Defines `HookRegistry` and `HookInterface`.
- `bayesian.rs`: Bayesian hooks.

### `hy_evo/` (Evolutionary Workflow Engine)
Adaptive system for evolving workflows using LLMs for reflection. Integrates with CPU.
- `mod.rs`: Exports `HyEvoEngine`, `HyEvoIntegration`, `Workflow`, and submodules like crossover, mutation, scoring.
- `engine.rs`: Core engine for seeding, evolving, best workflow.
- `integration.rs`: Wrapper for async evolution.
- `workflow.rs`: `Node` enum (Skill, Llm, Code), execution methods.
- `reflection.rs`: `ReflectionLlm` trait.
- `genome.rs`: Genome representation.
- `node.rs`: Node definitions.
- `mutation.rs`: Mutation engine.
- `crossover.rs`: Crossover engine.
- `scoring.rs`: Scoring system.

### `io/` (Input/Output Interfaces)
Handles external I/O like web server, LLM services, terminal.
- `mod.rs`: Declares submodules.
- `io.rs`: General IO utilities.
- `ollama/`: Submodule for Ollama integration.
- `web_server/`: HTTPS web server for interface.
- `terminal/`: Terminal I/O.
- `llm_gemini/`: Gemini LLM integration.
- `tests/`: IO tests.
- `logs/`: Log handling.

### `llm/` (Large Language Model Interfaces)
- `mod.rs`: Declares ollama submodule.
- `ollama.rs`: `OllamaLlm` struct for LLM interactions.

### `mcp/` (Model Control Protocol?)
- `mcp_handler.rs`: Handler for MCP.

### `memory/` (Belief and Context Management)
- `mod.rs`: Defines `MemoryHandle` (short-term), `MemoryManager`.
- `manager.rs`: `MemoryManager` combining working, episodic, vector memories.
- `episodic.rs`: Episodic memory.
- `vector.rs`: Vector memory.

### `skills/` (Modular Action Plugins)
- `mod.rs`: Defines `SkillRegistry` and `SkillInterface`.
- `bayesian.rs`: Bayesian skills.

### `tools/` (Tools)
- (Empty or placeholder)

## `tests/` (Unit and Integration Tests)
- Various test files for modules.

## Build and Run
- `cargo build`: Compile.
- `cargo test`: Run tests.
- `cargo run`: Run the bot (uses config.toml).

## Future Plans
- Implement missing modules like hartbeat, tools.
- Expand LLM providers.
- Add more I/O interfaces.
- Enhance HyEvo with more evolution strategies.
