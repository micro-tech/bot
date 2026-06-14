# LLM Router — Extension Hooks & Integration Points (Task 146.8)

This document describes how external code, tools, or future modules can observe, influence, or extend the LLM Router.

## 1. Public API Entry Point

The main integration point is:

```rust
pub fn decide_backend_with_full_context(
    prompt: &str,
    overrides: &OverrideStore,
    telemetry: Option<&TelemetrySnapshot>,
    health: Option<&HashMap<LLMBackend, HealthStatus>>,
    config: &RouterConfig,
) -> BackendSelection
```

All external consumers should call this function (or a thin wrapper around it).

## 2. Observing Routing Decisions

You can observe every decision by hooking into structured logging:

```rust
// In your logging setup
log_routing_decision(&ctx, &selection);
```

Or subscribe to `RouterMetrics`:

```rust
let metrics = RouterMetrics::new();
// After each decision:
metrics.record(backend_name, latency_ms);
```

## 3. Providing Custom Data Sources

### Custom Telemetry
Implement a background task that periodically updates a shared `TelemetrySnapshot` and pass it into `RoutingContext`.

### Custom Health Checks
Create your own `HealthStatus` map and inject it. The router will respect `reachable: false` and `degraded: true`.

### Custom Overrides
Use the existing `OverrideStore` and `OverrideCommand` system. You can build your own command parser on top of it.

## 4. Extending Behavior

### Custom Routing Strategy
Implement the `RoutingStrategy` trait:

```rust
pub trait RoutingStrategy {
    fn select(&self, ctx: &RoutingContext, config: &RouterConfig) -> LLMBackend;
}
```

Then pass your strategy into the decision layer (future extension point).

### Adding New Backends
1. Add the variant to `LLMBackend` enum
2. Update `sanitize_backend_name`
3. Add health probe logic in the health module
4. Document the new backend in `router.toml.example`

### Adding New Complexity Heuristics
Implement the `Heuristic` trait and register it in the complexity scoring pipeline.

## 5. Configuration Extension

- Add new sections to `RouterConfig`
- Update `validate_config`
- Document new fields in `router.toml.example`
- Support new environment variables via `apply_env_overrides`

## 6. Debug & Replay

Use the planned `router_debug` command (Task 147) to:
- Get explainable output for any prompt
- Capture and replay `RoutingContext` snapshots for regression testing

## Summary of Safe Extension Points

| Extension Type         | Safe? | How |
|------------------------|-------|-----|
| Custom telemetry source| Yes   | Inject `TelemetrySnapshot` |
| Custom health source   | Yes   | Inject `HealthStatus` map |
| User overrides         | Yes   | Use `OverrideStore` |
| Logging / metrics      | Yes   | Hook `log_routing_decision` |
| New backends           | Yes   | Extend enum + health probes |
| New heuristics         | Yes   | Implement `Heuristic` trait |
| Custom strategy        | Future| Via `RoutingStrategy` trait |
| Config fields          | Yes   | Extend `RouterConfig` + validation |

**Rule of thumb**: Never mutate router state from outside. Always inject data or observe via logs/metrics.
