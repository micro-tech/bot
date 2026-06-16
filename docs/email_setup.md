# Email Setup Guide

> **Author:** john mcconnell — john.microtech@gmail.com  
> **Repository:** https://github.com/microtech/grok-cli  
> Last updated: 2026-04-12

This guide covers configuring the bot's email tools for real send (`send_email`)
and read (`read_email`, `check_inbox`) capability.

All credentials go in your `.env` file in the project root — **never commit
this file to git**.

---

## Quick Start

Add the following to `.env` and restart the bot:

```
# ── SMTP (outbound) ───────────────────────────────────────────────────────────
SMTP_HOST=smtp.gmail.com
SMTP_PORT=587
SMTP_USER=you@gmail.com
SMTP_PASSWORD=your-16-char-app-password
SMTP_FROM=you@gmail.com

# ── IMAP (inbound) ────────────────────────────────────────────────────────────
IMAP_HOST=imap.gmail.com
IMAP_PORT=993
IMAP_USER=you@gmail.com
IMAP_PASSWORD=your-16-char-app-password
```

Then test from the chat box:

```
/status          — confirms the bot is running
/tools           — lists all available tools including email
```

Ask the bot:

```
check my inbox
send an email to me@example.com with subject Test and body Hello from bot
```

---

## Provider Setup

### Gmail

Gmail requires an **App Password** — your real Google password will not work
even if 2-FA is disabled.

**Step-by-step:**

1. Enable 2-Step Verification on your Google account  
   <https://myaccount.google.com/security>

2. Go to **App passwords**  
   <https://myaccount.google.com/apppasswords>

3. Click **Select app → Other (custom name)** → type `bot` → **Generate**

4. Copy the 16-character password (shown once — save it)

5. Add to `.env`:
   ```
   SMTP_HOST=smtp.gmail.com
   SMTP_PORT=587
   SMTP_USER=you@gmail.com
   SMTP_PASSWORD=abcd efgh ijkl mnop      # paste as-is, spaces are fine
   SMTP_FROM=you@gmail.com

   IMAP_HOST=imap.gmail.com
   IMAP_PORT=993
   IMAP_USER=you@gmail.com
   IMAP_PASSWORD=abcd efgh ijkl mnop
   ```

6. Enable IMAP in Gmail settings  
   **Settings → See all settings → Forwarding and POP/IMAP → IMAP access → Enable**

---

### Outlook / Microsoft 365

```
SMTP_HOST=smtp.office365.com
SMTP_PORT=587
SMTP_USER=you@outlook.com
SMTP_PASSWORD=your-password
SMTP_FROM=you@outlook.com

IMAP_HOST=outlook.office365.com
IMAP_PORT=993
IMAP_USER=you@outlook.com
IMAP_PASSWORD=your-password
```

> **Note:** If your organisation uses Conditional Access / MFA, you may need an
> app password or OAuth token.  Contact your IT administrator.

---

### Yahoo Mail

```
SMTP_HOST=smtp.mail.yahoo.com
SMTP_PORT=587
SMTP_USER=you@yahoo.com
SMTP_PASSWORD=your-app-password     # generate at account security settings
SMTP_FROM=you@yahoo.com

IMAP_HOST=imap.mail.yahoo.com
IMAP_PORT=993
IMAP_USER=you@yahoo.com
IMAP_PASSWORD=your-app-password
```

Generate a Yahoo app password at:  
<https://login.yahoo.com/account/security>  
**Manage app passwords → Add**

---

### Self-hosted (Postfix / Dovecot / Mailcow)

```
SMTP_HOST=mail.yourdomain.com
SMTP_PORT=587                       # or 465 for SSL
SMTP_USER=bot@yourdomain.com
SMTP_PASSWORD=your-password
SMTP_FROM=bot@yourdomain.com

IMAP_HOST=mail.yourdomain.com
IMAP_PORT=993
IMAP_USER=bot@yourdomain.com
IMAP_PASSWORD=your-password
```

Use `SMTP_PORT=465` if your server requires direct TLS instead of STARTTLS.

---

## Port Reference

| Port | Protocol | When to use |
|------|----------|-------------|
| `587` | STARTTLS | Default — most providers, Gmail, Outlook |
| `465` | SSL/TLS  | Older servers, some self-hosted setups |
| `25`  | Plain    | Server-to-server only — blocked by most ISPs |

The bot automatically selects STARTTLS for port 587 and direct TLS for port 465.

---

## Ubuntu Server — Extra Dependency

The IMAP read feature uses `native-tls` which links against the system OpenSSL.
Run this once on the Ubuntu server before building:

