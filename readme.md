# Helix

**Helix** is a modular Rust-based AI agent framework — an "operating system" for
intelligent agents powered by local LLMs (Ollama) and cloud LLMs (Google Gemini).

It provides a central message bus, CPU execution engine, memory system, LLM router,
plugin architecture, and a web UI, all running as a systemd service on Linux.

---

## Features

- **Multi-backend LLM routing** — Ollama (multiple instances) + Google Gemini, selectable per-message
- **Web UI** — chat interface, config editor, log viewer at `https://<server>:8443`
- **Memory system** — working, episodic, and vector memory with context injection
- **Reasoning engine** — hypothesis → plan → execute cycle with self-correction
- **Tool system** — extensible tools callable by LLMs and slash commands
- **HyEvo engine** — evolutionary workflow optimisation
- **OKF integration** — Open Knowledge Format librarian (optional)
- **Starlink-resilient** — all network calls have timeouts, retries, and exponential back-off

---

## Quick Start (server deploy)

```bash
# 1. Clone
git clone https://github.com/micro-tech/grok-cli ~/helix
cd ~/helix

# 2. Add API keys
echo "GEMINI_API_KEY=your_key_here" > .env
echo "GEMINI_MODEL=gemini-2.0-flash" >> .env

# 3. Build
cargo build --release

# 4. Install (copies binary, deploys config, enables systemd service)
cargo run --bin installer
```

Web UI: `https://<server-ip>:8443`

See **[Deployment Guide](docs/deployment.md)** for the full workflow including
branch strategy, update process, and troubleshooting.

---

## Update

```bash
cd ~/helix && git pull && cargo build --release && sudo systemctl restart helix
```

---

## Configuration

All configuration lives in `config.toml` (runtime directory) and `.env` (secrets, git-ignored).

See **[Configuration Reference](docs/configuration.md)**.

Key sections:

| Section              | Purpose                              |
| -------------------- | ------------------------------------ |
| `[[ollama]]`         | One entry per Ollama backend (array) |
| `[gemini]`           | Gemini model (key goes in `.env`)    |
| `[web]`              | Port and TLS settings                |
| `[logging]`          | Log file paths                       |
| `[reasoning]`        | Reasoning engine settings            |
| `[ollama_keepalive]` | Keep models warm in GPU memory       |

---

## Documentation

| Topic                   | File                                             |
| ----------------------- | ------------------------------------------------ |
| Deployment & Install    | [docs/deployment.md](docs/deployment.md)         |
| Configuration Reference | [docs/configuration.md](docs/configuration.md)   |
| Systemd & Docker        | [docs/docker_systemd.md](docs/docker_systemd.md) |
| Logging                 | [docs/logging.md](docs/logging.md)               |
| Email Setup             | [docs/email_setup.md](docs/email_setup.md)       |
| MCP                     | [docs/mcp.md](docs/mcp.md)                       |

---

## Development

```bash
# Run tests
cargo test

# Check for warnings/errors
cargo clippy

# Local dev (no systemd needed)
cargo run
```

**Branch workflow:**

- `prerelease` — development and testing
- `master` — stable, what the server runs

---

## License

Custom Non-Commercial License. See [LICENSE](LICENSE).

---

**Built with Rust · Tokio · Ollama · Gemini · Helix AI Agent**  
_Author: john mcconnell — john.microtech@gmail.com_
