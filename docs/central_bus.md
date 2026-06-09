# Central Bus System

The Central Bus is the core communication backbone of Agent OS.

## Architecture
- Publish/Subscribe model
- Decouples producers and consumers
- All major subsystems communicate via the bus

## Topics
- `cpu`
- `memory`
- `skills`
- `web`
- `unix_socket`
- `ollama`
- `heartbeat`

## Implementation
- `src/bus.rs`

Messages are JSON and include `to`, `from`, `data`, and `timestamp`.
