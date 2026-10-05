# Configuration (`config.toml`)

Helix is configured via `config.toml` in the project root.

## Main Sections

### `[ollama]`
Controls the connection to the local or remote LLM.

```toml
[ollama]
host = "192.168.1.149"
port = 11434
model = "llama3"
timeout = 30
retry_count = 3
```

| Key | Description | Default |
|-----|-------------|---------|
| `host` | Ollama server address | `127.0.0.1` |
| `port` | Ollama API port | `11434` |
| `model` | Default model name | `llama3` |
| `timeout` | Request timeout in seconds | `30` |
| `retry_count` | Number of retries on failure | `3` |

### `[web_interface]`
HTTPS web server settings.

```toml
[web_interface]
https_port = 8443
```

### `[socket]`
UNIX domain socket CLI settings.

```toml
[socket]
path = "/tmp/helix.sock"
mode = 0o660
```

| Key | Description | Default |
|-----|-------------|---------|
| `path` | Socket file location | `/tmp/helix.sock` |
| `mode` | File permissions (octal) | `0o660` |

### `[logging]`
Controls where logs are written.

```toml
[logging]
error_log = "logs/error_log.md"
chat_log = "logs/chat_log.md"
bus_log = "logs/bus_log.md"
```

### `[memory]`
Memory system limits.

```toml
[memory]
working_max = 50
episodic_max = 1000
```

### `[shell]`
Local shell tool (`run_shell`) — lets the agent run commands on its own
server via `/bin/sh -c`. **Disabled by default**; explicit opt-in required.
The SSH server (task 192) executes sessions through this same policy and
fails closed when it is off.

```toml
[shell]
enabled = false            # master switch — the tool refuses to run unless true
default_timeout_secs = 60  # per-command timeout when the caller doesn't set one (1-3600)
workdir = "."              # base directory commands run in (relative to Helix's CWD)
allow_absolute_paths = false  # when false, workdir args must stay under `workdir`
max_output_bytes = 32768   # per-stream (stdout/stderr) capture cap
```

Protections (all enforced, not just the denylist): two-layer
destructive-shell analysis (argv[0] program-name matching + substring
denylist — shared with the ACP surface, implemented once in
`src/tools/shell_security.rs`); timeout kills the whole process group;
stdout/stderr truncated with notice; stdin is `/dev/null` (no TTY, so
interactive prompts fail fast); working directory confined to `workdir`.
Defense-in-depth framing: the denylist sits *behind* the agent's own
judgment, never as the sole protection.

## Cross-Platform Paths

Some paths are expanded automatically:

| Platform | Example Expanded Path |
|----------|-----------------------|
| Windows | `%APPDATA%\helix\logs\error_log.md` |
| Linux/macOS | `~/.helix/logs/error_log.md` |

## Example Full File

See the root `config.toml` for a complete working example.

## Related
- [UNIX Socket CLI](unix_socket.md)
- [Logging System](logging.md)
- [Web Interface](web_interface.md)
