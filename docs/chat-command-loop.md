# Chat-command loop + task handoff

Helix watches a chat-hub room and acts on messages posted there by
authorized senders: read-only diagnostics (Phase 1) and LLM task
handoff (Phase 3). This doc covers both. The ASUS relay (Phase 2)
is a separate target for the same commands.

Status: **live on the R630** (deployed 2026-11-05). Phase 1 loop
responds to `!uptime` / `!ls`; task 308's `[exit 0]` line was
verified live 2026-11-05 19:20 EST. The formal drill checklists
(§9) are partly run — see the live-status notes there.

Related: the build spec for the handoff is
`chat-command-task-handoff-spec.md` (in the workspace, not the
repo); the implementation is `src/chat_command/`
(`mod.rs`, `config.rs`, `executor.rs`, `reply.rs`, `envelope.rs`,
`task_handler.rs`, `asus.rs`, `state.rs`).

## 1. What it is

A watcher-style routine, `chat-command-watch`, fires every minute
and polls one hub room. It executes read-only diagnostic commands
posted there, and runs LLM tasks as async agent jobs, posting
output and reports back to the same room. Plain commands run fully
in-process (no LLM on that path). LLM tasks run detached and never
block the tick.

- **Room:** `command` (configurable; hub rooms auto-create on first
  post — no manual step).
- **Routine:** `chat-command-watch`, every 1m. It ships **DISABLED**;
  John enables it after reviewing `[chat_command]` (see §2).
- **State:** cursor in `<runtime_dir>/chat_command/state.json`
  (room + last seen hub message id). On first run, or on corrupt
  state, the cursor seeds to the room's latest id — old messages
  never replay.
- **Config:** `[chat_command]` in `config.toml` is re-read on EVERY
  tick, so edits apply without a restart.
- **Prerequisite:** chat-mcp configured (`[[mcp.servers]] chat`)
  with the hub token. If no `mcp__chat__*` tools are advertised,
  the tick quietly skips. Do NOT put tokens in `config.toml` —
  chat-mcp holds the hub token.

## 2. Enable it

1. In `config.toml`, set the room and the allowed senders:

```toml
[chat_command]
room = "command"
allowed_senders = ["cobble-jr", "john"]
```

2. Enable the `chat-command-watch` routine (web UI toggle or
   `routines.json`).

Fail-closed defaults (omitting the section selects these):

| Key | Default | Meaning |
|---|---|---|
| `room` | `"command"` | Hub room watched |
| `allowed_senders` | `[]` (empty) | **Nothing executes** until configured |
| `command_timeout_secs` | `30` | Per-command wall-clock timeout (s) |
| `max_output_bytes` | `131072` | Captured output cap per stream (128 KiB) |
| `chunk_bytes` | `7500` | Reply chunk size (hub cap is 8 KB; `[n/m]` prefix included) |
| `max_commands_per_tick` | `20` | Commands per 1-minute tick; extras wait |
| `asus_enabled` | `false` | Phase 2 master switch (default OFF) |
| `asus_host` / `asus_user` / `asus_key_path` | none | REQUIRED for Phase 2; no defaults — a missing value keeps the relay off |

The example config (`example.config.toml`) has the section
commented out with these defaults annotated.

## 3. How commands work (Phase 1)

### Trigger and sender gate

1. **Sender gate FIRST, before any parsing.** The message's sender
   (stamped by the hub from the token) must be in
   `allowed_senders`. Expected values: `"cobble-jr"`, `"john"`.
   Everyone else — including `"helix"` — is ignored silently and
   audit-logged. This is the loop prevention: Helix's own replies
   can never re-trigger the routine, no matter what they start
   with.
2. **Trigger:** the trimmed body must start with `!`. Plain chat
   (no `!`) is ignored, no reply. A bare `!` gets an
   `empty command` reply.
3. **Target:** an exact-lowercase `asus:` prefix routes to the ASUS
   (§4); anything else runs locally.

### Executor (no shell, ever)

The command line is split on whitespace; every token is screened
for metacharacters (`| > < ; & $ \` " ' * ? ~ ( ) { }` and control
characters — a token with one is rejected whole, no escaping or
quoting in v1). `argv[0]` must be allowlisted, and the process is
spawned with `std::process::Command` directly. `run_shell` is
deliberately not used — it runs via a shell and allows writes.
stdin is `/dev/null` (interactive prompts fail fast).

**Local allowlist** (hardcoded in code, NOT config — a config edit
must never silently widen remote execution; allowlist changes go
through code review):

```
cat tail head grep ls ps df free uptime whoami hostname systemctl
```

