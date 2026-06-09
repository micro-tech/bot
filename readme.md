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
- **Reasoning Engine** — Hypothesis → Plan → Execute loop with self-correction, pause/resume, metrics, and bus control

### Integration
- **Ollama Integration** — Direct API access with retry/backoff logic
- **Heartbeat System** — Periodic status reporting to the LLM
- **JSON Schema Validation** — Structured command validation (Task 111)

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
| Reasoning Engine | [docs/reasoning.md](docs/reasoning.md) *(new)* |
| Configuration | [docs/configuration.md](docs/configuration.md) |
| CLI Commands | [docs/cli_commands.md](docs/cli_commands.md) |
| UNIX Socket | [docs/unix_socket.md](docs/unix_socket.md) |
| Plugins | [docs/plugins.md](docs/plugins.md) |
| Logging | [docs/logging.md](docs/logging.md) |
| Memory System | [docs/memory_system.md](docs/memory_system.md) |
| HyEvo | [docs/hyevo.md](docs/hyevo.md) |
| Web Interface | [docs/web_interface.md](docs/web_interface.md) |

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
