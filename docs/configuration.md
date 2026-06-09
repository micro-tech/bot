# Configuration (`config.toml`)

Agent OS is configured via `config.toml` in the project root.

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
path = "/tmp/agentos.sock"
mode = 0o660
```

| Key | Description | Default |
|-----|-------------|---------|
| `path` | Socket file location | `/tmp/agentos.sock` |
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

## Cross-Platform Paths

Some paths are expanded automatically:

| Platform | Example Expanded Path |
|----------|-----------------------|
| Windows | `%APPDATA%\bot\logs\error_log.md` |
| Linux/macOS | `~/.bot/logs/error_log.md` |

## Example Full File

See the root `config.toml` for a complete working example.

## Related
- [UNIX Socket CLI](unix_socket.md)
- [Logging System](logging.md)
- [Web Interface](web_interface.md)
