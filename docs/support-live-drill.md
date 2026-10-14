# Support-mail live drill (SYSADMIN-01 task 260.2)

Step-by-step for John. The mock integration (260.1) already proved the loop
end-to-end with stubs; this drill proves it against the real mailbox.
**Nothing in this drill can auto-send** — every reply needs your explicit
approve tap, then your explicit send tap.

## Prerequisites (your ops steps)

1. **mail-mcp installed**: binary at `/home/cobble/.local/bin/mail-mcp` with
   working env credentials (support@micro-tech.xyz mailbox).
2. **Helix on branch `ranger-255-260`** (this branch): `git pull`, rebuild,
   restart the service.
3. **config.toml**: uncomment the `[[mcp.servers]]` "mail" entry
   (template is right after the google entry), and make sure `[support]`
   has `enabled = true`.
4. **Tool-name check** (task 255.3): ask Helix "list your mail tools" (or run
   the bot once and check the log for `support_watch`). You should see
   `mcp__mail__*` tools advertised. The poll loop resolves the check/send
   names defensively at runtime, so exact names don't matter — but if NO
   `mcp__mail__*` tools appear, the watcher stays quiet and the drill stops
   here; tell me what the tool list shows.

## The drill

**a. Send a test email** to support@micro-tech.xyz from another address,
e.g. subject "test", body "my laptop won't turn on".

**b. Watch for the draft.** Within a few minutes (burst mode polls every 1m
while mail is unhandled) a draft should appear:
- Web UI: open the **Support** tab in the detail panel — the draft shows
  with its full text, status "pending".
- Or chat: `/support drafts`.
- Log line to look for: `support_watch: check done mode=BURST new=1 retried=0 unhandled=1 quiet_streak=0`.

**c. VERIFY NOTHING SENT (the critical assertion).** Before you tap anything:
- The draft status must still be **pending**.
- Check the mailbox's Sent folder / mail-mcp outbox: **no reply transmitted**.
- If anything sent without your approve→send taps, STOP the drill and tell
  me immediately — that's a release-blocking bug.

**d. Approve, then send.** In the Support tab (or `/support approve <id>` then
`/support send <id>` — two separate taps, in that order):
- After approve: status → "approved", still nothing sent.
- After send: status → "sent"; the reply should arrive at your test address.
- Sending without approving first is refused (try it if you like — the UI
  and the `/support` commands both reject it).

**e. Discard-path drill.** Send a second test email ("my printer is jammed").
When its draft appears, tap **Discard** (or `/support discard <id>`) instead
of approving. Verify: nothing sent, draft gone from the pending list, message
marked handled (it won't re-draft on the next poll).

**f. Overnight cadence.** Leave it alone overnight. Next day, check the log:
after the quiet period you should see `mode=NORMAL_NIGHT` checks at 30m
intervals, and 15m intervals during 08:00–18:00 ET. Burst mode exits after
2 consecutive clean checks.

## If anything looks wrong

Note the time, what you expected, and what you saw (log lines help), and
send it to me. Do not mark the drill passed on a partial run — I'll file a
follow-up task for any gap.
