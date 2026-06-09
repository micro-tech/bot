# Memory System

Agent OS combines multiple memory types for rich agent behavior.

## Memory Types
- **Working Memory**: Short-term context (recent messages)
- **Episodic Memory**: Event records with timestamps
- **Vector Memory**: Semantic search over facts using embeddings

## Key Components
- `src/memory/manager.rs`
- `src/memory/episodic.rs`
- `src/memory/vector.rs`

## Usage
Skills and the reasoning engine can read/write beliefs and facts through the `MemoryManager`.

See also: [Vector Memory](vector_memory.md)
