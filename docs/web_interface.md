# Web Interface

Agent OS includes a full-featured HTTPS web server.

## Features
- Real-time chat via WebSockets
- Live log viewing (`error_log.md`, `chat_log.md`, etc.)
- Self-signed certificate support (auto-generated)
- Responsive UI

## Endpoints
- `GET /` — Main chat interface
- `GET /ws` — WebSocket connection
- `GET /logs` — Log viewer

## Configuration
```toml
[web_interface]
https_port = 8443
```

## Related Code
- `src/io/web_server/mod.rs`
