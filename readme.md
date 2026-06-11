# Agent OS

**Agent OS** is a modular Rust-based framework that acts as an "operating system" for AI agents, primarily powered by local LLMs (Ollama).

It provides a central bus, execution engine, memory system, plugin architecture, and multiple interfaces (terminal, web, UNIX socket).

---

## Features

### Core System
- **[Central Bus](docs/central_bus.md)** — Decoupled publish-subscribe messaging between all components
- **[CPU Executor & Scheduler](docs/workflow_execution_cpu.md)** — Instruction execution, interrupts, and workflow scheduling
- **[Memory System](docs/memory_system.md)** — Working, episodic, and vector memory with belief management
- **[Skills & Hooks](docs/skills_hooks.md)** — Modular action plugins and lifecycle event handlers
- **[HyEvo Engine](docs/hyevo.md)** — Evolutionary workflow improvement using LLM reflection
- **[Reasoning Engine](docs/reasoning.md)** — Hypothesis → Plan → Execute loop with self-correction, pause/resume, metrics, and bus control
- **[Planning Loop](docs/planning_loop.md)** — LLM-based task decomposition and reflection scaffolding
- **[Workflow Registry](docs/workflow_registry.md)** — Register and retrieve best workflows by input type
- **[Workflow Triggers](docs/workflow_triggers.md)** — Cron, OnFailure, and Manual triggers for workflows
- **[Population Manager](docs/population_manager.md)** — Automatic workflow population culling and selection
- **[Evolution Cycle](docs/evolution_cycle.md)** — Top-N selection, crossover, mutation, and offspring generation
- **[Workflow Selection](docs/workflow_selection.md)** — Best workflow selection logic
- **[Vector Memory](docs/vector_memory.md)** — RAG-style long-term fact storage with cosine similarity search
- **[Ollama Tool Calling](docs/ollama_tool_calling.md)** — Agentic tool calling via Ollama `/api/chat` with tool definitions and iterative execution

### Interfaces
- **[Web Interface](docs/web_interface.md)** — HTTPS chat UI with live log viewing and WebSockets
- **[Terminal CLI](docs/terminal_cli.md)** — Interactive `AgentOS>` prompt
- **[UNIX Domain Socket CLI](docs/unix_socket.md)** — Scriptable JSON-lines interface with rich commands

### Reliability & Extensibility
- **[Plugin System](docs/plugins.md)** — Execute external scripts via the CLI
- **[Checksum & Resume Transfers](docs/checksum_resume.md)** — SHA-256 verification and resumable file operations
- **[Metrics Collection](docs/metrics.md)** — Command usage and performance tracking
- **[Human-Readable Logging](docs/logging.md)** — 12-hour timestamped Markdown logs (`error_log.md`, etc.)
- **[Configuration](docs/configuration.md)** — All settings via `config.toml`
- **[Docker & Systemd](docs/docker_systemd.md)** — Multi-stage Docker builds and systemd service deployment
- **MCP Integration** — Model Context Protocol server support via `src/mcp/`
- **Cron System** — Scheduled recurring tasks (`src/cron/`)
- **Events System** — Structured event handling (`src/events/`)
- **A2A (Agent-to-Agent)** — Inter-agent communication scaffolding (`src/a2a/`)

### Integration
- **Ollama Integration** — Direct API access with retry/backoff logic and tool calling
- **Heartbeat System** — Periodic status reporting to the LLM
- **JSON Schema Validation** — Structured command validation (Task 111)
- **Bayesian Inference** — Probabilistic reasoning module (`src/bayesian.rs`)

---

## Quick Start

### Prerequisites
- Rust (stable)
- Ollama running locally or remotely
- Self-signed certs in `certs/` (auto-created)

### Build & Run
```bash
git clone https://github.com/yourusername/bot.git
cd bot
cargo build --release
cargo run
```

### Test Ollama Connectivity
```bash
cargo run --bin test_ollama
```

---

## Communication Methods

### 1. Terminal CLI
Starts automatically with an `AgentOS>` prompt.

### 2. Web UI
Visit `https://localhost:8443`

### 3. UNIX Socket (Recommended for scripting)
```bash
echo '{"cmd":"ping"}' | nc -U /tmp/agentos.sock
```

See the full [CLI Commands Reference](docs/cli_commands.md).

---

## Configuration

Edit `config.toml`:
```toml
[ollama]
host = "192.168.1.149"
port = 11434
model = "llama3"

[web_interface]
https_port = 8443

[socket]
path = "/tmp/agentos.sock"
mode = 0o660
```

---

## Documentation

All feature documentation lives in the `docs/` folder:

| Topic | Link |
|-------|------|
| Architecture Overview | [PROJECT_LAYOUT.md](PROJECT_LAYOUT.md) |
| Data Flows | [flow_map.md](flow_map.md) |
| Reasoning Engine | [docs/reasoning.md](docs/reasoning.md) |
| Planning Loop | [docs/planning_loop.md](docs/planning_loop.md) |
| Workflow Registry | [docs/workflow_registry.md](docs/workflow_registry.md) |
| Workflow Triggers | [docs/workflow_triggers.md](docs/workflow_triggers.md) |
| Population Manager | [docs/population_manager.md](docs/population_manager.md) |
| Evolution Cycle | [docs/evolution_cycle.md](docs/evolution_cycle.md) |
| Workflow Selection | [docs/workflow_selection.md](docs/workflow_selection.md) |
| Vector Memory | [docs/vector_memory.md](docs/vector_memory.md) |
| Ollama Tool Calling | [docs/ollama_tool_calling.md](docs/ollama_tool_calling.md) |
| Configuration | [docs/configuration.md](docs/configuration.md) |
| CLI Commands | [docs/cli_commands.md](docs/cli_commands.md) |
| UNIX Socket | [docs/unix_socket.md](docs/unix_socket.md) |
| Plugins | [docs/plugins.md](docs/plugins.md) |
| Logging | [docs/logging.md](docs/logging.md) |
| Memory System | [docs/memory_system.md](docs/memory_system.md) |
| HyEvo | [docs/hyevo.md](docs/hyevo.md) |
| Web Interface | [docs/web_interface.md](docs/web_interface.md) |
| Docker & Systemd | [docs/docker_systemd.md](docs/docker_systemd.md) |

---

## Development

- Add a new skill: Implement `SkillInterface`
- Add a hook: Implement `HookInterface`
- Run tests: `cargo test`
- Full test coverage on new modules (checksum, plugins, metrics, resume, schema)

---

## License

Custom Non-Commercial License. See [LICENSE](LICENSE).

---

**Built with Rust + Tokio + Ollama**
