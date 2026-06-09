# Terminal CLI

The built-in terminal interface provides direct interaction with Agent OS.

## Starting
The CLI starts automatically when you run:
```bash
cargo run
```

You will see the `AgentOS>` prompt.

## Basic Usage
Type natural language queries or structured commands:
```
AgentOS> What's the system status?
AgentOS> {"cmd":"health"}
```

## Features
- Direct LLM prompting
- Bus message publishing
- Real-time responses
- Chat history saved to `logs/chat_log.md`

## Limitations
For advanced scripting and structured commands, use the [UNIX Socket CLI](unix_socket.md) instead.
