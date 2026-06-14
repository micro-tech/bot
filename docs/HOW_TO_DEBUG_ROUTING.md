# How to Debug Routing Decisions (Task 148.6)

This guide explains how to investigate and understand why the router made a particular decision.

## 1. Enable Structured Logging

Make sure your logging is set to capture router events:

```rust
log_routing_decision(&ctx, &selection);
```

Look for lines containing `[ROUTER]` or `RoutingContext`.

## 2. Use the `router_debug` Command

If your CLI exposes it:

```bash
bot router_debug "Explain quantum entanglement in detail"
```

This will output:
- Human-readable narrative
- Full decision tree (JSON)
- Which factors influenced the choice

## 3. Capture a Snapshot

Use the debug module to save a routing decision for later analysis:

```rust
save_snapshot(&tree, "debug_snapshot.json");
```

Later you can reload it:

```rust
let tree = load_snapshot("debug_snapshot.json")?;
```

## 4. Read the Explain Output

A typical explain output looks like:

```
1. Analyzed prompt (420 tokens, code: false)
2. Complexity score calculated: 0.78
3. User override applied: Gemini
4. Health status considered during selection
5. Telemetry: GPU 72.0%, VRAM 18.3GB
6. Selected backend: Gemini
```

## 5. Common Questions & Answers

**Q: Why did it choose Gemini instead of Ollama?**
- Check the complexity score
- Check if there was a user override
- Check health/telemetry status of Ollama

**Q: Why was a fallback used?**
- Look at the `BackendSelection` fallback list and reason codes

**Q: The decision changed after I updated config — why?**
- Use golden-file tests or snapshot replay to compare before/after

## 6. Production Safety

Always use the redaction function when sharing debug output that may contain user prompts:

```rust
let safe_tree = redact_sensitive(tree);
```

## 7. TUI Inspector

For interactive debugging, run the simple TUI:

```rust
run_simple_tui(&tree);
```

This is especially useful during development or when attached to a running instance.

With these tools you can answer almost any question about routing behavior.
