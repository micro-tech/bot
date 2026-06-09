# Metrics Collection

Agent OS records usage metrics for CLI commands.

## Features
- Per-command timing and success/failure tracking
- Stored in `~/.bot/metrics.json` (or `%APPDATA%\bot\metrics.json` on Windows)
- Exportable via the `metrics` command

## Command
```json
{"cmd": "metrics"}
```

## Output
Returns a JSON-lines file with entries containing:
- `ts`: Unix timestamp
- `cmd`: Command name
- `duration_ms`
- `success`

## Related Code
- `src/io/unix_cli/metrics.rs`
