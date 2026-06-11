# Integration Test Harness Design (Task 122)

## Overview
Build a fully automated integration test harness (`cargo test --test integration_harness` or a dedicated `test_harness` binary) that exercises the entire bot system end-to-end.

## Architecture Decisions

### 1. Execution Model
- **Primary**: `tests/integration_harness.rs` (cargo integration test)
- **Alternative**: Separate binary `src/bin/test_harness.rs` for CI / standalone runs
- Use `#[tokio::test]` with multi-thread runtime

### 2. Bootstrap Strategy
- Start a **real** `Bus` (in-memory)
- Start core modules in background tasks:
  - CPU executor
  - Memory system
  - Skills/hooks registry
  - HyEvo engine (lightweight mode)
  - Mock Ollama / Gemini endpoints (or real if configured)
- Use **mocks** for external services (web server, sockets, Ollama) when full stack is not needed
- Provide a `TestContext` struct that holds:
  - `bus: Arc<Bus>`
  - `handles: Vec<JoinHandle>`
  - `metrics_collector`
  - `temp_dirs`

### 3. Test Scenarios Categories

#### A. Core Bus & Messaging
- Publish/subscribe roundtrip
- Message ordering & correlation IDs
- Error propagation on bad subscribers

#### B. CPU Executor
- Load & execute simple workflow
- Skill node execution
- LLM node (mocked)
- Error recovery & retry

#### C. Memory System
- Save / recall facts
- Vector search (if enabled)
- Persistence across restarts (temp DB)

#### D. Skills & Hooks
- Register & invoke skill
- Hook trigger via bus
- Error handling in skills

#### E. HyEvo / Workflow Evolution
- Create workflow
- Run evolution cycle (small population)
- Score & mutate
- Reflection cycle

#### F. Interfaces
- Web server (axum test client)
- UNIX socket (if on Unix)
- Terminal / CLI commands

#### G. Ollama / LLM Integration
- Chat request → response flow
- Tool calling roundtrip
- Fallback between backends

#### H. Error Paths & Recovery
- Bus message with invalid JSON
- Ollama timeout / 500
- Workflow with dangling edges
- Memory corruption simulation

#### I. Performance Baselines
- Measure end-to-end latency for common flows
- Count LLM calls, errors, success rate
- Resource usage (optional)

### 4. Result Collection & Reporting
- `TestReport` struct:
  - `passed: usize`
  - `failed: usize`
  - `duration_ms`
  - `scenarios: Vec<ScenarioResult>`
  - `metrics: HashMap<String, f64>`
- Output: JSON + human readable
- Exit code: 0 on all pass, 1 on any failure

### 5. CI-Friendly Features
- `--filter <pattern>` to run subset
- `--json` output
- `--timeout <secs>`
- `--mock-ollama` flag
- Environment variable overrides for config

### 6. File Structure
```
tests/
  integration_harness.rs          # Main harness
  scenarios/
    bus_tests.rs
    cpu_tests.rs
    hy_evo_tests.rs
    ...
  mocks/
    ollama_mock.rs
    web_client.rs
```

## Next Steps (Subtasks)
1. [122.1] Finalize this design + scenario list
2. [122.2] Implement bootstrap + TestContext
3. [122.3–122.6] Implement scenario runners
4. [122.7] Result collection & assertions
5. [122.8] CLI flags
6. [122.9] Documentation

---
*Created during Task 122 execution*
