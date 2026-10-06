# A2A v1.0 in Helix

Helix speaks [A2A](https://a2a-protocol.org/) (Linux Foundation `lf.a2a.v1`,
JSON-RPC binding) on its web server. Tasks execute on the same agent loop as
ACP (`HelixAcpAgent` wrapping `RuntimeLoop`) — one agent loop, two protocol
surfaces (ACP on stdio via `helix acp`, A2A on HTTP).

## Endpoints

| Endpoint | Auth | Notes |
|---|---|---|
| `GET /.well-known/agent-card.json` | none (public discovery) | Agent Card: name, version, protocol `1.0`, skills, `authentication: ["bearer"]` |
| `POST /a2a` | web token required | JSON-RPC 2.0 dispatcher |

Auth reuses the web token: `Authorization: Bearer <token>` or
`X-Web-Token: <token>`, where `<token>` is `[web] auth_token` (or
`HELIX_WEB_TOKEN`). Missing/invalid → `401`. The server binds `127.0.0.1` by
default — same posture as the other mutating web endpoints.

## Methods

Both v1.0 (`message/send`) and v0.3-era (`message:send`) spellings are
accepted; v1.0 is emitted. `contextId` (v1.0) and `sessionId` (v0.3) are both
accepted on input.

- `message/send` — `{message: {role, parts[]}, contextId?}` → creates a task,
  spawns execution, returns the `Task` (usually still `submitted`/`working`).
- `message/stream` — same params, returns `text/event-stream` (SSE) with
  JSON-RPC task snapshots until the task reaches a terminal state.
- `tasks/get` — `{id}` → the `Task`.
- `tasks/cancel` — `{id}` → aborts the worker, marks the task `canceled`.
  Canceling an unknown id → `-32001`; canceling a terminal task → `-32602`.
- `tasks/list` — Helix extension (not in the spec) → all known tasks.

Task states: `submitted` → `working` → `completed` | `failed` | `canceled`
(kebab-case on the wire). The completed task carries the agent's final message
as an artifact plus the full user/agent history.

## Example

```bash
TOKEN="your-web-token"
BASE="http://127.0.0.1:8443"

# 1. Discovery (no auth needed)
curl $BASE/.well-known/agent-card.json

# 2. Send a message
TASK=$(curl -s -X POST $BASE/a2a \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"message/send",
       "params":{"message":{"role":"user",
         "parts":[{"kind":"text","text":"check the inbox"}]}}}' \
  | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['id'])")

# 3. Poll until done
curl -s -X POST $BASE/a2a \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tasks/get\",\"params\":{\"id\":\"$TASK\"}}"

# 4. Or stream it (SSE)
curl -N -X POST $BASE/a2a \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"message/stream",
       "params":{"message":{"role":"user",
         "parts":[{"kind":"text","text":"check the inbox"}]}}}'
```

## Notes

- Task storage is in-memory (per process); tasks don't survive restarts.
- `max_steps` for A2A turns comes from `[acp] max_steps` (default 25).
- No push notifications in v1 (`pushNotifications: false` on the card).
