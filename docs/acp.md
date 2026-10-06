# ACP Server (drive Helix from Zed)

Helix speaks the [Agent Client Protocol](https://agentclientprotocol.com)
(ACP) over stdio, so Zed and other ACP clients can drive it natively from
the editor's agent panel.

## Quick start

1. Enable it in `config.toml` (opt-in — the server never starts unasked):

   ```toml
   [acp]
   enabled = true
   max_steps = 25   # runtime-loop steps per session/prompt turn
   ```

2. Point Zed at the binary. In Zed's `settings.json`:

   ```json
   {
     "agent_servers": {
       "Helix": {
         "command": "/home/cobble/.local/bin/helix",
         "args": ["acp"]
       }
     }
   }
   ```

   (Use the path to your helix binary; the R630 service layout puts config
   at `/home/cobble/helix/config.toml`.)

3. Open Zed's Agent Panel, pick **Helix**, and chat. Prompts run through
   Helix's runtime loop (planner + tool supervisor); tool calls stream back
   as `session/update` notifications.

## How it works

- `src/acp/` — the ACP module. Wire types come from the official
  `agent-client-protocol` crate (schema v1); `HelixAcpAgent`
  (`src/acp/agent.rs`) adapts ACP sessions onto Helix's `RuntimeLoop`.
- `helix acp` — the entry point (`src/main.rs`). Reads the same
  `config.toml` resolution as the main server, gates on `[acp] enabled`,
  then serves JSON-RPC on stdin/stdout. Logging goes to stderr so the
  protocol stream stays clean.
- Tool execution is Helix's own: the runtime loop runs tools through
  `ToolSupervisorV2`. ACP only transports the conversation.

## Notes and limits

- Sessions are in-memory only in v1 (`session/load` is not advertised).
- Tool-call notifications are replayed from the runtime trace after each
  turn completes (the loop has no live step hooks yet) — they arrive in
  order, just batched.
- The planner behind ACP is Helix's standard planner stack. A future LLM
  planner upgrade flows into ACP automatically.
- `[acp]` defaults to **disabled**. Enabling it exposes a remote-control
  surface: only enable it on machines you trust, same posture as the web
  UI lockdown.
