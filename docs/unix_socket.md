# UNIX Domain Socket CLI

The UNIX socket interface provides a powerful, scriptable way to interact with Agent OS.

## Features
- JSON-lines protocol
- Concurrent client support
- Structured error responses
- Schema validation on commands
- Built-in commands: `ping`, `health`, `version`, `checksum`, `metrics`, `plugins`, `run-plugin`, etc.

## Location
- Default socket path: configurable in `config.toml`
- Permissions: `0o660` by default (owner + group)

## Usage Example
```bash
echo '{"cmd":"ping"}' | nc -U /tmp/agentos.sock
```

## Related Modules
- `src/io/unix_socket.rs`
- `src/io/unix_cli/`

See also: [CLI Commands Reference](cli_commands.md)
