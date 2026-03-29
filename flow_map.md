# Flow Map

This document describes the key data and control flows in the bot project. It includes text descriptions and Mermaid diagrams for visualization. Flows cover execution, memory, skills/hooks, and the new HyEvo evolutionary layer.

## Overall System Flow
1. **Initialization**: `main.rs` creates `Executor`, `MemoryHandle`, `SkillRegistry`, `HookRegistry`, and `HyEvoIntegration`.
2. **Event Loop**: Executor polls interrupts/bus for `CpuEvent`s, runs phases (hooks), executes skills/LLMs, updates beliefs in memory.
3. **Feedback Loop**: Metrics from executions feed into HyEvo for workflow evolution.
4. **Output**: Processed events produce actions (e.g., API calls via skills) or evolved behaviors.

## Core Execution Flow (CPU Executor)
Text Flow:
- Receive `CpuEvent` via bus/interrupts.
- Run init phase (hooks).
- Read belief from memory (key-based).
- Execute skill (lookup in registry, pass args as HashMap -> Value).
- Write/update belief in memory.
- Run post-phase (hooks).
- Serialize event for logging/queue.

Mermaid Diagram:
```mermaid
graph TD
    A[Bus/Interrupts: CpuEvent] --> B[Executor: Poll try_recv]
    B --> C[Hooks: run_phase('init', state)]
    C --> D[Memory: read(key)]
    D --> E[Skills: run(name, args)]
    E --> F[Memory: write/update_belief(key, value)]
    F --> G[Hooks: run_phase('post', state)]
    G --> H[Output: Action or Log]
    style A fill:#f9f,stroke:#333
```

## Memory Management Flow
- Async reads/writes via `MemoryInterface`.
- Context joining: `VecDeque<String>.iter().join(" ")` for concatenated beliefs.
- Beliefs updated post-execution (e.g., after skill run).

## Skills and Hooks Flow
- **Skills**: Registry lookup by name; execute async Fn with params (HashMap serialized to Value).
- **Hooks**: Phase-based (e.g., "init"); sequential execution on mutable state.

## HyEvo Evolutionary Flow (NEW SECTION)
**Purpose**: Evolves workflows (sequences of Nodes: Skill/LLM/Code) using genetic algorithms and LLM reflection. Integrates with executor for dynamic adaptation.

Text Flow:
1. **Seeding**: Initialize `HyEvoEngine` with initial genome (workflow JSON/tree).
2. **Execution**: Executor retrieves `best_workflow()` as Nodes; executes via `execute_workflow` (converts params HashMap to Value).
3. **Evaluation**: Collect metrics (e.g., success rate, latency) from executor runs.
4. **Evolution**: Call `evolve_once(metrics)`; uses `ReflectionLlm` to mutate/reflect on workflows (e.g., prompt: "Improve this skill sequence for better performance").
5. **Integration Loop**: Lock `HyEvoIntegration` mutex; update executor's workflow on each cycle.
6. **Output**: Best evolved workflow cloned and injected into executor for next events.

Mermaid Diagram (HyEvo Integration):
```mermaid
graph LR
    A[main.rs: Init HyEvoIntegration<L: ReflectionLlm>] --> B[Seed: engine.seed(initial_genome)]
    B --> C[Executor: Get best_workflow() via integration.lock()]
    C --> D[Execute Workflow: Match Node -> execute_skill/llm/code]
    D --> E[Skills/Hooks/Memory: Run Node Actions]
    E --> F[Collect Metrics: e.g., success, time]
    F --> G[Evolve: engine.evolve_once(metrics).await]
    G --> H[LLM Reflection: Prompt for mutations/improvements]
    H --> I[New Population: Mutate/Select Workflows]
    I --> C
    style G fill:#ff9,stroke:#f66  %% Highlight evolution step
```

**HyEvo Node Execution Sub-Flow** (Within Workflow):
- Skill Node: `params: HashMap -> to_value()` -> `execute_skill(name, &Value)`.
- LLM Node: Similar, with model/prompt.
- Code Node: Executes custom Rust/JS-like code with params.
- Error Handling: `NodeResult` propagates up.

## Interrupt and Event Flow
- `try_recv()` on bus returns `Result<Message, TryRecvError>`; loop with `while let Ok(msg)`.
- Events trigger executor methods (e.g., handle `CpuEvent::Interrupt`).

## Error Handling Flow
- All async ops use `?` propagation via `NodeResult`.
- Serde serialization for events/params (e.g., `json!({ "event": ev })` requires `#[derive(Serialize)]` on `CpuEvent`).

## Future Flows
- **Metrics to External**: Feed HyEvo metrics to a dashboard (e.g., Prometheus).
- **Multi-Agent**: Parallel executors sharing HyEvo engine via Arc<Mutex>.
- **Visualization**: Generate DOT files from workflows for Graphviz rendering.
