# Reasoning Engine

The Reasoning Engine provides a first-class **hypothesis → plan → execute** loop with self-correction, observability, and external control.

## Overview

The engine is integrated into `Cpu<L>` and runs automatically via the heartbeat system. It can also be controlled via bus messages.

## Key Methods (Cpu)

### Control
| Method | Description |
|--------|-------------|
| `start_reasoning(goal)` | Initialize engine with a goal |
| `stop_reasoning()` | Stop and clear the engine |
| `reset_reasoning()` | Full reset |
| `change_goal(new_goal)` | Restart with a new goal |
| `pause_reasoning()` | Temporarily halt cycles |
| `resume_reasoning()` | Resume cycles |
| `force_next_reasoning_step()` | Manually trigger one cycle |

### Observation
| Method | Description |
|--------|-------------|
| `reasoning_metrics()` | Goal, phase, hypothesis count, plan steps, correction cycles |
| `reasoning_health_check()` | Progress detection + status message |
| `reasoning_status()` | Combined paused + metrics + health snapshot |
| `reasoning_summary()` | Redacted current state |
| `reasoning_trace()` | Detailed debug trace |

## Bus Integration

### Published Messages
- `reasoning_metrics` — published every 40 ticks to `web_interface`
- Periodic state updates

### Accepted Commands (`reasoning_command`)
```json
{ "type": "reasoning_command", "command": "pause" }
{ "type": "reasoning_command", "command": "resume" }
{ "type": "reasoning_command", "command": "reset" }
{ "type": "reasoning_command", "command": "force_step" }
```

## Heartbeat Behavior

- Auto-starts on tick 50 if no engine exists (default goal: "Improve agent reliability...")
- Runs one cycle every 20 ticks (unless paused)
- Health is checked during `self_repair()` every 100 ticks

## State

The engine maintains:
- Current goal
- Phase (Proposing, Planning, Executing, Correcting, etc.)
- List of hypotheses
- Current plan + step index
- Correction cycle count

## Self-Healing

If `reasoning_health_check()` reports unhealthy for too long, `self_repair()` can automatically reset the engine.

## Configuration (`config.toml`)

You can configure the Reasoning Engine via the `[reasoning]` section:

```toml
[reasoning]
enabled = true
default_goal = "Improve agent reliability, safety, and self-correction"
cycle_interval_ticks = 20
metrics_interval_ticks = 40
health_check_interval_ticks = 100
```

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `true` | Whether the reasoning engine runs automatically |
| `default_goal` | (see above) | Goal used when auto-starting the engine |
| `cycle_interval_ticks` | `20` | How often a reasoning cycle runs |
| `metrics_interval_ticks` | `40` | How often metrics are published to the bus |
| `health_check_interval_ticks` | `100` | How often health is evaluated in `self_repair` |

If the section is missing, the engine falls back to sensible defaults.

## Files

- `src/reasoning/engine.rs` — Core engine implementation
- `src/cpu/mod.rs` — Integration + public API
- `src/cpu/state.rs` — `reasoning_paused` flag
- `src/config/reasoning.rs` — Configuration loader

---

**Status**: Fully integrated (B1–B14 complete) + configurable via `config.toml`.
