# MCP Client (Model Context Protocol)

Helix can spawn **external MCP servers** as subprocesses over stdio and use
their tools — for example the standalone `google-mcp` server that exposes
Gmail + Google Calendar.

> Not to be confused with `src/mcp/` ("Master Control Program"), which is
> Helix's internal bus message router. The two are unrelated; that's why the
> client lives in `src/mcp_client/`.

## Quick start: google-mcp

1. Build the server (sibling project):
   `cd ~/workspace/google-mcp && cargo build`
2. Follow `~/workspace/google-mcp/README.md` to create a Google Cloud OAuth
   client and run `google-mcp auth` once. This caches a token at
   `~/.config/google-mcp/token.json`.
3. In Helix's `config.toml`, uncomment the `[mcp]` section at the bottom of
   the file and point it at the binary:
   ```toml
   [mcp]
   enabled = true

   [[mcp.servers]]
   name = "google"
   command = "/home/YOU/workspace/google-mcp/target/debug/google-mcp"

   [mcp.servers.env]
   GOOGLE_CLIENT_ID = "<your-client-id>"
   GOOGLE_CLIENT_SECRET = "<your-client-secret>"
   ```
   (Keeping secrets in `config.toml` is convenient but the file is plain
   text — on a shared machine prefer exporting the vars in the environment
   Helix runs in instead; `[mcp.servers.env]` entries are optional and the
   server inherits the normal environment.)
4. Restart Helix. The agent now sees tools like `mcp__google__gmail_search`.

## Config reference

```toml
[mcp]
enabled = false            # master switch; default false (silent no-op)

[[mcp.servers]]            # repeat per server
name = "google"            # namespace used in tool names; must not contain "__"
command = "/path/to/bin"   # executable to spawn (PATH lookup works too)
args = ["--flag"]          # optional extra argv entries

[mcp.servers.env]          # optional extra env vars for the server process
FOO = "bar"
```

## Tool naming

Every upstream tool is exposed as:

```
mcp__<server>__<tool>
```

e.g. `mcp__google__gmail_search`, `mcp__google__calendar_create_event`.
Tool descriptions are prefixed with `[MCP:<server>]` so the model knows the
origin. The upstream JSON schema is passed through unchanged as the
`parameters` schema.

## How it works

- `src/config/mcp.rs`: parses the `[mcp]` section.
- `src/mcp_client/manager.rs`: spawns each server once (lazily, on first
  use), runs the MCP initialize handshake (30 s timeout), caches the
  `tools/list` result, and forwards `tools/call` (120 s timeout).
- `src/tools/mod.rs`: `tool_definitions()` appends the namespaced schemas;
  `execute()` routes any `mcp__*` name to the manager.
- Sync/async bridging: `execute()` is synchronous but is also called from
  async contexts (axum handlers, the Ollama loop), where a fresh
  `Runtime::new().block_on()` would panic. All MCP futures run on one
  dedicated multi-threaded runtime (`helix-mcp` threads); sync call sites
  enter it via `block_on`, or `block_in_place` when an ambient runtime is
  already present. See `src/mcp_client/mod.rs::bridge`.

When `[mcp]` is disabled (the default), nothing is spawned, no tools are
advertised, and there is no startup delay or behavior change.

## Troubleshooting

- **No `mcp__` tools listed**: check `enabled = true` and restart — tool
  schemas are fetched once per process.
- **`mcp: failed to spawn server 'X'`** in logs: `command` is wrong or not
  executable. Use an absolute path to rule out PATH issues.
- **Handshake timeout**: the server didn't speak MCP on stdio within 30 s.
  Run the command by hand to see its stderr.
- **Tool call returns an error string**: the server ran but the call failed
  (e.g. google-mcp without OAuth credentials — run `google-mcp auth`).
- **One bad server doesn't break the rest**: a server that fails to spawn or
  handshake is skipped with a warning; its tools are simply absent.