```bash
sudo apt-get install -y libssl-dev pkg-config
```

SMTP send uses pure-Rust TLS (`rustls`) and has no extra system dependencies.

---

## Environment Variable Reference

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SMTP_HOST` | For sending | — | SMTP server hostname |
| `SMTP_PORT` | No | `587` | SMTP port (587 = STARTTLS, 465 = SSL) |
| `SMTP_USER` | For sending | — | SMTP login username |
| `SMTP_PASSWORD` | For sending | — | SMTP password or app password |
| `SMTP_FROM` | No | same as `SMTP_USER` | From address shown to recipients |
| `IMAP_HOST` | For reading | — | IMAP server hostname |
| `IMAP_PORT` | No | `993` | IMAP port (993 = TLS) |
| `IMAP_USER` | For reading | — | IMAP login username |
| `IMAP_PASSWORD` | For reading | — | IMAP password or app password |

All variables are optional at compile time — the bot starts without them and
returns a configuration hint when an email tool is invoked without the relevant
variables set.

---

## Fallback Behaviour (No SMTP Configured)

When `SMTP_HOST`, `SMTP_USER`, or `SMTP_PASSWORD` are absent, `send_email`
appends the message to `logs/email_outbox.md` instead of sending it:

```
---
To: admin@example.com
Subject: Server alert

Disk usage above 90%
```

This means no emails are silently lost — you can inspect the outbox, configure
SMTP, and re-send manually.

---

## Testing the Configuration

### 1 — Check SMTP from the chat UI

Ask the bot:
```
send a test email to your-address@example.com with subject SMTP Test and body It works
```

Expected success response:
```
✅ Email sent to 'your-address@example.com' (subject: 'SMTP Test')
```

### 2 — Check IMAP from the chat UI

Ask the bot:
```
check my inbox
```

Expected response:
```
📬 INBOX — 47 total messages, 2 recent
```

Or ask it to read recent mail:
```
read my last 3 emails
```

### 3 — Test from the command line (SMTP)

```bash
# Send a test using the Rust binary directly via the skill bus
# (or just use swaks / curl if available on the server)
swaks --to you@example.com --server smtp.gmail.com --port 587 --auth LOGIN \
      --au you@gmail.com --ap "your-app-password" --tls
```

---

## Troubleshooting

### `❌ SMTP error: Invalid credentials`

- Gmail: make sure you are using an **App Password**, not your Google account password.
- App passwords are exactly 16 characters, no spaces required (spaces are stripped automatically).
- Check `SMTP_USER` matches the account that generated the app password.

### `❌ SMTP error: Connection refused`

- Verify `SMTP_HOST` and `SMTP_PORT`.
- Run `nc -zv smtp.gmail.com 587` from the server — if it hangs, port 587 is
  blocked by your ISP or firewall.  Try port 465 instead.
- Starlink: outbound port 25 is always blocked. Ports 587 and 465 should work.

### `IMAP connect to imap.gmail.com:993 failed`

- Check `IMAP_HOST` / `IMAP_PORT`.
- Confirm IMAP is enabled in Gmail settings.
- Run `openssl s_client -connect imap.gmail.com:993` from the server to verify
  network connectivity.

### `IMAP login failed`

- Verify `IMAP_USER` and `IMAP_PASSWORD` (same app password as SMTP).
- Gmail: if you see `[AUTHENTICATIONFAILED]`, the app password was entered
  incorrectly or has been revoked — generate a new one.

### `libssl-dev` missing on Ubuntu (IMAP only)

```
error: failed to run custom build command for `openssl-sys`
```

Fix:
```bash
sudo apt-get install -y libssl-dev pkg-config
cargo build
```

### Emails are going to `logs/email_outbox.md` instead of being sent

SMTP is not configured.  Check that all three required variables are set:
```bash
grep SMTP_ .env
```
Expected output should show `SMTP_HOST`, `SMTP_USER`, and `SMTP_PASSWORD`.

---

## Security Notes

- **Never commit `.env` to git.** It is listed in `.gitignore` but git tracks
  files that were committed before the ignore rule was added — run
  `git rm --cached .env` if it was ever committed.

- Use **App Passwords** rather than your main account password wherever
  possible.  If the bot server is compromised, you can revoke the app password
  without changing your main password.

- The SMTP connection uses **rustls** (pure-Rust TLS) — no OpenSSL required for
  sending.  The IMAP connection uses **native-tls** (system OpenSSL) for maximum
  compatibility with IMAP servers.

- Credentials are read from environment variables at runtime — they are never
  logged or written to disk by the bot itself.