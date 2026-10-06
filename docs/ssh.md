# SSH server into Helix

Helix can run its own SSH server (russh 0.63, `aws-lc-rs` crypto) so John can
open a shell into the Helix service from the box itself (or the Tailscale
interface) and run commands. Disabled by default.

```toml
[ssh]
enabled = false                    # master switch — the listener never starts unasked
bind = "127.0.0.1"                 # loopback by default
port = 2222                        # avoids the system sshd on 22
authorized_keys = "ssh/authorized_keys"  # relative → <config-dir>/ssh/
host_key = "ssh/ssh_host_key"            # generated (Ed25519, 0600) on first start if missing
max_sessions = 8                   # concurrent session cap
idle_timeout_secs = 900            # idle sessions reaped
auth_rate_limit_per_minute = 10    # per-IP failed-auth throttle
```

`authorized_keys` / `host_key` default under `<config-dir>/ssh/` next to the
config file Helix actually loaded. Set them up once:

```bash
# on the Helix box, as the service user
mkdir -p /etc/helix/ssh
cat ~/.ssh/id_ed25519.pub >> /etc/helix/ssh/authorized_keys
chmod 600 /etc/helix/ssh/authorized_keys
# then in config.toml: [ssh] enabled = true  (and [shell] enabled = true)
```

Connect: `ssh -i ~/.ssh/id_ed25519 -p 2222 helix@127.0.0.1 'systemctl status helix'`

## Threat model

- **Asset:** shell on the Helix service user's machine.
- **Adversaries:** attacker on the LAN; compromised Tailscale peer pivoting;
  malicious prompt-injection or web content trying to widen the bind or add
  keys.
- **Controls:**
  - Public-key auth ONLY. No passwords, no keyboard-interactive, ever —
    the russh defaults reject and we don't override them.
  - Keys come from the dedicated Helix-managed file, NOT the service user's
    `~/.ssh/authorized_keys` — Helix access stays auditable and separate
    from John's own keys. Compared by key material (OpenSSH comments are
    ignored).
  - Bind `127.0.0.1` by default. The only documented non-loopback option is
    the Tailscale interface IP. `0.0.0.0` requires an explicit
    `bind = "0.0.0.0"` in the config file AND logs a loud startup warning.
  - **Bind comes ONLY from the config file** — there is no tool, chat
    command, or web endpoint that can change it (prompt-injection guard).
  - **Fail closed:** no authorized keys (missing/empty/unparseable) → the
    listener refuses to start with a clear log line; it never accepts
    connections in a degraded state. SSH also refuses to start when
    `[shell]` is disabled.
  - Auth attempts rate-limited per IP (sliding 1-minute window) plus russh's
    per-connection `max_auth_attempts` and constant-time rejection delay.
    Every attempt is logged with user, peer IP, and key fingerprint
    (SHA256) — never key material.
  - Idle sessions reaped (`idle_timeout_secs`); concurrent sessions capped
    (`max_sessions`, extra channels get `ResourceShortage`).
  - Same posture as the web lockdown (tasks 183–185): loopback bind, strong
    auth, no silent fallbacks.

## One shell path

SSH exec channels run commands through `run_shell` (task 191) — the
`[shell]` gate, destructive-shell denylist, timeout with process-group kill,
output caps, no TTY, workdir confinement. There is exactly ONE shell
execution path, not a second one with its own rules. A denylisted command
over SSH is blocked with exit code 126; timeouts exit 124.

v1 is **exec-channel only** (`ssh -p 2222 helix@127.0.0.1 'command'`).
Interactive `shell_request`/PTY is rejected with a clear message — accepting
it would be a second shell path outside the `run_shell` policy. PTY support
is a follow-up, as is SFTP (out of scope; system sshd is untouched).
