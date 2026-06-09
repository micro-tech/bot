# Skills and Hooks

## Skills
Skills are modular, reusable actions that the agent can invoke.

- Implement the `SkillInterface` trait
- Register in `main.rs` or via the skill registry
- Can be called from workflows or directly via CLI

## Hooks
Hooks are lifecycle event handlers that run at specific phases:

- `init`
- `pre_execution`
- `post_execution`
- `error`

They allow cross-cutting concerns like logging, metrics, or safety checks.

## Related Code
- `src/skills/`
- `src/hooks/`
