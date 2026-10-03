# Helix

**Helix** is a modular Rust-based AI agent framework that acts as an "operating system" for intelligent agents, primarily powered by local LLMs (Ollama).

It provides a central bus, execution engine, memory system, plugin architecture, and multiple interfaces (terminal, web, UNIX socket).

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
git clone https://github.com/yourusername/helix.git
cd helix
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
