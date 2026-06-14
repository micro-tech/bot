# LLM Router — Architecture Overview (Task 148.1)

## Purpose

The LLM Router is a deterministic, low-latency decision layer that sits between the CPU prompt preprocessor and the actual LLM inference backends. Its goal is to choose the best available backend for every user prompt based on complexity, time of day, system load, backend health, user preferences, and fallback rules.

## Core Design Principles

- **Deterministic**: Same input → same output (critical for testing and debugging)
- **Fast**: < 2 ms P99 latency on the hot path
- **Side-effect free** in the core routing function
- **Extensible** via configuration and injection
- **Observable** through structured logging and metrics

## High-Level Data Flow

```
User Prompt
     │
     ▼
CPU Prompt Preprocessor
     │
     ▼
RoutingContext (built here)
     │
     ▼
decide_backend_with_full_context()
     │
     ├── Complexity Scoring
     ├── Schedule Evaluation
     ├── Telemetry Snapshot
     ├── Health Status
     ├── User Overrides
     └── Fallback Engine
     │
     ▼
BackendSelection (chosen backend + audit trail)
     │
     ▼
LLM Inference (Ollama / Gemini / Grok / etc.)
```

## Key Data Structures

### RoutingContext
The single source of truth passed into the router. Contains:
- Prompt text + token estimate
- Complexity score
- Timestamp
- User override (if any)
- Telemetry snapshot (optional)
- Health status map (optional)

### BackendSelection
The output of the router. Contains:
- Chosen `LLMBackend`
- Ordered list of fallbacks attempted
- Reason codes for each decision
- Health and telemetry snapshots used
- Precise timestamps

### RouterConfig
All tunable parameters loaded from TOML/JSON with hot-reload support.

## Major Subsystems

| Subsystem          | Responsibility                              | Key Files                  |
|--------------------|---------------------------------------------|----------------------------|
| Complexity         | Score prompt difficulty (0.0–1.0)           | `complexity.rs`            |
| Schedule           | Time-of-day / profile routing               | `schedule.rs`              |
| Telemetry          | GPU/CPU load, LAN RTT collection            | `telemetry.rs`             |
| Health             | Backend reachability & degradation tracking | `health.rs`                |
| Config             | TOML/JSON loading + validation + hot-reload | `config.rs`                |
| Overrides          | One-shot, session, and persistent preferences | `override_mod.rs`        |
| Fallback           | Resilient backend selection chain           | `fallback.rs`              |
| Strategy           | Core pure `route()` decision function       | `strategy.rs`              |
| Integration        | Public entry point + context assembly       | `integration.rs`           |
| Debug / Explain    | Decision trees, narrative, snapshots        | `debug.rs`, `debug_cli.rs` |
| Observability      | Structured logging + metrics                | `observability.rs`         |

## Public API Surface

The only function external code should call is:

```rust
pub fn decide_backend_with_full_context(...) -> BackendSelection
```

All other modules are internal and may change.

## Extension & Observation Points

See `docs/LLM_ROUTER_EXTENSION_HOOKS.md` for safe ways to extend or observe the router.

## Performance Targets

- P99 routing latency: < 2 ms
- Zero allocations in the hot path (where possible)
- Deterministic behavior under all conditions (including chaos tests)

This architecture ensures the router can evolve independently while remaining safe and observable.
