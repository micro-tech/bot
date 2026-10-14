# Helix

**Helix** is a self-hosted Rust AI agent — a conversational commander that
delegates to sub-agents running on local Ollama machines or the Gemini API.
Runs on the Dell R630; chat with it from the web UI.

---

## How it works

- **Commander pattern** — pick an LLM/machine in the web UI and chat with it
  as the commander; it gives the marching orders. No routing tables.
  ([docs](docs/configuration.md#helixcommander-task-215))
- **Sub-agent delegation** — the commander spins up sub-agents (`planner`,
  `builder`, `reviewer`, `ui`, …), each on whichever machine fits, with a
  live working/idle panel in the UI. Sub-agents can't self-spawn (max-depth
  guardrail by construction).
- **Persona system** — `agent.md` + `agents/<role>.md` give each role a
  persona, job description, and tool allowlist (least privilege). Personas
  are self-authoring (opt-in): drafts land as `.pending` for human approval.
- **Prompt-injection defense** — untrusted web/email content is tagged and
  validated before tool execution: shell/file-write calls block on
  suspicious args, everything else warns-and-logs.
  ([docs](docs/injection_defense.md))
- **Real usage metering** — per-backend token counts from Ollama/Gemini
  (not estimates), daily spend cap, `GET /ops/usage`.
- **Ops buttons** — self-update (`POST /ops/update`) and restart
  (`POST /ops/restart`) from the web UI, auth-gated.
- **MCP / Gmail** — external tools via MCP stdio servers (Gmail + Calendar
  through `google-mcp`); replies stay draft-for-review.
  ([docs](docs/configuration.md#mcp--mcpservers))
- **Email channel** — send `hi helix` + `key:` line to trigger work orders
  from anywhere.
- **Web UI** — three-column chat, sub-agent panel, log viewers, config
  editor. ([docs](docs/configuration.md#web))

## Quick start

```bash
git clone https://github.com/micro-tech/bot.git
cd bot
# config.toml ships with sane defaults; edit it for your backends/secrets
cargo build --release
./target/release/helix
```

Then open the web UI (default `https://127.0.0.1:8443`; set `[web] auth_token`
if bound beyond localhost). Full config reference:
[docs/configuration.md](docs/configuration.md).

## Docs

| Topic | File |
|-------|------|
| Configuration (every section) | [docs/configuration.md](docs/configuration.md) |
| Injection defense | [docs/injection_defense.md](docs/injection_defense.md) |
| Backup & restore | [docs/backup-restore.md](docs/backup-restore.md) |
| Eval harness | [evals/README.md](evals/README.md) |
| Changelog | [CHANGELOG.md](CHANGELOG.md) |

## Development

- Run tests: `cargo test` (lib) · `cargo test --test evals` (behavioral evals)
- Branch workflow: `PreRelese` = work/test, `master` = release

---

**Built with Rust + Tokio + Ollama · Helix AI Agent**
