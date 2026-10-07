# Helix

**Helix** is a modular Rust-based AI agent framework that acts as an "operating system" for intelligent agents, primarily powered by local LLMs (Ollama).

It provides a central bus, execution engine, memory system, plugin architecture, and multiple interfaces (terminal, web, UNIX socket).

---

## What's new (Oct 2026) — merged via PR #5

The overnight mega-batch (tasks 187–211) landed on `PreRelese`:

- **Protocols**: ACP server (189), real A2A v1.0 (190), guarded `run_shell` tool (191), embedded SSH server (`src/ssh/`, 192).
- **Web UI**: Grok-style three-column layout (193–198), Helix brand icon as favicon/header (209).
- **Autonomy**: heartbeat rewired as a real job pump (199) with observability (200), a real cron registry (201), and nightly maintenance (202).
- **Memory rebuilt** (203–206): `beliefs.json` as the single source of truth, disk-backed episodic + vector memory with real Ollama embeddings, nightly consolidation.
- **OKF v1** (207–208): protocol contract in `PROTOCOL.md`; Helix runs its own OKF server (`[helix.okf]`, serves `/okf/*`, self-fetch) with `OkfLibrarian`/`OkfFetcher`/`OkfRegistry` in `src/okf/`.
- **Cleanup** (210–211): warning-free build, runtime rule layer wired, `tests/*.rs` ported to the `helix::` crate name — which caught a real bug (trace JSONL is now flushed before the 202 ack).

**Direction**: the in-Helix OKF server stays as an **offline fallback**; the Dell's shared OKF backend becomes the primary knowledge store (unification plan, Phase 3). Full details in [CHANGELOG.md](CHANGELOG.md).

---

## Features

### Core System
- [Central Bus](docs/central_bus.md)
- [CPU Executor & Scheduler](docs/workflow_execution_cpu.md)
- [Memory System](docs/memory_system.md)
- [Skills & Hooks](docs/skills_hooks.md)
- [HyEvo Engine](docs/hyevo.md)
- [Reasoning Engine](docs/reasoning.md)
- [Planning Loop](docs/planning_loop.md)
- [Workflow Registry](docs/workflow_registry.md)
- [Workflow Triggers](docs/workflow_triggers.md)
- [Population Manager](docs/population_manager.md)
- [Evolution Cycle](docs/evolution_cycle.md)
- [Workflow Selection](docs/workflow_selection.md)
- [Vector Memory](docs/vector_memory.md)
- [Ollama Tool Calling](docs/ollama_tool_calling.md)

### Interfaces
- [Web Interface](docs/web_interface.md)
- [Terminal CLI](docs/terminal_cli.md)
- [UNIX Domain Socket CLI](docs/unix_socket.md)

### Reliability & Extensibility
- [Plugin System](docs/plugins.md)
- [Checksum & Resume Transfers](docs/checksum_resume.md)
- [Metrics Collection](docs/metrics.md)
- [Human-Readable Logging](docs/logging.md)
- [Configuration](docs/configuration.md)
- [Docker & Systemd](docs/docker_systemd.md)

---

## Quick Start

```bash
git clone https://github.com/micro-tech/bot.git
cd bot
cargo build --release
cargo run
```

---

## Documentation

All documentation lives in the **`docs/`** folder.

### Special Root Files
These files are intentionally kept in the project root:

| File            | Purpose                                      |
|-----------------|----------------------------------------------|
| `Grok.md`       | Instructions / context for the Grok CLI agent |
| `hartbeat.md`   | Heartbeat configuration / status reference    |

### Main Documentation
| Topic                        | File                                      |
|-----------------------------|-------------------------------------------|
| Architecture Overview       | [PROJECT_LAYOUT.md](docs/PROJECT_LAYOUT.md) |
| Data Flows                  | [flow_map.md](docs/flow_map.md)           |
| Project Layout (Legacy)     | [PROJECT_LAYOUT_legacy.md](docs/PROJECT_LAYOUT_legacy.md) |
| Flow Map (Legacy)           | [flow_map_legacy.md](docs/flow_map_legacy.md) |
| System Manifest             | [system_manifest.md](docs/system_manifest.md) |
| Testing Guide               | [TESTING.md](docs/TESTING.md)             |
| Integration Plan            | [INTEGRATION_PLAN_121.md](docs/INTEGRATION_PLAN_121.md) |

### LLM Router Documentation
| Topic                              | File |
|------------------------------------|------|
| Architecture Overview              | [LLM_ROUTER_ARCHITECTURE.md](docs/LLM_ROUTER_ARCHITECTURE.md) |
| Subsystem Reference                | [LLM_ROUTER_SUBSYSTEMS.md](docs/LLM_ROUTER_SUBSYSTEMS.md) |
| Data Flow & Diagrams               | [LLM_ROUTER_DATA_FLOW.md](docs/LLM_ROUTER_DATA_FLOW.md) |
| Concrete Examples                  | [LLM_ROUTER_EXAMPLES.md](docs/LLM_ROUTER_EXAMPLES.md) |
| Extension Hooks                    | [LLM_ROUTER_EXTENSION_HOOKS.md](docs/LLM_ROUTER_EXTENSION_HOOKS.md) |
| How to Add a New Backend           | [HOW_TO_ADD_NEW_BACKEND.md](docs/HOW_TO_ADD_NEW_BACKEND.md) |
| How to Tune Router Behavior        | [HOW_TO_TUNE_ROUTER.md](docs/HOW_TO_TUNE_ROUTER.md) |
| How to Debug Routing Decisions     | [HOW_TO_DEBUG_ROUTING.md](docs/HOW_TO_DEBUG_ROUTING.md) |
| Documentation Pipeline             | [LLM_ROUTER_DOCS_PIPELINE.md](docs/LLM_ROUTER_DOCS_PIPELINE.md) |
| Production Readiness Checklist     | [LLM_ROUTER_PRODUCTION_CHECKLIST.md](docs/LLM_ROUTER_PRODUCTION_CHECKLIST.md) |

---

## Development

- Run tests: `cargo test`
- Add a new skill: Implement `SkillInterface`
- Add a hook: Implement `HookInterface`

---

## License

Custom Non-Commercial License. See [LICENSE](LICENSE).

---

**Built with Rust + Tokio + Ollama • Helix AI Agent**
