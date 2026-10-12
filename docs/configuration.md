# Configuration (`config.toml`) — task 245

Helix is configured via `config.toml` in the runtime directory (default:
the directory Helix is launched from; the R630 runs from `/home/cobble/helix`).

**Conventions used below:**
- **Required** = Helix fails or misbehaves without it.
- **Defaulted** = safe default applies when omitted (shown in `Default` column).
- Every default was verified against the serde structs / parsing code; the
  section list was generated from the shipped `config.toml` so it can't drift.

**Secrets:** `config.toml` is git-tracked with `skip-worktree` on the R630
so pulls never clobber it. File should be mode `0600`. `HELIX_WEB_TOKEN`
env var overrides `[web] auth_token`.

---

## `[helix]`

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `name` | No | `"Helix"` | Agent display name |

## `[[ollama]]` (repeatable — one per Ollama instance)

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `name` | **Yes** | — | Instance id (`server`, `desktop`); used in backend ids `ollama_<name>` |
| `url` | **Yes** | — | Base URL, e.g. `http://192.168.1.149:11434` |
| `model` | **Yes** | — | Model tag, e.g. `gemma4:e4b` |

## `[gemini]`

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `model` | No | `"gemini-3.6-flash"` | Gemini model. `GEMINI_MODEL` env var overrides. API key from `GEMINI_API_KEY` (`.env`) |

## `[web]`

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `bind` | No | `"127.0.0.1"` | Bind address. `0.0.0.0` only with `auth_token` set |
| `port` | No | `8443` | HTTPS port |
| `auth_token` | No (but set it if bound beyond localhost) | `""` (open) | Token for mutating endpoints; `HELIX_WEB_TOKEN` overrides |
| `tls_enabled` | No | `false` | TLS on/off |
| `cert_path` | No | `"cert.pem"` | TLS cert |
| `key_path` | No | `"key.pem"` | TLS key |

## `[logging]`

All paths must be relative starting with `logs/`.

| Key | Required? | Default |
|-----|-----------|---------|
| `chat_log` | No | `"logs/chat_log.md"` |
| `error_log` | No | `"logs/error_log.md"` |
| `bus_log` | No | `"logs/bus_log.md"` |
| `hartbeat_log` | No | `"logs/hartbeat_log.md"` |

## `[heartbeat]`

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `interval_seconds` | No | `300` | Heartbeat tick interval |

## `[ollama_keepalive]` (task 89)

Keeps the Ollama model warm in GPU memory.

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `preload` | No | `true` | Dummy request at startup to pre-warm |
| `interval_secs` | No | `240` | Heartbeat interval (0 = disable). Env: `OLLAMA_KEEP_ALIVE_SECS` |

## `[reasoning]`

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `enabled` | No | `true` | Reasoning engine on/off |
| `default_goal` | No | `"Improve agent reliability, safety, and self-correction"` | Fallback goal |
| `cycle_interval_ticks` | No | `20` | Reasoning cycle cadence |
| `metrics_interval_ticks` | No | `40` | Metrics cadence |
| `health_check_interval_ticks` | No | `100` | Health-check cadence |

## `[helix.okf]` (task 164)

Helix as its own OKF server.

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `enabled` | No | `false` | Master switch |
| `server_url` | No | `"http://localhost:8080"` | Self-fetch URL |
| `auto_reload` | No | `true` | Reload on webhook |
| `index_file` | No | `"okf_index.json"` | Index path |
| `poll_interval_secs` | No | `60` | Poll cadence |
| `request_timeout_secs` | No | `30` | Fetch timeout |
| `trace_file` | No | `"okf_traces.jsonl"` | Trace append target |
| `auth_token` | No | `""` | Optional Bearer <redacted> write endpoints |

## `[mcp]` / `[[mcp.servers]]`

External MCP servers over stdio. Disabled unless `[mcp] enabled = true`.

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `[mcp] enabled` | No | `false` | Master switch |
| `name` | **Yes** (per server) | — | Namespace: tools appear as `mcp__<name>__<tool>` |
| `command` | **Yes** (per server) | — | Executable to spawn |
| `args` | No | `[]` | Extra argv |
| `[mcp.servers.env]` | No | — | Extra env vars (e.g. `GOOGLE_CLIENT_ID`) |

## `[helix.commander]` (task 215)

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `enabled` | No | `true` | Commander delegation mode. `HELIX_COMMANDER_ENABLED=0/1` overrides |

## `[helix.persona]` (task 233)

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `self_author` | No | `false` | **Opt-in.** Lets sub-agents draft persona updates (land as `.pending`, John approves) |

## `[router]` (tasks 217, 236)

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `daily_api_cap_usd` | No | `5.00` | Daily paid-API spend cap (metered tokens, task 236) |
| `cap_block_on_exceed` | No | `false` | `true` = block paid calls over cap; `false` = warn + continue (default; John's call to flip) |

## `[shell]`

Local shell tool (`run_shell`). **Disabled by default**; explicit opt-in.

| Key | Required? | Default | Description |
|-----|-----------|---------|-------------|
| `enabled` | No | `false` | Master switch |
| `default_timeout_secs` | No | `60` | Per-command timeout (1–3600) |
| `workdir` | No | `"."` | Base dir for commands |
| `allow_absolute_paths` | No | `false` | When false, paths must stay under `workdir` |
| `max_output_bytes` | No | `32768` | Per-stream capture cap |

---

## Minimal working example

```toml
[helix]
name = "Helix"

[[ollama]]
name = "server"
url = "http://192.168.1.149:11434"
model = "llama3.2:1b"

[gemini]
model = "gemini-3.6-flash"

[web]
bind = "127.0.0.1"
port = 8443
auth_token = "set-me"

[logging]
chat_log = "logs/chat_log.md"
error_log = "logs/error_log.md"
bus_log = "logs/bus_log.md"
hartbeat_log = "logs/hartbeat_log.md"
```
