# Tools & Skills Reference

> **Author:** john mcconnell — john.microtech@gmail.com  
> **Repository:** https://github.com/microtech/grok-cli  
> Last updated: 2026-04-12

This document covers every tool and skill available in the bot.  
Tools are callable in three ways:

| Caller | How |
|--------|-----|
| **Ollama (LLM)** | Model emits a `tool_calls` JSON block; the agent loop executes it and feeds the result back automatically |
| **Chat slash command** | Type `/toolname args` directly in the web UI chat box |
| **CPU skill request** | Internal bus message `{"type":"skill_request","skill":"name","args":{}}` |

All three paths share a single implementation in `src/tools/`.

---

## File Tools

### `read_log`

Read the tail of a log file from the `logs/` directory.

**Parameters**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `log_file` | string | yes | — | Relative path, e.g. `logs/chat_log.md` |

**Security:** Only paths inside `logs/` are allowed. Attempting to read `../secrets` or any path outside `logs/` returns a security error.

**Returns:** Last 2 000 characters of the file, or an error message.

**Examples**
```json
{"log_file": "logs/chat_log.md"}
{"log_file": "logs/error_log.md"}
{"log_file": "logs/email_outbox.md"}
```

**Slash command**
```
/log               (reads logs/chat_log.md by default)
/log error_log.md  (reads logs/error_log.md)
```

---

### `write_note`

Save a markdown note to the `notes/` directory. The title becomes the filename.

**Parameters**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `title` | string | yes | Short title — used as the filename (special chars sanitised to `_`) |
| `content` | string | yes | Note body in markdown format |

**Returns:** Confirmation with the saved path, or an error.

**Example**
```json
{"title": "Meeting notes", "content": "## 2026-04-12\n\n- Discussed email integration\n- Next: add scheduler"}
```
Saves to `notes/Meeting_notes.md`.

---

### `read_note`

Read a previously saved note.

**Parameters**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `title` | string | yes | Title of the note to read |

**Returns:** Note contents, or a list of available notes if the title is not found.

**Slash command**
```
/note Meeting notes
```

---

### `list_notes`

List all `.md` files in the `notes/` directory.

**Parameters:** None

**Returns:** Sorted bullet list of note titles, or a message if no notes exist.

**Slash command**
```
/notes
```

---

## Email Tools

> **Setup required** — see [`Doc's/email_setup.md`](email_setup.md) for full configuration.

### `send_email`

Send an email via SMTP. Falls back to `logs/email_outbox.md` when SMTP is not configured.

**Parameters**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `to` | string | yes | Recipient email address |
| `subject` | string | yes | Subject line |
| `body` | string | yes | Plain-text body |

**Environment variables (`.env`)**

```
SMTP_HOST=smtp.gmail.com
SMTP_PORT=587            # 587 = STARTTLS  |  465 = SSL
SMTP_USER=you@gmail.com
SMTP_PASSWORD=app-password
SMTP_FROM=you@gmail.com  # optional — defaults to SMTP_USER
```

**Behaviour**
- If SMTP env vars are set → sends the email immediately via SMTP and returns a confirmation.
- If SMTP env vars are missing → appends the email to `logs/email_outbox.md` and returns a warning.
- Port `465` uses direct TLS (SSL); any other port uses STARTTLS.

**Returns:** `✅ Email sent…` on success, or `❌ SMTP error: …` with a hint on failure.

**Example**
```json
{
  "to": "admin@example.com",
  "subject": "Server alert",
  "body": "Disk usage above 90% on /dev/sda1"
}
```

---

### `read_email`

Read recent emails from an IMAP folder. Returns subject, sender, and date.

**Parameters**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `folder` | string | no | `INBOX` | IMAP folder name |
| `count` | integer | no | `5` | Number of messages to return (max 20) |

**Environment variables (`.env`)**

```
IMAP_HOST=imap.gmail.com
IMAP_PORT=993
IMAP_USER=you@gmail.com
IMAP_PASSWORD=app-password
```

**Returns:** Formatted list of the most recent N messages (newest first), each showing:
- Sender (`From:`)
- Subject (`Subject:`)
- Date (`Date:`)

**Example**
```json
{"folder": "INBOX", "count": 10}
```

**Slash command**
```
/email          (reads last 5 from INBOX — add via web_server if needed)
```

---

### `check_inbox`

Check the total and recent message count for an IMAP folder.

**Parameters**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `folder` | string | no | `INBOX` | IMAP folder name |

**Returns:** `📬 INBOX — 42 total messages, 3 recent`

**Example**
```json
{"folder": "INBOX"}
```

---

## System Tools

### `system_status`

Return the current system health summary.

**Parameters:** None

**Returns:**
- Current Unix timestamp
- Size of `logs/chat_log.md`, `logs/error_log.md`, `logs/email_outbox.md`
- Number of saved notes
- Number of stored beliefs

**Slash command**
```
/status
```

---

### `list_tools`

Print the complete catalogue of available tools and slash commands.

**Parameters:** None

**Slash command**
```
/tools
```

---

## Memory / Belief Tools

### `get_beliefs`

Read all agent beliefs from `beliefs.json`. Beliefs are key/value facts that persist across restarts.

**Parameters:** None

**Returns:** Pretty-printed JSON of all beliefs, or a message if none are stored.

**Slash command**
```
/beliefs
```

---

### `set_belief`

Store or update an agent belief in `beliefs.json`.

**Parameters**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `key` | string | yes | Belief key (short identifier, e.g. `owner_name`) |
| `value` | string | yes | Belief value |

**Returns:** `✅ Belief 'key' = 'value' saved to beliefs.json`

**Example**
```json
{"key": "server_location", "value": "rack-2 unit-4"}
```

**Slash command**
```
/set server_location=rack-2 unit-4
```

---

## Slash Command Quick Reference

Type any of these directly into the chat box — no LLM round-trip required.

| Command | Tool called | Description |
|---------|-------------|-------------|
| `/status` | `system_status` | System health snapshot |
| `/tools` | `list_tools` | List all tools |
| `/notes` | `list_notes` | List saved notes |
| `/note <title>` | `read_note` | Read a specific note |
| `/beliefs` | `get_beliefs` | Show agent beliefs |
| `/set key=value` | `set_belief` | Store a belief |
| `/log` | `read_log` | Tail `logs/chat_log.md` |
| `/log <file>` | `read_log` | Tail a specific log file |
| `/help` | — | List all slash commands |

---

## Adding a New Tool

1. Add the implementation function to the appropriate file in `src/tools/`:
   - File/note operations → `src/tools/file_tools.rs`
   - Email → `src/tools/email_tools.rs`
   - System/memory → `src/tools/system_tools.rs`
   - New category → create `src/tools/your_category.rs` and add `pub mod your_category;` to `src/tools/mod.rs`

2. Add a dispatch arm in `src/tools/mod.rs` `execute()`:
   ```rust
   "your_tool" => your_category::your_tool(args),
   ```

3. Add the Ollama JSON schema to `tool_definitions()` in `src/tools/mod.rs`.

4. Register it in `src/skills/mod.rs` `SkillRegistry::new()`:
   ```rust
   registry.register("your_tool", |params| {
       NodeResult::Text(crate::tools::execute("your_tool", params))
   });
   ```

5. Optionally add a slash command handler in `src/io/web_server/mod.rs` `handle_slash_command()`.

6. Update this document.