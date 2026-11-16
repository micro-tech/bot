# Mini-browser install (tasks 350–360)

The mini-browser is a headless Firefox on the R630, driven by Helix's
`browser` tool over WebDriver BiDi (thirtyfour 0.38.3). It exists for one
job: the Google sign-in / Gmail OAuth flow that google-mcp can't finish
on its own (the consent redirect lands on a random localhost port, and
the browser has to be local to the R630), plus user-directed browsing.
It is **not** open-ended LLM web browsing with your Google session.

The installer does all of this automatically (task 360) — this doc is
the manual version of the same steps, for when you want to see or redo
them by hand.

## What the installer does

1. Checks `firefox --version` (minimum 141, target 159) and whether the
   installed Firefox is the Mozilla APT build or the Ubuntu snap.
2. If Firefox is missing, a snap, or below 141: adds the Mozilla APT
   repo (key + sources list + pin, idempotently) and `apt install firefox`.
   **Never the snap** — snap confinement breaks automation and the
   profile paths.
3. Geckodriver is **not** hand-downloaded: thirtyfour's `manager`
   feature downloads the matching geckodriver on first browser launch
   and caches it in `~/.cache/thirtyfour/drivers`. The installer only
   makes sure that cache dir exists and is owned by `cobble`.
4. Ensures the `[browser]` section exists in the live config with safe
   defaults (`enabled = false` — fail-closed) — only if missing, never
   overwriting your values.
5. Creates the browser dirs with `cobble` ownership (profile +
   geckodriver cache). It does **not** touch your personal Firefox
   profile — the Sync copy below is your step.
6. If the Firefox install fails, the installer warns loudly, records it,
   and continues — Helix works without the browser, and the `browser`
   tool reports "browser unavailable" until Firefox is present.

## Manual steps (what the installer encodes)

All commands run on the R630 as `cobble` unless noted (the `sudo`
ones run as root — the installer already re-execs under sudo, so it
asks for the password once for the whole install).

### 1. Check the Ubuntu version

```text
lsb_release -a
```

Firefox 159 needs a current Ubuntu (24.04 on the R630). If the release
is ancient, upgrade Ubuntu first — the Mozilla repo won't have a
matching build.

### 2. Check the current Firefox

```text
firefox --version
snap list firefox
```

- No output from `firefox --version` → Firefox is missing.
- `snap list firefox` shows a row → it's the Ubuntu snap: replace it.
- Version below 141 → too old for BiDi (CDP is gone in Fx 141+).

### 3. Add the Mozilla APT repo and install Firefox

```text
sudo install -d -m 0755 /etc/apt/keyrings
wget -q https://packages.mozilla.org/apt/repo-signing-key.gpg -O- | sudo tee /etc/apt/keyrings/packages.mozilla.org.asc > /dev/null
echo "deb [signed-by=/etc/apt/keyrings/packages.mozilla.org.asc] https://packages.mozilla.org/apt mozilla main" | sudo tee /etc/apt/sources.list.d/mozilla.list > /dev/null
```

Pin Mozilla's Firefox above Ubuntu's snap-shim package:

```text
echo 'Package: *
Pin: origin packages.mozilla.org
Pin-Priority: 1000' | sudo tee /etc/apt/preferences.d/mozilla
```

Then install (and remove the snap first if it was there):

```text
sudo snap remove firefox
sudo apt-get update
sudo apt-get install -y firefox
firefox --version
```

Expect `Mozilla Firefox 159.0` (or newer). Re-running these commands
is safe — the repo/key/pin writes are idempotent, and `apt install`
is a no-op when Firefox is already current.

### 4. Copy your signed-in Firefox profile (task 353 — your step)

The browser signs into your Firefox account with Sync on, so it has
your saved passwords (including Google). The installer deliberately
does **not** do this — it's your profile, your step.

