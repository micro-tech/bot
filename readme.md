Agent OS Bot

## Overview

**Agent OS** is a Rust-based modular framework designed as an "operating system" for AI agents, primarily powered by local LLMs like Ollama. It features:

- **Central Bus System**: Decoupled inter-component communication using publish-subscribe messaging.
- **Heartbeat Module**: Periodic system status signals sent to Ollama for analysis.
- **Ollama Integration**: Direct API communication for LLM tasks (with Gemini/Grok placeholders).
- **Interfaces**:
  - HTTPS Web Interface (chat, logs, WebSockets).
  - Terminal CLI for direct interaction.
- **Logging**: Comprehensive Markdown logs for errors, chats, bus transactions, heartbeats.
- **Modular Design**: Components like A2A, Cron, MCP, Memory, Tooling, Hooks (some TBD).
- **Resilient Networking**: Retries, timeouts, exponential backoff for unreliable connections (e.g., Starlink).
- **Test Binary**: `test_ollama` to verify Ollama connectivity.

The system acts as a kernel (`src/main.rs`), coordinating subsystems via the bus. All settings in `config.toml`. Full architecture: [PROJECT_LAYOUT.md](PROJECT_LAYOUT.md).

## Prerequisites

- **Rust**: [Install rustup](https://rustup.rs/) (stable channel).
- **Ollama**: Running on your network (default: `192.168.1.149:11434`), model `llama3` pulled and serving. Bind to `0.0.0.0` for remote access.
- **SSL Certs**: Self-signed in `certs/` for HTTPS web server.
- **Dirs**: `logs/`, `certs/`, `src/memory/` (auto-created if missing).

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

- Boots kernel: bus, heartbeat (60s interval), web server (`https://localhost:8443`), terminal.
- Heartbeats/logs routed to Ollama via bus.

**Daemon Mode** (future): Use systemd/supervisor.

## How to Communicate

### Terminal CLI
- Starts with `AgentOS> ` prompt.
- Type queries → Bus → Ollama → Response displayed.
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
- Publish to bus: e.g., `to: ollama, data: "your prompt"`.
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

## Development & Extensibility

- **Add Module**: New `src/my_module/`, subscribe/publish to bus.
- **Hooks/Tools**: LLM-triggered scripts (`src/hooks/`, `src/tooling/`).
- **Memory**: JSON stores (`src/memory/short_term/session_data.json`).
- **Tests**: `cargo test`.
- **TBD**: A2A, Cron, MCP, Gemini full impl.

**Dir Structure**: See [PROJECT_LAYOUT.md](PROJECT_LAYOUT.md).

## Dependencies (`Cargo.toml`)

- `tokio`, `axum` (web/WS), `ollama-rs`, `serde`, `log`, etc.
- Update: `cargo update`.

## License

MIT.

---

⭐ **Star on GitHub** | **Issues?** Open one! | Powered by Rust & Ollama.

*(README auto-generated/updated based on project scan.)
