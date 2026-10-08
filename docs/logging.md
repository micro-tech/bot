# Logging

Helix writes human-readable Markdown log files relative to its working directory
(`/home/cobble/helix/logs/` on the server).

---

## Log Files

| File                   | Purpose                                           |
| ---------------------- | ------------------------------------------------- |
| `logs/chat_log.md`     | All user messages and LLM replies with timestamps |
| `logs/error_log.md`    | Errors and important system events                |
| `logs/bus_log.md`      | Internal bus message traffic                      |
| `logs/hartbeat_log.md` | Heartbeat ticks and agent status signals          |

Paths are configured in `config.toml` under `[logging]` and must be relative:

```toml
[logging]
chat_log     = "logs/chat_log.md"
error_log    = "logs/error_log.md"
bus_log      = "logs/bus_log.md"
hartbeat_log = "logs/hartbeat_log.md"
```

---

## Log Format

```
[2025-10-04 14:23:11] User: Hello, what can you do?
[2025-10-04 14:23:13] gemini: I can help with reasoning, planning, tool use...
```

---

## Viewing Logs

**Via the web UI** — open the Logs tab at `https://<server-ip>:8443`

**Via terminal:**

```bash
# Live tail via journalctl (service output)
journalctl -u helix -f

# Read a log file directly
cat ~/helix/logs/chat_log.md
tail -f ~/helix/logs/error_log.md
```

---

## Log Rotation

Logs are reset (not rotated) each time the installer runs. Back them up first if needed:

```bash
cp ~/helix/logs/chat_log.md ~/helix/logs/chat_log_backup_$(date +%F).md
```

---

## Related Code

- `src/utils.rs` — `log_to_file()` helper
- `src/io/web_server/mod.rs` — chat log writes + web UI log viewer endpoints (`/logs/chat`, `/logs/error`)