`systemctl` only with a read-only subcommand: `status`,
`is-active`, `show`, `list-units`. `systemctl restart` / `stop` /
`enable` (and bare `systemctl`) are rejected.

**Timeout:** `command_timeout_secs` (default 30 s). On timeout the
whole process group is SIGKILLed (backgrounded grandchildren die
too); partial output captured so far is returned.

**Output cap:** `max_output_bytes` per stream (stdout and stderr
separately). Bytes past the cap are read and discarded (so a chatty
child never blocks on a full pipe) and the reply carries a
`[truncated]` note.

### Reply format

One reply per executed command, sent to the same room:

```
$ uptime
 00:20:54 up 30 min,  1 user,  load average: 0.21, 0.13, 0.10
[exit 0]
```

- Echo line first (`$ <command>`; `asus$ <command>` for the ASUS).
- Then stdout, then a `[stderr]` section if stderr is non-empty.
- `[truncated]` when the output cap cut anything.
- **Exit status LAST** (task 308), so it rides the last chunk:
  exit code `N` renders `[exit N]` (including `[exit 0]`);
  killed by a signal renders `[exit signal]`. A timeout keeps its
  `[TIMEOUT after Ns — process killed]` note up top and gets no
  second outcome line. Empty output renders `(no output)`.

### Chunking

Replies are split into `chunk_bytes`-byte chunks on UTF-8 char
boundaries, prefixed `[n/m] ` (the prefix counts against the
budget), sent in order to the same room, paced ~150 ms (hub burst
rate limit), retry-once per chunk. A failed reply is abandoned
after its retry and logged — it never re-executes the command.
Chunks reassemble to the original text (strip the `[n/m] ` prefix
from each and concatenate).

### Tick limits

- **Tick budget ~45 s:** after that, remaining messages wait for
  the next tick. A tick that overlaps a still-running tick skips
  (guard).
- **`max_commands_per_tick` (default 20):** a message that would
  consume a command slot waits when the cap is reached — its cursor
  does NOT advance, so the next tick retries it. Ignored messages
  never consume slots and always advance the cursor.

## 4. ASUS relay (Phase 2)

`!asus: <command>` routes over Tailscale SSH to Windows OpenSSH
Server on the ASUS. The same sender gate runs before routing, and
the same metachar rejection runs before ssh — the remote shell only
ever sees simple words. No shell runs on the R630 side either; ssh
itself is spawned directly.

- **Enable:** `asus_enabled = true` plus `asus_host`,
  `asus_user`, `asus_key_path` — all three required, no defaults.
  Anything less fails closed: the reply is
  `ASUS relay not enabled/configured` and **zero ssh attempts**
  are made.
- **SSH:** `BatchMode=yes` (fail fast, never hang on a password
  prompt) + `ConnectTimeout=15s`, key, `user@host`.
- **ASUS allowlist** (starter set, PowerShell, case-insensitive —
  PowerShell cmdlets are): `hostname`, `whoami`, `Get-Process`,
  `Get-Service`, `Get-ComputerInfo`, `systeminfo`. Single cmdlets
  with simple args only — no pipelines, redirection, or
  substitution (already rejected by the metachar rule).
- **Caveat:** the allowlist is a starter set. Task 305 finalizes it
  against the CONFIRMED ASUS shell + PowerShell version from tasks
  302/303 — if the drill shows the default shell is cmd.exe, the
  list becomes the cmd set and this doc is updated. ASUS setup
  verification (task 303) is still open.

## 5. Task handoff (Phase 3)

Cobble jr (or John) hands a TASK to one of Helix's LLM backends
through the same `command` room. The backend runs it as an agent
job and reports back in the same room. Diagnostics keep working
exactly as before.

### Envelope schema

A `!` message body, after stripping `!` and trimming, is JSON of
this shape. `how` is the dispatch key.

| Field | Type | Meaning |
|---|---|---|
| `how` | `"command"` \| `"llm"` (required) | Which path: run a shell command, or send a task string to an LLM |
| `which` | string | Target. For `how: command`: `"local"` (default) or `"asus"` (Phase 2). For `how: llm`: `"server"`, `"desktop"`, or `"gemini"` (required — no silent default; agent time is never spent on the wrong backend by accident) |
| `time` | integer seconds | Wall-clock budget. Defaults: command `command_timeout_secs` (30), llm 300. Must be 1–3600. Exceeding it stops the run and posts a timeout report, never a silent hang |
| `scope` | string (required) | Permission. `"read"` = report only, no writes of any kind. `"write:<domain>"` = may write within that domain only. Enforcement is by tool allowlist, not by asking the LLM nicely (§6) |
| `command` | string (required iff `how: command`) | The shell command. Same allowlist, no-shell executor, and metachar rules as Phase 1 |
| `task` | string (required iff `how: llm`) | The task description for the LLM, in plain language. Must fit the 8 KB envelope |

