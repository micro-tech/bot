# LLM Router — Concrete Examples (Task 148.7)

This document contains realistic `RoutingContext` → `BackendSelection` examples for the most common scenarios.

## Example 1: Simple Question (Low Complexity)

**Input**
```rust
RoutingContext {
    prompt: "What is the capital of France?",
    token_estimate: 12,
    has_code: false,
    complexity_score: 0.18,
    timestamp: "2025-01-15T10:30:00Z",
    user_override: None,
    telemetry: Some(TelemetrySnapshot { gpu_util: 32.0, ... }),
    health: Some({Ollama: healthy, Gemini: healthy}),
}
```

**Expected Output**
```rust
BackendSelection {
    chosen: Ollama,
    fallbacks_attempted: [],
    reason: "low complexity, local backend preferred",
    timestamp: "...",
    health_snapshot: None,
}
```

## Example 2: Code + Reasoning Task (High Complexity)

**Input**
```rust
RoutingContext {
    prompt: "Explain how to implement a lock-free ring buffer in Rust with step-by-step reasoning.",
    token_estimate: 420,
    has_code: true,
    complexity_score: 0.81,
    ...
}
```

**Expected Output**
```rust
BackendSelection {
    chosen: Gemini,
    fallbacks_attempted: [],
    reason: "high complexity score + code + reasoning keywords",
    ...
}
```

## Example 3: User Override Active

**Input**
```rust
RoutingContext {
    ...
    user_override: Some(Grok),
    ...
}
```

**Expected Output**
```rust
BackendSelection {
    chosen: Grok,
    fallbacks_attempted: [],
    reason: "user override",
    ...
}
```

## Example 4: Primary Backend Unhealthy → Fallback

**Input**
```rust
health: Some({
    Ollama: {reachable: false, degraded: true, ...},
    Gemini: {reachable: true, ...}
})
```

**Expected Output**
```rust
BackendSelection {
    chosen: Gemini,
    fallbacks_attempted: [(Ollama, "primary degraded/unreachable")],
    reason: "primary unhealthy, used fallback",
    ...
}
```

## Example 5: High System Load

**Input**
```rust
telemetry: Some(TelemetrySnapshot {
    gpu_util: 94.0,
    vram_used: 21.8,
    ...
})
```

**Expected Output**
```rust
BackendSelection {
    chosen: Gemini,   // or Grok
    reason: "high GPU load, prefer remote backend",
    ...
}
```

## Example 6: Night Schedule Profile

**Input**
```rust
timestamp: "2025-01-15T23:45:00Z",   // 23:45
schedule.windows: [{start_hour: 22, end_hour: 6, profile: "night"}]
profiles.night: {complexity_threshold: 0.40, preferred_backends: ["ollama"]}
```

**Expected Output**
```rust
BackendSelection {
    chosen: Ollama,
    reason: "night profile active, lower complexity threshold",
    ...
}
```

These examples are used in golden-file regression tests and documentation.
