# CLI Commands Reference

This document lists all supported commands for both the terminal and UNIX socket interfaces.

## Core Commands
| Command | Description |
|---------|-------------|
| `help` | List available commands |
| `ping` | Connectivity + latency test |
| `health` / `status` | System health and uptime |
| `version` | Version information |
| `ask` | Send a prompt to the LLM |
| `chat` | Multi-turn chat |

## File Operations
| Command | Description |
|---------|-------------|
| `upload` | Upload file data |
| `download` | Request file download |
| `checksum` | SHA-256 of a file |
| `resume-offset` | Get resume position |

## Plugins & Extensibility
| Command | Description |
|---------|-------------|
| `plugins` | List installed plugins |
| `run-plugin` | Execute a plugin |

## Diagnostics
| Command | Description |
|---------|-------------|
| `metrics` | Export usage metrics |
| `cpu-debug` | CPU state snapshot |
| `skill-debug` | Skill registry snapshot |

## Memory & Tools
| Command | Description |
|---------|-------------|
| `memory-list` | List memory entries |
| `memory-dump` | Dump all beliefs |
| `tools` | List available tools |
| `reload-tools` | Reload tool definitions |

## Error Handling
All commands return structured JSON. Errors use the format:
```json
{"error": {"code": "SCHEMA_ERROR", "message": "..."}}
```

See also: [UNIX Socket CLI](unix_socket.md), [Plugins](plugins.md)
