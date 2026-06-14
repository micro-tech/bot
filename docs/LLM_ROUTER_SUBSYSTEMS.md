# LLM Router — Subsystem Reference (Task 148.2)

This document provides a detailed reference for every major subsystem in the LLM Router.

## 1. Complexity Scoring (`complexity.rs`)
- **Purpose**: Computes a normalized 0.0–1.0 difficulty score for each prompt.
- **Key Types**: `ComplexityFeatures`, `Heuristic` trait.
- **Heuristics**: TokenCount, CodeDensity, ReasoningKeywords, MathSymbols, MultiStep, StructuralDepth.
- **Config Section**: `[complexity]`
- **Performance**: Designed for <1 ms P99 with zero-allocation fast path.

## 2. Schedule Evaluation (`schedule.rs`)
- **Purpose**: Routes based on time-of-day and named profiles.
- **Key Types**: `ScheduleWindow`, `RoutingProfile`, `ScheduleConfig`.
- **Behavior**: Deterministic evaluator that returns the active profile for any timestamp.
- **Config Section**: `[[schedule.windows]]` + `[profiles.*]`

## 3. Telemetry Collection (`telemetry.rs`)
- **Purpose**: Gathers real-time system metrics (GPU, VRAM, CPU, LAN RTT).
- **Key Types**: `TelemetrySnapshot`, `TelemetryCollector`.
- **Behavior**: Non-blocking background sampling with lock-free publication.
- **Used By**: Routing decisions under high load.

## 4. Health Checking (`health.rs`)
- **Purpose**: Tracks reachability and degradation of every backend.
- **Key Types**: `HealthStatus`, `HealthStore`.
- **Probes**: Local Ollama, LAN Ollama, Gemini, Grok.
- **Config Section**: `[health_thresholds]`

## 5. Configuration System (`config.rs`)
- **Purpose**: TOML/JSON loading, validation, defaults, env overrides, hot-reload.
- **Key Types**: `RouterConfig`, `ConfigManager`.
- **Features**: Atomic swap, rollback on invalid config, `LLM_ROUTER_*` env vars.
- **Validation**: `validate_config()` with detailed error reporting.

## 6. User Overrides (`override.rs`)
- **Purpose**: Allows users to force a specific backend.
- **Tiers**: OneShot (highest), Session, Persistent.
- **Key Types**: `OverrideStore`, `OverrideCommand`, `UserOverride`.
- **Commands**: `/use`, `/prefer`, `/clear-override`.

## 7. Fallback Engine (`fallback.rs`)
- **Purpose**: Resilient backend selection when the primary choice is unhealthy.
- **Key Types**: `BackendSelection`.
- **Behavior**: Respects health, load, and override precedence.
- **Config**: `fallback_chain` array.

## 8. Core Strategy (`strategy.rs`)
- **Purpose**: Pure `route()` decision function.
- **Contract**: Deterministic, side-effect free.
- **Input**: `RoutingContext` + `RouterConfig`.
- **Output**: `LLMBackend`.

## 9. Integration Layer (`integration.rs`)
- **Purpose**: Public entry point for the CPU pipeline.
- **Main Function**: `decide_backend_with_full_context(...)`.
- **Wiring**: Combines all subsystems and calls the fallback engine.

## 10. Debug & Explainability (`debug.rs`, `debug_cli.rs`, `debug_narrative.rs`)
- **Purpose**: Human-readable and machine-readable decision traces.
- **Key Types**: `DecisionTree`, `ExplainStep`, `VizJSON`.
- **Tools**: `router_debug` command, snapshot/replay, redaction, TUI.

## 11. Observability (`observability.rs`)
- **Purpose**: Structured logging and metrics aggregation.
- **Exports**: `log_routing_decision()`, `RouterMetrics`.
- **Targets**: Local JSON, SQLite, Prometheus (via exporters).

## 12. Security & Validation (`security.rs`)
- **Purpose**: Capability validation and input sanitization.
- **Functions**: `validate_capabilities()`, `sanitize_backend_name()`.
- **Capabilities**: ToolCalling, Vision, LongContext, FunctionCalling.

## 13. Performance (`perf.rs`)
- **Purpose**: Fast-path scoring and latency measurement helpers.
- **Functions**: `is_simple_prompt()`, `fast_score()`, `measure_route_latency()`.

All subsystems are designed to be independently testable and safely extensible.
