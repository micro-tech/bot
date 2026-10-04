# Deployment Guide

How to install, update, and manage Helix on a Linux server.

---

## Prerequisites

- Rust toolchain (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- `openssl` (for TLS cert generation — usually pre-installed)
- `systemd` (standard on Ubuntu/Debian/Fedora servers)

---

## First-Time Install

### 1. Clone the repo

Clone directly into the runtime directory so source and runtime are the same place:

```bash
git clone https://github.com/micro-tech/grok-cli ~/helix
cd ~/helix
```

### 2. Create `.env` with your API keys

```bash
nano ~/helix/.env
```

```env
GEMINI_API_KEY=your_real_gemini_key_here
GEMINI_MODEL=gemini-2.0-flash
```

### 3. Build

```bash
cargo build --release
```

### 4. Run the installer

```bash
cargo run --bin installer
```

The installer will:
- Copy the binary to `/usr/local/bin/helix`
- Deploy `config.toml`, `.env`, certs, and logs to `~/helix/`
- Write and enable `/etc/systemd/system/helix.service`
- Start the service automatically

### 5. Verify

```bash
systemctl status helix
journalctl -u helix -n 30 --no-pager
```

Access the web UI at `https://<server-ip>:8443`.

---

## Update Workflow

```bash
cd ~/helix
git pull
cargo build --release
sudo systemctl restart helix
```

If config files or the service unit changed, re-run the installer instead:

```bash
cargo run --bin installer
```

---

## Branch Strategy

```
prerelease  ──► master  ──► server (runs master)
(dev/test)       (stable)
```

| Branch | Purpose |
|--------|---------|
| `prerelease` | Active development and testing on Windows |
| `master` | Stable — what the server always runs |

### Dev workflow (Windows)

```bash
git checkout prerelease
# make changes, test locally
git add -A && git commit -m "Fix: ..."
git push origin prerelease

# When ready to deploy:
git checkout master
git merge prerelease
git push origin master
```

### Server update

```bash
cd ~/helix
git pull          # gets latest master
cargo build --release
sudo systemctl restart helix
```

---

## Directory Layout

```
/usr/local/bin/helix          ← compiled binary

/home/cobble/helix/           ← runtime working directory (git repo + runtime files)
  Cargo.toml                  ← source
  src/                        ← source
  target/release/helix        ← build artifact
  config.toml                 ← active configuration
  .env                        ← secrets (git-ignored)
  cert.pem / key.pem          ← TLS certs (git-ignored)
  system_manifest.md          ← agent personality/instructions
  logs/
    chat_log.md
    error_log.md
    bus_log.md
    hartbeat_log.md

/etc/helix/                   ← mirror of config files (backup)
/etc/systemd/system/helix.service
```

---

## Uninstall

```bash
cargo run --bin installer -- --uninstall
```

This stops and disables the service and removes the binary. Config files and logs in `~/helix/` are preserved.

---

## Troubleshooting

### Service won't start

```bash
journalctl -u helix -n 50 --no-pager
```

Common causes:
- `.env` missing `GEMINI_API_KEY` — add it and `sudo systemctl restart helix`
- `config.toml` not found — check `WorkingDirectory` in the service matches where config lives
- TLS cert placeholder — re-run installer or generate certs manually:
  ```bash
  openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 3650 -nodes -subj '/CN=localhost'
  sudo systemctl restart helix
  ```

### Server on wrong branch

```bash
cd ~/helix
git branch          # check current branch
git checkout master
git pull
cargo build --release && sudo systemctl restart helix
```

### Gemini not working

1. Check the API key in `.env`: `cat ~/helix/.env`
2. Check the model name — must be a real Gemini model (e.g. `gemini-2.0-flash`, not `gemini-3.1-flash-lite-preview`)
3. Check logs: `journalctl -u helix -n 20 --no-pager`

---

## Related

- [Configuration](configuration.md)
- [Docker & Systemd](docker_systemd.md)
- [Logging](logging.md)