1. On Windows, close Firefox completely (a running Firefox locks the
   profile and Sync won't have flushed the latest state).
2. Copy the profile folder:
   `%APPDATA%\Mozilla\Firefox\Profiles\*.default-release`
   to the R630 as `/home/cobble/.mozilla/firefox/helix`.
3. Set perms 0700 on the copy:
   ```text
   chmod 0700 /home/cobble/.mozilla/firefox/helix
   ```
4. Sync keeps the passwords current after the copy — no re-copy needed
   unless the profile itself is damaged.

Never commit the profile to the repo, and don't back it up there
either. The LLM must never exfiltrate or echo saved passwords — the
browser tool strips password fields from page text and refuses
`evaluate` scripts that read password values.

### 5. Enable the browser in the live config

Edit the **live** config only — `/home/cobble/helix/config.toml`
(never the web UI's config view; it holds a stale copy). Then copy it
to `/etc/helix/config.toml` as usual.

```toml
[browser]
enabled = true
profile_dir = "~/.mozilla/firefox/helix"
headless = true
action_timeout_secs = 30
allowed_hosts = ["google.com", "localhost", "127.0.0.1"]
gmail_token_path = "~/.config/google-mcp/token.json"
```

`allowed_hosts` is the navigation allowlist (exact host or subdomain).
Google is on it by default; any other host needs your confirmation
when the LLM asks to go there.

### 6. Restart Helix

```text
sudo cp ~/helix/config.toml /etc/helix/config.toml
sudo systemctl restart helix
```

## Headless navigation smoke test

With the browser enabled, ask Helix (web chat, any backend) to run:

```text
browser navigate https://www.google.com
browser status
```

Expected: the navigate returns the Google URL + page title, and
`status` reports the session up with the current URL. If it reports
"browser unavailable", check `firefox --version` on the R630 and the
`[browser] enabled` line in the live config.

A deeper check (on the R630, in the repo checkout):

```text
cargo test -- --ignored browser_real_smoke
```

That test launches a real headless Firefox, navigates to a local page,
and exercises click/type/screenshot. It is `#[ignore]`d in the normal
suite because the sandbox has no Firefox.

## What if it fails

- **Snap installed by mistake.** `snap list firefox` shows it, or
  `firefox --version` works but automation fails with profile/permission
  errors. Fix: `sudo snap remove firefox`, then re-run step 3. The
  Mozilla APT build must win — check the pin file
  `/etc/apt/preferences.d/mozilla` exists.
- **Geckodriver download blocked.** The first browser launch downloads
  geckodriver into `~/.cache/thirtyfour/drivers`. If the R630 has no
  outbound network at that moment, the launch fails with a download
  error and the tool reports "browser unavailable". Fix: check the
  network, then retry the launch (Helix `browser restart`). The cache
  dir must be owned by `cobble`, not root — the installer chowns it;
  if you created it by hand as root, `sudo chown -R cobble:cobble
  ~/.cache/thirtyfour`.
- **Profile lock file left behind.** If Firefox was killed mid-run,
  `.parentlock` / `lock` may remain in the profile dir and the next
  launch refuses to start. Fix: make sure no `firefox` process is
  running (`pgrep -x firefox`), then remove the lock file from
  `/home/cobble/.mozilla/firefox/helix/.parentlock`. Never delete the
  lock while Firefox is actually running.
- **Google 2FA challenge.** The R630 browser can't do a WebAuthn
  security-key challenge — your YubiKey is on the Windows machine.
  Use the **Browser 2FA** box in the web UI (ops area): Screenshot to
  see the challenge, then approve the Google prompt on your phone, or
  type the authenticator-app code with the Type box and press Enter
  with the Key box. (Task 355.)
- **Gmail OAuth still fails.** The browser can only complete the flow
  if the google-mcp client secret in the live config is the current
  one (the new secret created 2026-11-14, ending `hldr`). If the
  consent page errors about the client, check
  `[mcp.servers.env]` in `/home/cobble/helix/config.toml` first.

## Versions

- Firefox: target **159** (current stable, Nov 2026), minimum **141**
  (BiDi requires it — CDP removed).
- Driver: **thirtyfour 0.38.3** (features `bidi`, `manager`), pinned in
  `Cargo.toml`. geckodriver version is managed by thirtyfour, not
  pinned by hand.
- Live view (task 358) is **not** built yet — the 2FA box is a
  one-shot screenshot, not a video stream. Firefox/BiDi screencast is
  still in flight (Firefox M20 targets screen-recording).
