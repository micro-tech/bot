# Logging System

Agent OS uses a human-readable Markdown logging system.

## Log Files
| File | Purpose |
|------|---------|
| `logs/error_log.md` | All errors and important events (12-hour timestamps) |
| `logs/chat_log.md` | Chat history |
| `logs/bus_log.md` | Internal bus messages |
| `logs/hartbeat_log.md` | Heartbeat / status signals |

## Error Log Format
```
[2025-04-05 02:34:11 PM] Checksum mismatch on /tmp/file.bin
```

## Cross-Platform
- Windows: `%APPDATA%\bot\logs\error_log.md`
- Linux/macOS: `~/.bot/logs/error_log.md`

## Related Code
- `src/utils.rs` (`log_to_file`)
- `src/io/unix_cli/*` (integrated logging on errors)
