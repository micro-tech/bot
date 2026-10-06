# OKF Protocol v1

**Status:** canonical contract for all OKF clients and servers in John's stack.
**Applies to:** Helix (`src/okf/`, `src/io/web_server/`) and grok-cli
(`src/workflow/okf.rs`, `src/tools/okf_tools.rs`).

## Background

The two OKF clients were built against two different imagined servers and a
standalone OKF server was never built. Helix, however, already contains the
server half: its web server mounts OKF routes (gated on `[helix.okf] enabled`).
This document is the ONE contract — clients and the in-Helix server both
conform to it. Do not invent a second contract.

## Base

All paths live under `/okf/`. The base URL is configurable per client
(Helix: `[helix.okf] server_url`, default `http://localhost:8080`;
grok-cli trace forwarder: `[okf] server` + `port`, knowledge remote:
`[okf] remote_url`).

## Conventions

- **Auth:** `Authorization: Bearer <token>` accepted on every endpoint.
  Servers are fail-open when no token is configured (matches the Helix web
  posture: no token set, no auth demanded). When a token IS configured,
  write endpoints require it.
- **Conditional GETs:** `GET` endpoints that serve versioned content send an
  `ETag` response header and honor `If-None-Match`, returning `304 Not
  Modified` with an empty body on a match.
- **Errors:** JSON bodies shaped `{"error": "<message>"}` with an appropriate
  status code (`400` bad input, `401` bad/missing token when required,
  `404` unknown bundle/knowledge id).
- **Content types:** manifests and JSON payloads are `application/json`;
  knowledge documents are served as their `content_type`
  (`text/plain`, `text/markdown`, …).

## Read endpoints (public)

### `GET /okf/health` → `200`
Liveness + readiness. Response always includes `"status": "ok"`:
```json
{
  "status": "ok",
  "okf_enabled": true,
  "bundle_loaded": true,
  "tool_count": 12
}
```

### `GET /okf/status` → `200`
Registry summary (bundle id/version, counts, last reload info, poll config).

### `GET /okf/registry` → `200`
Full registry: `{bundle_id, version, tool_count, knowledge_count, tools,
knowledge, schemas}`.

### `GET /okf/schema` → `200`
The OKF Bundle Schema v1.0 (`okf_bundle_schema.json`).

### `GET /okf/manifest.json` → `200`
The currently loaded bundle manifest. Must conform to
`okf_bundle_schema.json` (required: `id`, `version`). Sends `ETag`;
honors `If-None-Match` → `304`.

### `GET /okf/bundles/{id}/manifest.json` → `200`
Manifest for a specific bundle id. `404 {"error": ...}` when unknown.
ETag / `If-None-Match` as above.

### `GET /okf/knowledge/{kid}` → `200`
A single knowledge document (body text). `404` when unknown.

## Write endpoints

### `POST /okf/traces` → `202`
Append a workflow trace. Body: grok-cli `WorkflowTrace` JSON.
Response: `{"status": "accepted"}`. Bearer optional (required when a token
is configured). This replaces grok-cli's legacy `/api/traces` path.

### `POST /okf/bundles/{bundle}/concepts` → `201`
Append a knowledge concept to a bundle. Body: `OkfConcept` JSON
(`{id, type, title, description, resource, tags, timestamp, body}`).
Response: `{"status": "created", "id": "<concept-id>"}`.
Bearer optional (required when a token is configured).
This replaces grok-cli's legacy `/bundles/{bundle}/concepts` path.

### `POST /okf/reload` → `200` (protected)
Force a manifest hot-reload from the configured upstream. (Helix-internal
management; requires the web token via existing middleware.)

### `POST /okf/reload/force` → `200` (protected)
Reload ignoring any cached ETag.

### `POST /okf/manifest` → `200` (protected)
Push a manifest JSON directly (testing / local bundles).

### `POST /okf/webhook` → `200` (public, server-to-server)
Change notification: `{"bundle_id": "...", "action": "updated"}`.
Triggers a reload when `auto_reload` is on. Deliberately unauthenticated
(same posture as before — revisit if a shared webhook secret is added).

## Client mapping

| Client | Operation | v1 path |
|---|---|---|
| Helix fetcher | manifest | `GET /okf/manifest.json` (+ ETag) |
| Helix fetcher | per-bundle manifest | `GET /okf/bundles/{id}/manifest.json` (+ ETag) |
| Helix fetcher | knowledge doc | `GET /okf/knowledge/{kid}` |
| Helix fetcher | health | `GET /okf/health` |
| grok-cli forwarder | trace append | `POST /okf/traces` (legacy `/api/traces` translated internally) |
| grok-cli `okf_create` | concept push | `POST /okf/bundles/{bundle}/concepts` (legacy `/bundles/{b}/concepts` translated internally) |
| grok-cli `okf_lookup` | remote fetch | `GET /okf/manifest.json` (+ ETag), then `GET /okf/knowledge/{kid}` per hit; local dirs stay the offline fallback |

## Migration notes

- grok-cli configs that set `[okf] endpoint = "/api/traces"` keep working:
  the client translates the legacy path to `/okf/traces` internally and logs
  a deprecation note. New configs should use `/okf/traces`.
- grok-cli `remote_url` values keep working: `/bundles/{b}/concepts` is
  translated to `/okf/bundles/{b}/concepts` internally.
- Helix `[helix.okf] server_url` is unchanged; the fetcher's existing
  `/okf/*` paths were already v1-conformant.

## Out of scope for v1

Remote tool *execution* (Helix's `try_execute_okf_tool` stays simulated).
A v2 dispatch protocol needs its own threat model first.

## Deploying the in-Helix OKF server (Ubuntu box)

Helix serves the v1 endpoints itself — no separate server binary. On the
machine that should host OKF (John's Ubuntu box on the R630):

1. In that machine's live `config.toml`:
   ```toml
   [helix.okf]
   enabled = true
   server_url = "http://localhost:8080"   # or this box's LAN address:port
   trace_file = "okf_traces.jsonl"
   # auth_token = "..."                   # set this if grok-cli clients are untrusted
   ```
   The OKF endpoints ride on the **web server port**, not 8080, unless the
   web server itself is bound to 8080. Point `server_url` at wherever the
   Helix web UI listens.
2. Restart Helix. Verify:
   ```bash
   curl -s http://localhost:<web-port>/okf/health
   # {"status":"ok","okf_enabled":true,...}
   curl -s http://localhost:<web-port>/okf/manifest.json | head -c 200
   ```
3. grok-cli clients point `[okf] remote_url` at that base URL; the trace
   forwarder's `[okf] server`/`port` likewise.
4. Binding follows `[web] bind` (default `127.0.0.1` — LAN exposure is an
   explicit config choice). Write endpoints additionally accept the OKF
   `auth_token` Bearer <redacted> grok-cli sends, so set `auth_token` if the box is
   shared. No separate systemd unit is needed beyond the existing
   `helix.service`.

