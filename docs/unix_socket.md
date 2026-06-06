# UNIX Socket CLI Interface (Tasks 76-87)

- Socket path: configurable via `config/socket.rs`
- Permissions: 0o660 by default
- Protocol: JSON-lines
- Concurrency: tokio::spawn per client
- Bus integration: messages published under "unix_socket" topic

See src/io/unix_socket.rs for implementation.
