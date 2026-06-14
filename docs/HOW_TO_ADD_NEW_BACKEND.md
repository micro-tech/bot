# How to Add a New Backend (Task 148.4)

This guide explains how to safely add a new LLM backend to the router.

## Step 1: Extend the LLMBackend Enum

Edit `src/router/context.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LLMBackend {
    Ollama,
    Gemini,
    Grok,
    Claude,        // ← Add your new backend here
    Custom(String),
}
```

## Step 2: Update Backend Sanitization

In `src/router/context.rs`, update `sanitize_backend_name`:

```rust
pub fn sanitize_backend_name(name: &str) -> Option<LLMBackend> {
    match name.to_lowercase().as_str() {
        "ollama" => Some(LLMBackend::Ollama),
        "gemini" => Some(LLMBackend::Gemini),
        "grok"   => Some(LLMBackend::Grok),
        "claude" => Some(LLMBackend::Claude),   // ← Add here
        _ => None,
    }
}
```

## Step 3: Add Health Probing Support

In `src/router/health.rs`, add a probe function for your backend:

```rust
async fn probe_claude() -> HealthStatus {
    // Implement reachability + latency check
    // Return HealthStatus { reachable, latency_ms, ... }
}
```

Register it in the health checker loop.

## Step 4: Update Configuration

Add the new backend to your `router.toml.example` and default `RouterConfig`:

```toml
fallback_chain = ["ollama", "gemini", "claude", "fallback"]
```

Also add it to any named profiles that should use it.

## Step 5: Update Capability Matrix (if applicable)

If your backend supports special capabilities (vision, tool calling, long context), update the capability validation logic in `src/router/context.rs`.

## Step 6: Add Tests

- Unit test for `sanitize_backend_name`
- Integration test that routes to the new backend under specific conditions
- Health probe test

## Step 7: Documentation

Update:
- `docs/LLM_ROUTER_ARCHITECTURE.md`
- `router.toml.example`
- Any "How to" guides

## Checklist

- [ ] Enum variant added
- [ ] Sanitization updated
- [ ] Health probe implemented
- [ ] Config examples updated
- [ ] Tests written
- [ ] Documentation updated

Never add a backend without health checking — the router must be able to detect when it is unavailable.
