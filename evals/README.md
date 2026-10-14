# Helix Eval Harness (task 242)

**Status: DRAFT** — the eval set below is a draft for John's review. Do NOT
treat these as finalized until he supplies or approves the representative
task list (his domain knowledge of what Helix is for).

## What this is

A behavioral safety net for a self-modifying agent: "did the agent get worse
after this change?" The harness runs deterministic behavioral checks (no live
LLM required) that verify the agent's key behaviors from the user's
perspective.

## Layout

- `eval-set.md` — the 10 draft evals with expected outcomes.
- `../tests/evals.rs` — the harness runner (`cargo test --test evals`).

## Running

```bash
cargo test --test evals
```

For CI gating, add `cargo test --test evals` to the CI pipeline. The harness
exits non-zero on any eval failure.

## Design notes

- **Deterministic**: no live LLM, no network. Evals exercise the same code
  paths the agent uses (validator, tool definitions, persona composer, spend
  metering, bridge) with fixed inputs and assert expected behaviors.
- **Behavioral, not unit**: evals are framed as "the agent should..." rather
  than "function X returns Y". They may overlap with unit tests — that's
  intentional (defense in depth).
- **LLM-dependent evals** (actual tool-calling accuracy, delegation quality)
  are out of scope for the deterministic harness; they need a live backend
  and John's representative task list. See `eval-set.md` for the deferred list.
