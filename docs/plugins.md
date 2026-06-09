# Plugin System

Agent OS supports a lightweight plugin architecture for extending CLI functionality.

## How It Works
- Plugins are executable files placed in `~/.bot/plugins/` (or `%APPDATA%\bot\plugins\` on Windows)
- Discovered automatically via the `plugins` command
- Executed via the `run-plugin` command

## Commands
- `plugins` / `list-plugins` — List available plugins
- `run-plugin` — Execute a plugin with arguments

## Example
```json
{"cmd": "run-plugin", "args": {"name": "my_script.sh", "args": ["--verbose"]}}
```

## Security Note
Plugins run with the same permissions as the bot process. Use with caution.

## Related Code
- `src/io/unix_cli/plugins.rs`
