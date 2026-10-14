# Backup & Restore — Agent State (task 244)

## What gets backed up

Nightly at 03:00 (system cron on the R630, `scripts/backup-state.sh`), a
timestamped snapshot lands in `<helix-dir>/backups/state-YYYYMMDD-HHMMSS/`:

| File | What it is | Losing it means |
|---|---|---|
| `beliefs.json` | Belief store (memory manager) | Agent forgets what it believes |
| `episodes.jsonl` | Episodic memory | Agent forgets what happened |
| `spend_state.json` | API spend ledger (task 236) | Spend cap resets (could overspend) |
| `agent.md` | Commander persona (task 233) | Persona reverts to default |
| `agents/*.md` | Per-role personas | Role prompts revert |
| `logs/chat_history.jsonl` | Conversation history (task 243) | "Try again" stops resolving |
| `MANIFEST.txt` | Snapshot inventory + sizes | — |

7 most recent snapshots are kept; older ones rotate out automatically.

## Restore procedure

**When:** a bad self-authoring session, disk hiccup, or fresh machine leaves
the agent's state wrong or missing.

**Order matters** — restore in this sequence:

1. **Stop Helix** (so nothing writes mid-restore):
   `sudo systemctl stop helix`

2. **Pick a snapshot**: `ls -t ~/helix/backups/ | head` — newest first.
   Check its `MANIFEST.txt` for the file list and sizes.

3. **Back up the current (broken) state** — never restore over the only copy:
   `cp -r ~/helix/backups ~/helix/backups-before-restore-$(date +%Y%m%d)`

4. **Copy files back** (from the helix dir, e.g. `/home/cobble/helix`):
   ```bash
   SNAP=backups/state-20261011-030000   # pick yours
   cp -p $SNAP/beliefs.json .
   cp -p $SNAP/episodes.jsonl .
   cp -p $SNAP/spend_state.json .
   cp -p $SNAP/agent.md .
   cp -p $SNAP/agents/*.md agents/
   cp -p $SNAP/logs/chat_history.jsonl logs/
   ```

5. **Verify** — every file from the manifest exists and is non-empty:
   ```bash
   diff <(cd $SNAP && find . -type f | sort) \
        <(for f in beliefs.json episodes.jsonl spend_state.json agent.md logs/chat_history.jsonl; do echo "./$f"; done; echo "./MANIFEST.txt" | sort)
   ```
   (Simpler: `ls -la` each file and confirm sizes look sane against the manifest.)

6. **Restart Helix**: `sudo systemctl start helix`, then check the web UI
   loads and the chat history shows prior turns.

## Notes

- Persona files (`agent.md`, `agents/*.md`) already get `.bak` rotation on
  self-edits (task 233) — the snapshot is the off-site equivalent for when
  the whole working dir is at risk.
- The backup script skips missing files (fresh installs), but fails loudly
  if *nothing* was found — that means `HELIX_DIR` is wrong.
- `config.toml` is intentionally NOT in the snapshot (it's git-tracked with
  skip-worktree on the R630; restore it from git).
