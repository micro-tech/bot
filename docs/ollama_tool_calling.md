# Ollama Tool Calling (Task #1)

## Overview
The bot supports **agentic tool calling** via Ollama's `/api/chat` endpoint with the `tools` parameter. This allows the LLM to decide when to invoke local tools (skills) instead of just returning text.

## Flow
1. User sends a chat message via the bus.
2. `handle_ollama_message` is called.
3. `call_ollama_tools` enters a loop:
   - Sends the conversation + tool definitions to Ollama.
   - If the model returns `tool_calls`, each tool is executed locally via `crate::tools::execute`.
   - Tool results are sent back to the model as `role: "tool"` messages.
   - Loop continues until the model returns plain `content` (final answer).
4. Final response is published to the `cpu` topic and also forwarded to `web_interface`.

## Safety Limits
- Maximum 10 tool-calling rounds per request (prevents infinite loops).
- Health check + model validation performed before every request.
- Automatic retries (3x) with exponential back-off on transient failures.

## Error Handling
- Unreachable Ollama → immediate error published to web UI.
- Missing model → clear error listing available models.
- Tool execution errors → logged + returned to the model for self-correction.
- Empty responses → treated as error.

## Adding New Tools
1. Define the tool in `src/tools/mod.rs` (name, parameters, implementation).
2. The tool will automatically appear in the list sent to Ollama via `tool_definitions()`.

## Files
- `src/io/ollama/mod.rs` — main handler + tool loop
- `src/tools/mod.rs` — tool registry and execution

## Status
- Core functionality: Complete
- Error checking: Complete (as of 2026-06-08)
- Documentation: Complete (this file)