Example — diagnostic as JSON:

```json
{"how": "command", "which": "local", "time": 30, "scope": "read", "command": "uptime"}
```

Example — the spam task as JSON:

```json
{"how": "llm", "which": "server", "time": 300, "scope": "gmail:read", "task": "Check Gmail for spam. Report back the senders and subjects of anything that looks like spam; do not label or archive anything."}
```

Notes:

- `which` for llm tasks names a Helix chat **backend**, not a model
  version. `"server"` is gpt-oss-20b on the ollama box (.149),
  `"desktop"` is gemma4:e4b on the ASUS (.196), `"gemini"` is the
  Gemini backend as Helix has it configured. The exact model
  version is Helix's config, not the envelope's job. No new SSH is
  needed for llm tasks — the backends are already how Helix chats.
- Extra fields are ignored (forward compatibility); the field
  irrelevant to `how` (`task` on a command, `command` on an LLM
  task) is ignored.
- A structured envelope over 8 KB (the hub's cap) is rejected with
  a "message too large" reply, never silently truncated. Plain
  (non-structured) messages are NOT subject to this cap — the
  command path has its own output cap.

### Dispatch rules (parse-or-fallback)

1. Message does not start with `!` → ignore (plain chat).
2. Message starts with `!` → strip `!`, trim, try JSON parse.
3. Parse succeeds AND `how` is `"command"` or `"llm"` → structured
   dispatch (below). Required fields for that path are validated
   first; on any validation failure, a **rejection reply naming
   the problem** is posted (e.g. `rejected: unknown backend 'x'`,
   `rejected: llm task needs a task string`) and **nothing
   executes**. Never run a malformed structured message.
4. Parse fails, or `how` is missing/invalid → **fall back to the
   plain command path** (today's behavior: `asus:` split,
   allowlist, metachar rules). This is what keeps hand-typed
   `!uptime` working alongside machine-generated JSON. A non-JSON
   `!` message is a command, not an error.

The task string on the llm path is DATA for the LLM. It is never
executed as a shell command, even if it contains shell-looking
text.

### Command path (unchanged)

The Phase 1 executor, verbatim: hardcoded allowlist, no shell,
metachar rejection, timeout (`time` overrides
`command_timeout_secs` when present), output cap, chunked replies
`[n/m]`, audit log, plus the task-308 `[exit N]` line. Routing is
by `which`, NOT by an `asus:` prefix inside the command string —
the JSON command is verbatim, so a `which: local` command starting
with `asus:` fails the local allowlist rather than routing. Scope
does not widen commands: a `write:` scope on a command still runs
the read-only allowlist.

### LLM path

- **Handler.** Takes (task string, backend, time budget, scope).
  Spawns an agent run on that backend via Helix's existing agent-run
  machinery (`dispatch_spawn` — the same path the commander pattern
  uses); no new LLM client is invented here. The backend acts as
  commander and may delegate to sub-agents.
- **Async.** Agent runs take minutes, so the handler MUST NOT block
  the 1-minute routine tick. The tick that accepts a task posts an
  ACK (`task #<hub_id> accepted, running on <backend>, budget
  <time>s`) and returns. A later tick (or completion callback)
  posts the RESULT when the run finishes, or a timeout report if
  the budget expires. Every task ends in a report or a timeout
  report — never silence.
- **Concurrency.** One running task per backend. A task for a busy
  backend QUEUES behind it (FIFO per backend). The sender gets a
  reply stating the backend is busy, that the task is queued, and
  its position (`task #<id> queued: <backend> is busy running task
  #<running>; position <n> in queue`). A task for an idle backend
  runs immediately. v1 sets no queue cap; a cap plus a queue-full
  reply is a follow-up if backlogs grow in practice. When a queued
  task starts, the room gets a `task #<id> started on <backend>`
  notice.
- **Report.** Posted to the SAME room (`command`), chunked `[n/m]`
  at 7500 bytes like diagnostics, citing the task's hub message id
  (so the sender can correlate task → report). Header is
  `task #<id> result|failed|timed out (backend <name>)`. A
  read-task report states explicitly that no writes were made (or
  lists the writes, for a write task). Reports never start with `!`.
- **Audit.** Every acceptance, start, completion, timeout,
  rejection, and busy reply is audit-logged (§8).

## 6. Scope enforcement

Scope is enforced by **tool allowlist**, not by the prompt. The
agent run's toolset is set from `scope` before spawning:

- The scope maps to a persona role file
  (`chat-task-read`, `chat-task-write-<domain>`) under
  `<runtime_dir>/agents/`, whose frontmatter `tools:` list is the
  allowlist. The file is (re)written on task submit (skipped when
  the content is already identical).
- **The runner verifies the filtered toolset BEFORE spawning**
  (fail closed): non-empty, a subset of the scope's allowlist, and
  — for a write scope — containing at least one write tool for that
  domain. Any failure → the task is refused with a report, never
  spawned. (Persona files alone are NOT fail-closed: a missing file
  or an all-unknown allowlist yields the FULL toolset, which the
  verifier refuses.)
- A read task that attempts a write fails closed (the tool is
  unavailable) and the report says so.

**Scope vocabulary** (proposal — open question Q3, §10):
`"read"`, `"write:<domain>"`, with known domains `notes` and
`gmail`. The parser also accepts the domain-first forms
`<domain>:read` / `<domain>:write` for known domains (the spec's
§3 example uses `"gmail:read"`); reports normalize to the table
form (`read` / `write:<domain>`).

**Read-task tools:** `read_log`, `read_note`, `list_notes`,
`check_inbox`, `read_email`, `system_status`, `list_tools`,
`get_beliefs`, `repo_glob`, `repo_read`, `repo_grep`,
`scan_directory`, plus Gmail read tools
`mcp__google__gmail_search`, `mcp__google__gmail_read`.

**Write tools per domain:** `notes` → `write_note`; `gmail` →
`mcp__google__gmail_modify`, `mcp__google__gmail_label`,
`mcp__google__gmail_archive`, `mcp__google__gmail_send`.

**Always excluded** (even from write tasks): `spawn_subagent`,
`run_shell` (a shell can write), `send_email`, `set_belief`,
`bayes_reset`, `generate_mermaid_project_map` (writes files).

**Gmail write names are BEST-EFFORT** — they are not confirmed
against the live google-mcp tool list anywhere in this repo. If the
MCP server advertises different names, the filtered set contains
no Gmail write tool and the verifier refuses the spawn (safe, but
the task does not run). Confirming the real names against the live
tool list is a deploy-time follow-up, not a silent grant.

## 7. Known v1 limits

- **Queue + running state are IN-MEMORY.** A Helix restart drops
  queued/running tasks without a report; the sender must re-send
  if no report arrives. Persisting the queue is a follow-up.
- **Write-task reports state the scope limit but do NOT enumerate
  the writes made** — the spawn report carries no tool-call trace.
  Enumerating writes needs tracing from the spawn machinery
  (follow-up).
- **Timeout reports carry no partial results** — the spawn timeout
  cancels the run and returns no partial text. The report says
  timed out, honestly, without invented partials.
- **The `chat-task-read` role has no hostname tool.** In drill (f3)
  the agent could not answer a hostname question — its tools are
  system-status/log tools only. A task that needs the R630 hostname
  (or similar) may go unanswered; this follow-up is not filed.
- **Gmail write tool names are best-effort** (see §6).

## 8. Audit log and troubleshooting

**Audit log:** every execution, rejection, acceptance, start,
completion, timeout, and busy/queued reply is written with a
`chat-command:` prefix to the error log —
`$HOME/.helix/logs/error_log.md` on the R630
(`/home/cobble/.helix/logs/error_log.md`). Tail it to see what the
loop did; the hub room shows the replies. (The web UI's error-log
clear endpoint clears this same file.)

**Troubleshooting:**

| Symptom | Cause / check |
|---|---|
| Nothing executes, no replies | Routine disabled, OR `allowed_senders` empty, OR no `mcp__chat__*` tools advertised (chat-mcp not configured). The tick quietly skips in all three cases — check the routine, the config, and the `[[mcp.servers]] chat` block. |
| `rejected: ...` reply | The message failed validation (unknown command/backend, bad scope, missing task/command, oversize envelope, metachar in a token). The reply names the problem; nothing ran. |
| `ASUS relay not enabled/configured` | Phase 2 not enabled or incompletely configured (all of `asus_enabled`, `asus_host`, `asus_user`, `asus_key_path` are required). Zero ssh attempts were made. |
| Reply is `[1/2]`, `[2/2]`, … | Long output chunked (§3). Strip the `[n/m] ` prefix from each chunk and concatenate to reassemble. |
| `[TIMEOUT after Ns — process killed]` | The command (or task) exceeded its budget; the process was killed. Raise `time` (max 3600 s) or split the work. |
| Task accepted but no report arrives | The run is still going, OR Helix restarted (in-memory queue dropped — re-send), OR the backend errored (check the error log for the `chat-command:` lines). |
| Task refused: `no write tool for domain 'gmail' available` | google-mcp advertises no Gmail write tool under the expected names (§6) — confirm the real tool names at deploy time. |
| `!tail` / `!cat` on a big file | Don't cat whole files — the output cap is 128 KiB per stream. Tail the file (or the error log) instead. |

## 9. Live drill status

### Phase 1 drill (task 302.2 — John's steps)

1. Set config + enable routine. **Proven live** (R630, 2026-11-05).
2. Cobble jr posts `!hostname` → Helix replies with the R630
   hostname. **Open** (not run end-to-end as specified).
3. John posts `!whoami` → Helix replies. **Open.**
4. Helix token posts `!hostname` → IGNORED (no execution, no
   reply). **Open** (the sender gate is unit-tested; live run
   pending).
5. Cobble jr posts `!systemctl restart helix` → REJECTED reply,
   nothing restarted. **Open.**
6. Long-output command (e.g. `!ps aux`) → chunked replies in
   order, reassembled equals full output. **Open** (chunking is
   unit-tested; live run pending).
7. Cobble jr posts `hello` (no `!`) → ignored, no reply. **Open.**

The drill passes only if ALL items behave as specified, with audit
log lines present for each execution/rejection. Incidental live
evidence (not the drill): the loop answered Cobble jr's `!uptime`
and `!ls` on 2026-11-05, and John's own `!uptime` / `!ls` the same
day; `!uptime` replies carry `[exit 0]` (task 308, verified live
2026-11-05 19:20 EST).

### Task-handoff drills (task 311.2 — spec §9, John's steps)

Run 2026-11-05 00:56 EST (room `command`, verified from hub
reads), plus a re-run at 01:06 EST:

- (a) JSON command (`uptime`) → correct reply. **PASS.**
- (b) Plain `!uptime` still works (fallback). **PASS.**
- (c) JSON llm task (safe read-only, e.g. report disk usage) →
  agent report back in the room. **FAIL** — ACK, then timeout at
  120 s, no report. Re-run at a 300 s budget (01:06 EST) **failed
  again**: `error sending request for url
  (http://192.168.1.149:11434/api/chat)`.
- (d) Malformed JSON task → rejection, nothing runs. **PASS**
  (`rejected: llm task needs a task string`).
- (e) Task from the helix token → ignored (loop prevention).
  **NOT RUN** — needs a message sent from Helix's own token.
- (f) Second task for a busy backend → queued, sender gets a busy
  reply with position; task for an idle backend runs immediately.
  **PASS** — the queued task reported position 1 behind the
  running task, started when it failed, then timed out at 60 s;
  the idle-desktop task ACKed immediately and reported back in
  47 s. (But see §7: the desktop agent could not answer the
  hostname question — no hostname tool in the read role.)

The handoff machinery was correct in every case (ACKs, rejections,
queue, timeout/failure reports citing task ids, chunked, "no writes
were made", never starting with `!`). The (c)/(f1) failures are the
**.149 Ollama backend** (timeouts + Ollama request errors), not the
handoff code — the server backend is the weak link, and a
warm-model re-run is still untested. Drills pass only when ALL
items behave as specified, with audit log lines for each.

## 10. Open questions and safety summary

**Open questions** (spec §10; decisions recorded):

1. DECIDED 2026-11-04: `which` stays required for llm tasks. No
   default — the sender names the backend explicitly.
2. DECIDED 2026-11-04: busy reply AND queue (not busy-reject).
   A busy reply tells the sender the backend is running; tasks
   queue FIFO per backend and the reply states the queue position.
3. OPEN: is the `read` / `write:<domain>` scope vocabulary enough,
   or finer domains? The proposal stands until John revises it.
   The parser accepts both orders (`write:gmail`, `gmail:read`)
   for known domains `notes` / `gmail`.
4. DECIDED 2026-11-04: reports stay in the same room
   (`command`). No separate results room.

**Safety summary:** sender gate first; task strings are data,
never shell; scope enforced by tool allowlist (verified before
spawn); time budgets always, with timeout reports; one task per
backend, extras queue behind it, busy reply shows running + queue
position; reports never retrigger (sender gate + no `!` prefix);
8 KB envelope cap (oversize → rejection, not truncation); command
allowlist is code, not config; no shell anywhere on the command
path; fail-closed defaults everywhere (empty `allowed_senders`,
disabled routine, disabled ASUS relay).
