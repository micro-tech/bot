# Testing Guide

This document covers how to run tests for the Agent OS project, with special focus on the integration harness added in Task 122.

## Running the Integration Harness

The integration harness (`tests/integration_harness.rs`) exercises the core subsystems (Bus, Memory, Skills, CPU, HyEvo, LLM, Error paths) through scenario modules.

### Basic Commands

```bash
# Run all integration harness tests
cargo test --test integration_harness

# With full output (recommended during development)
cargo test --test integration_harness -- --nocapture

# Quiet mode (just pass/fail summary)
cargo test --test integration_harness -- --quiet

# Run a specific test
cargo test --test integration_harness test_bus_publish_subscribe_roundtrip -- --nocapture

# Force single-threaded execution (useful for timing-sensitive tests)
cargo test --test integration_harness -- --test-threads=1
```

### Available Test Modules

All tests live under `tests/scenarios/`:

| Module | File | Tests |
|--------|------|-------|
| Bus | `bus_tests.rs` | publish/subscribe round-trip, error propagation |
| Memory | `memory_tests.rs` | context write/read, belief set/get |
| Skills | `skills_tests.rs` | noop skill, list_tools, unknown skill error |
| CPU | `cpu_tests.rs` | memory-write instruction path |
| HyEvo | `hyevo_tests.rs` | basic workflow creation placeholder |
| LLM | `llm_tests.rs` | mocked chat-request flow |
| Error | `error_tests.rs` | invalid JSON, unknown skill handling |

## General Testing

```bash
# Run all tests in the workspace
cargo test

# Run only unit tests (lib)
cargo test --lib

# Run a specific package test
cargo test -p bot

# Check for compilation errors without running tests
cargo check --tests
```

## Common Issues & Fixes

### Windows: "LNK1104 cannot open file ...integration_harness-*.exe"

This occurs when a previous test binary is still locked (common on Windows).

**Fix:**
```powershell
powershell -Command "Stop-Process -Name 'integration_harness*' -Force -ErrorAction SilentlyContinue; Remove-Item -Path 'target\debug\deps\integration_harness*.exe' -Force -ErrorAction SilentlyContinue"
```

Then re-run the test command.

### Tests appear to hang

Some tests use `tokio::time::sleep`. If a subscriber loop is left running, it can block.

**Workarounds:**
- Use `--test-threads=1`
- Reduce sleep durations in the scenario files during debugging

### Want to see internal metrics/logs

Always use `-- --nocapture` when you need to inspect `TestMetrics` or debug output.

## Adding New Scenario Tests

1. Create a new file in `tests/scenarios/your_feature_tests.rs`
2. Add the module declaration in `tests/scenarios/mod.rs`
3. Use `use crate::TestContext;` at the top of your test file
4. Follow the existing pattern (`ctx.bootstrap()`, run test logic, `ctx.shutdown().await`)

## Continuous Integration

The harness is designed to be fast and self-contained. It does **not** require a running Ollama instance (LLM tests are mocked).

---

**Last updated:** Task 122 completion
