# How to Tune Router Behavior (Task 148.5)

This guide explains how to adjust the LLM Router's decision-making through configuration.

## 1. Complexity Scoring Weights

Edit the `[complexity]` section in your config:

```toml
[complexity]
token_weight = 0.20          # Longer prompts get higher score
code_weight = 0.25           # Code blocks increase score significantly
reasoning_weight = 0.20      # "explain", "step-by-step", "prove"
math_weight = 0.15
multi_step_weight = 0.10
structural_weight = 0.10
global_threshold = 0.65      # Score above this → prefer stronger model
```

**Tip**: Increase `code_weight` and `reasoning_weight` if you want code and reasoning tasks to more aggressively use Gemini/Grok.

## 2. Load Thresholds

```toml
[load_thresholds]
gpu_max = 85.0
vram_max = 90.0
rtt_max_ms = 40.0
```

When these thresholds are exceeded, the router deprioritizes local backends.

## 3. Health Thresholds

```toml
[health_thresholds]
error_rate_max = 0.05
latency_max_ms = 2000
unreachable_seconds = 30
```

Backends exceeding these are marked degraded or offline.

## 4. Schedule Profiles

Define time-based routing profiles:

```toml
[[schedule.windows]]
start_hour = 22
end_hour = 6
profile = "night"
days = ["Mon", "Tue", "Wed", "Thu", "Fri"]

[profiles.night]
complexity_threshold = 0.40      # Lower bar at night
preferred_backends = ["ollama"]
fallback_chain = ["gemini"]
```

## 5. Fallback Chain

The order matters — put your most reliable backend first:

```toml
fallback_chain = ["ollama", "gemini", "grok", "fallback"]
```

## 6. Environment Variable Overrides

You can override values at runtime without editing files:

```bash
export LLM_ROUTER_LOAD_GPU_MAX=92.0
export LLM_ROUTER_HOT_RELOAD=false
```

## 7. Hot Reload

Set `hot_reload = true` in config. The router will watch the file and apply changes without restart.

## Best Practices

- Start with the defaults and only tune after observing real traffic
- Use the debug tools (Task 147) to see how changes affect decisions
- Keep `global_threshold` between 0.55–0.75 for most workloads
- Test schedule changes during low-traffic periods

Tuning is an iterative process — use golden-file tests to detect unintended behavior changes.
