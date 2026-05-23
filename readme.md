Agent OS Bot

## Overview

**Agent OS** is a Rust-based modular framework designed as an "operating system" for AI agents, primarily powered by local LLMs like Ollama. It features:

- **Central Bus System**: Decoupled inter-component communication using publish-subscribe messaging.
- **CPU Execution Layer**: Handles instruction execution, scheduling, interrupts, and integration with subsystems like memory, skills, hooks, and HyEvo.
- **Memory Manager**: Combines working (short-term context), episodic (event records), and vector (fact search) memories for belief management.
- **Skills and Hooks**: Modular action plugins and lifecycle event handlers for extensible behavior.
- **HyEvo Evolutionary Engine**: Adaptive system for evolving workflows using LLMs for reflection and improvement.
- **Heartbeat Module**: Periodic system status signals sent to Ollama for analysis.
- **Ollama Integration**: Direct API communication for LLM tasks (with placeholders for Gemini/Grok).
- **Interfaces**:
  - HTTPS Web Interface (chat, logs, WebSockets).
  - Terminal CLI for direct interaction.
- **Logging**: Comprehensive Markdown logs for errors, chats, bus transactions, heartbeats.
- **Modular Design**: Components like A2A (agent-to-agent), Cron (scheduled tasks), MCP (model control protocol), and more.
- **Resilient Networking**: Retries, timeouts, exponential backoff for unreliable connections (e.g., Starlink).
- **Test Binary**: `test_ollama` to verify Ollama connectivity.

The system acts as a kernel (`src/main.rs`), coordinating subsystems via the bus. All settings in `config.toml`. Full architecture: [Doc's/PROJECT_LAYOUT.md](Doc's/PROJECT_LAYOUT.md). Data flows: [Doc's/flow_map.md](Doc's/flow_map.md).

## Prerequisites

- **Rust**: [Install rustup](https://rustup.rs/) (stable channel).
- **Ollama**: Running on your network (default: `192.168.1.149:11434`), model `llama3` pulled and serving. Bind to `0.0.0.0` for remote access.
- **SSL Certs**: Self-signed in `certs/` for HTTPS web server.
- **Dirs**: `logs/`, `certs/` (auto-created if missing).

## Installation

1. **Clone**:
   ```
   git clone https://github.com/yourusername/bot.git
   cd bot
   ```

2. **Build**:
   ```
   cargo build --release
   ```

3. **SSL Certs** (if missing):
   ```
   mkdir -p certs
   openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
     -keyout certs/server.key -out certs/server.crt \
     -subj "/CN=localhost"
   ```

4. **Configure** `config.toml`:
   - Update Ollama `host`/`port`/`model`.
   - Adjust ports, log paths, timeouts.

5. **Test Ollama**:
   ```
   cargo run --bin test_ollama
   ```
   Should confirm Ollama is reachable.

## Running

```
cargo run
```

- Boots kernel: bus, memory manager, skills, hooks, hy_evo, cpu executor, time scheduler for heartbeat, web server (`https://localhost:8443`), terminal.
- Heartbeats/logs routed to Ollama via bus for analysis.

**Daemon Mode** (future): Use systemd/supervisor.

## Linux Server Deployment (systemd)

### 1. Build Release Binary
```bash
cargo build --release
```

### 2. Install as Systemd Service
Run the installer (creates `/etc/systemd/system/bot.service` and enables it):
```bash
sudo ./target/release/bot_installer
```

Or manually install:
```bash
cargo run --bin installer
```

### 3. Manage the Bot Service

| Command | Description |
|---------|-------------|
| `sudo systemctl start bot` | Start the bot |
| `sudo systemctl stop bot` | Stop the bot |
| `sudo systemctl restart bot` | **Restart the bot** |
| `sudo systemctl status bot` | Check status and recent logs |
| `sudo journalctl -u bot -f` | Follow live logs |
| `sudo systemctl enable bot` | Enable auto-start on boot |
| `sudo systemctl disable bot` | Disable auto-start |

After making code changes, rebuild and restart:
```bash
cargo build --release
sudo systemctl restart bot
```

## How to Communicate

### Terminal CLI
- Starts with `AgentOS> ` prompt.
- Type queries → Bus → CPU → Ollama → Response displayed.
- Chats logged to `logs/chat_log.md`.

**Example**:
```
AgentOS> What's the system status?
[Ollama Response...]
```

### Web Interface
- Visit `https://localhost:8443` (accept cert warning).
- **Chat Tab**: Real-time LLM chat via WebSockets.
- **Logs Tab**: Live view of `error_log.md`, `chat_log.md`, `hartbeat_log.md`, `bus_log.md`.
- Responsive UI for monitoring.

### Programmatic (Advanced)
- Publish to bus: e.g., `to: cpu, data: "your prompt"`.
- Subscribe for responses.

## Configuration (`config.toml`)

Key sections:
```toml
[ollama]
host = "192.168.1.149"  # Your Ollama server
port = 11434
model = "llama3"
timeout = 30  # Increase for slow setups
retry_count = 3

[web_interface]
https_port = 8443

[logging]
error_log = "logs/error_log.md"  # Timestamped errors!

# Full file: config.toml
```

## Logs & Monitoring

- `logs/error_log.md`: `[YYYY-MM-DDTHH:MM:SSZ ERROR] message`
- `logs/chat_log.md`: Conversations.
- `logs/hartbeat_log.md`: Status data.
- `logs/bus_log.md`: All messages (`to`, `from`, `data`).

Tail: `tail -f logs/*.md`

## Troubleshooting

| Issue | Fix |
|-------|-----|
| **Ollama Timeout** | Increase `timeout`/retries in config; run `cargo run --bin test_ollama`; check Ollama logs. |
| **Web Cert** | Accept warning or use proper certs. |
| **Build Fail** | `rustup update`; `cargo clean`; check Windows paths. |
| **No Bus Messages** | Check `bus_log.md`; verify subscriptions. |
| **Slow Ollama** | Use lighter model; optimize hardware. |
| **Memory Issues** | Adjust working/episodic max sizes in `main.rs` (currently 50/1000). |

## Development & Extensibility

- **Add Skill**: Implement `SkillInterface` in `src/skills/`, register in `main.rs`.
- **Add Hook**: Implement `HookInterface` for phases like init/post-execution.
- **HyEvo Workflow**: Define nodes (Skill, Llm, Code) in `src/hy_evo/workflow.rs`, evolve via reflection.
- **Memory**: Extend episodic/vector for custom storage.
- **New Module**: Add `src/new_module/`, subscribe/publish to bus, integrate with CPU.
- **Tests**: `cargo test`.
- **TBD**: Full impl of A2A, Cron, MCP, Gemini.

**Dir Structure**: See [PROJECT_LAYOUT.md](PROJECT_LAYOUT.md). **Flows**: See [flow_map.md](flow_map.md).

## Dependencies (`Cargo.toml`)

- `tokio`, `axum` (web/WS), `ollama-rs`, `serde`, `log`, etc.
- Update: `cargo update`.

## License

Custom License (Non-Commercial). See [LICENSE](LICENSE) for details.

---

⭐ **Star on GitHub** | **Issues?** Open one! | Powered by Rust & Ollama.

*(README auto-generated/updated based on project scan.)