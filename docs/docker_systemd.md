# Systemd Deployment

Helix runs as a `systemd` service on Linux. The recommended way to install
it is via the built-in **installer binary** — do not manually copy the service
file unless you have a specific reason.

---

## Installer (recommended)

```bash
# Build first
cargo build --release

# Install — prompts for sudo once, then:
#   - copies binary to /usr/local/bin/helix
#   - deploys config, .env, certs to ~/helix/
#   - writes, enables, and starts helix.service
cargo run --bin installer

# Uninstall (preserves config and logs)
cargo run --bin installer -- --uninstall
```

See [Deployment Guide](deployment.md) for the full workflow.

---

## Service file (`helix.service`)

The service file is kept in the project root and deployed by the installer.

```ini
[Unit]
Description=Helix Agent Service
After=network.target

[Service]
Type=simple
User=cobble
WorkingDirectory=/home/cobble/helix
ExecStart=/usr/local/bin/helix
EnvironmentFile=/home/cobble/helix/.env
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
```

Key points:

- `WorkingDirectory` — where Helix looks for `config.toml`, certs, and logs. Must match where those files live.
- `EnvironmentFile` — loads `.env` so the API keys reach the process without being in the unit file.
- `Restart=always` — service auto-restarts on crash with a 5-second delay.

---

## Manual systemctl commands

```bash
sudo systemctl status helix       # check status
sudo systemctl restart helix      # restart after update
sudo systemctl stop helix         # stop
sudo systemctl start helix        # start
sudo systemctl disable helix      # prevent autostart
journalctl -u helix -f            # live log tail
journalctl -u helix -n 50 --no-pager   # last 50 lines
```

---

## Docker

A `Dockerfile` exists in the project root for containerised deployments.

```bash
docker build -t helix .
docker run -p 8443:8443 \
  -e GEMINI_API_KEY=your_key \
  -v $(pwd)/config.toml:/app/config.toml \
  helix
```

> **Note:** The Docker image does not use systemd. For production servers,
> the systemd service approach (above) is preferred.
