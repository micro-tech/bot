# Configuration (`config.toml`)

Helix is configured via `config.toml` in the runtime working directory
(`/home/cobble/helix/` on the server, or the project root during local dev).

Environment variable overrides for secrets go in `.env` (git-ignored).

---

## `[helix]`

```toml
[helix]
name = "Helix"
```

| Key    | Description                |
| ------ | -------------------------- |
| `name` | Display name for the agent |

---

## `[[ollama]]` — Multiple backends (array)

Multiple Ollama instances can be declared. Each becomes a selectable LLM in the web UI.

```toml
[[ollama]]
name   = "server"
url    = "http://192.168.1.149:11434"
model  = "qwen3.5:0.8b"

[[ollama]]
name   = "desktop"
url    = "http://192.168.1.196:11434"
model  = "gemma4:e4b"
```

| Key     | Description                                                             |
| ------- | ----------------------------------------------------------------------- |
| `name`  | Unique ID — used as the bus topic (`ollama_<name>`) and UI button label |
| `url`   | Full base URL of the Ollama instance                                    |
| `model` | Default model to use for requests                                       |

> **Remote Ollama tip:** The remote machine must run `OLLAMA_HOST=0.0.0.0 ollama serve`
> to accept connections from other hosts.

---

## `[gemini]`

```toml
[gemini]
model = "gemini-2.0-flash"
```

| Key     | Description                                                          |
| ------- | -------------------------------------------------------------------- |
| `model` | Gemini model name. Overridden at runtime by `GEMINI_MODEL` in `.env` |

**Valid models:** `gemini-2.0-flash`, `gemini-1.5-flash`, `gemini-1.5-pro`, `gemini-2.5-pro-preview-03-25`

API key is set in `.env` only — never in `config.toml`:

```env
GEMINI_API_KEY=your_key_here
GEMINI_MODEL=gemini-2.0-flash   # optional override
```

---

## `[web]`

```toml
[web]
port        = 8443
tls_enabled = true
cert_path   = "cert.pem"
key_path    = "key.pem"
```

| Key           | Description                                            | Default    |
| ------------- | ------------------------------------------------------ | ---------- |
| `port`        | HTTPS port the web UI listens on                       | `8443`     |
| `tls_enabled` | Enable TLS                                             | `true`     |
| `cert_path`   | Path to TLS certificate (relative to WorkingDirectory) | `cert.pem` |
| `key_path`    | Path to TLS private key (relative to WorkingDirectory) | `key.pem`  |

The installer generates self-signed certs automatically if none are found.
Access the UI at `https://<server-ip>:8443`.

---

## `[logging]`

```toml
[logging]
chat_log     = "logs/chat_log.md"
error_log    = "logs/error_log.md"
bus_log      = "logs/bus_log.md"
hartbeat_log = "logs/hartbeat_log.md"
```

All paths are **relative to the service WorkingDirectory** (`/home/cobble/helix/`).
Do not use absolute paths — they are rejected by the security sandbox.

---

## `[heartbeat]`

```toml
[heartbeat]
interval_seconds = 300
```

| Key                | Description                                  |
| ------------------ | -------------------------------------------- |
| `interval_seconds` | How often the heartbeat tick fires (seconds) |

---

## `[ollama_keepalive]`

Keeps the Ollama model loaded in GPU/CPU memory to avoid cold-start delays.

```toml
[ollama_keepalive]
preload       = true
interval_secs = 240
```

| Key             | Description                                                                 |
| --------------- | --------------------------------------------------------------------------- |
| `preload`       | Send a warm-up request at startup                                           |
| `interval_secs` | Keepalive ping interval (default 240 s, just under Ollama's 5-min eviction) |

Can also be set in `.env`:

```env
OLLAMA_PRELOAD=true
OLLAMA_KEEP_ALIVE_SECS=240
```

---

## `[reasoning]`

```toml
[reasoning]
enabled                  = true
default_goal             = "Improve agent reliability, safety, and self-correction"
cycle_interval_ticks     = 20
metrics_interval_ticks   = 40
health_check_interval_ticks = 100
```

| Key                           | Description                                             |
| ----------------------------- | ------------------------------------------------------- |
| `enabled`                     | Master switch for the reasoning engine                  |
| `default_goal`                | Starting goal injected at startup                       |
| `cycle_interval_ticks`        | How often the reasoning cycle runs (in heartbeat ticks) |
| `metrics_interval_ticks`      | How often reasoning metrics are published to the bus    |
| `health_check_interval_ticks` | How often the reasoning health check runs               |

---

## `[helix.okf]`

Open Knowledge Format integration (disabled by default).

```toml
[helix.okf]
enabled              = false
server_url           = "http://localhost:8080"
auto_reload          = true
index_file           = "okf_index.json"
poll_interval_secs   = 60
request_timeout_secs = 30
```

| Key                    | Description                               |
| ---------------------- | ----------------------------------------- |
| `enabled`              | Master switch — set to `true` to activate |
| `server_url`           | OKF server base URL                       |
| `auto_reload`          | Reload index when the server changes      |
| `poll_interval_secs`   | How often to poll for changes             |
| `request_timeout_secs` | HTTP timeout for OKF requests             |

---

## `.env` reference

The `.env` file (git-ignored) holds secrets and runtime overrides.

```env
# Required
GEMINI_API_KEY=your_gemini_api_key_here

# Optional overrides
GEMINI_MODEL=gemini-2.0-flash
OLLAMA_PRELOAD=true
OLLAMA_KEEP_ALIVE_SECS=240
```

Location after install: `/home/cobble/helix/.env`

---

## Related

- [Deployment & Install](deployment.md)
- [Logging](logging.md)
- [Docker & Systemd](docker_systemd.md)
