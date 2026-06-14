# LLM Router — Module Dependency Matrix (Task 146.3)

This document describes how the LLM Router modules depend on each other.

## Core Modules & Their Responsibilities

| Module              | Responsibility                              | Depends On                          | Used By                     |
|---------------------|---------------------------------------------|-------------------------------------|-----------------------------|
| `context`           | `RoutingContext`, `LLMBackend` enum         | None                                | All modules                 |
| `config`            | `RouterConfig`, `ConfigManager`, validation | None                                | `integration`, `strategy`   |
| `complexity`        | Complexity scoring engine                   | `config`                            | `integration`, `strategy`   |
| `schedule`          | Time-of-day / schedule evaluation           | `config`                            | `integration`, `strategy`   |
| `telemetry`         | System load & RTT collection                | None                                | `integration`, `strategy`   |
| `health`            | Backend health probing & status             | None                                | `integration`, `strategy`   |
| `override_mod`      | User override system                        | `context`                           | `integration`, `strategy`   |
| `fallback`          | Fallback chain resolution                   | `context`, `health`                 | `integration`, `strategy`   |
| `strategy`          | Core `route()` decision function            | All of the above                    | `integration`               |
| `integration`       | `decide_backend_with_full_context`          | All modules                         | CPU pipeline / external     |
| `observability`     | Logging + `RouterMetrics`                   | `context`                           | `integration`               |

## Dependency Rules

- **No circular dependencies** are allowed.
- `strategy` is the only module allowed to call most other modules.
- `integration` is the single public entry point for external consumers.
- All data sources (telemetry, health, config, overrides) must be injected into `RoutingContext` before calling `route()`.
- Optional modules (telemetry, health) must gracefully degrade when missing.

## Extension Points (for 146.8)

External code can extend the router via:

- Implementing `RoutingStrategy` trait
- Providing custom `TelemetrySnapshot` / `HealthStatus` sources
- Registering new `RoutingProfile`s via config
- Hooking into `log_routing_decision` for observability
- Using the override system (`/use`, `/prefer` commands)

## Visual Dependency Graph (simplified)

```
                  ┌─────────────────────┐
                  │   CPU Pipeline      │
                  └──────────┬──────────┘
                             │
                             ▼
                  ┌─────────────────────┐
                  │  decide_backend_    │
                  │  with_full_context  │◄── Public API
                  └──────────┬──────────┘
                             │
          ┌──────────────────┼──────────────────┐
          │                  │                  │
          ▼                  ▼                  ▼
   ┌────────────┐     ┌────────────┐     ┌────────────┐
   │  context   │     │  config    │     │  strategy  │
   └────────────┘     └────────────┘     └─────┬──────┘
          │                  │                 │
          │                  │                 ▼
          │                  │          ┌──────────────┐
          │                  │          │ complexity   │
          │                  │          │ schedule     │
          │                  │          │ telemetry    │
          │                  │          │ health       │
          │                  │          │ fallback     │
          │                  │          │ overrides    │
          │                  │          └──────────────┘
          └──────────────────┴──────────────────────────┘
```

This matrix ensures clean separation of concerns and makes future extensions safe.
