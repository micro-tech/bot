# Checksum & Resume Transfers

Agent OS includes built-in support for reliable file transfers over the CLI.

## Features
- **SHA-256 Checksums**: Verify file integrity with the `checksum` command
- **Resume Support**: Track transfer offsets for resumable uploads/downloads

## Commands
- `checksum` — Calculate SHA-256 of a file
- `resume-offset` — Get the last known offset for a path

## Usage
```json
{"cmd": "checksum", "args": {"path": "/tmp/largefile.bin"}}
{"cmd": "resume-offset", "args": {"path": "/tmp/largefile.bin"}}
```

## Implementation
- `src/io/unix_cli/checksum.rs`
- `src/io/unix_cli/resume_transfer.rs`
