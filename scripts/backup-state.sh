#!/usr/bin/env bash
# Helix agent-state backup (task 244).
#
# Nightly snapshot of the agent's accumulated self: beliefs, episodic memory,
# spend state, persona files, and chat history. A bad self-authoring session
# or disk hiccup must not vaporize these.
#
# Usage: backup-state.sh [helix-dir]
#   helix-dir defaults to $HELIX_DIR or the current directory.
#
# Snapshots land in <helix-dir>/backups/state-YYYYMMDD-HHMMSS/.
# Keeps the 7 most recent snapshots; older ones rotate out.
#
# Install (on the R630, as the helix user):
#   crontab -e
#   0 3 * * * /home/cobble/helix/scripts/backup-state.sh /home/cobble/helix >> /home/cobble/helix/logs/backup.log 2>&1
#
# Restore: see docs/backup-restore.md

set -euo pipefail

HELIX_DIR="${1:-${HELIX_DIR:-.}}"
cd "$HELIX_DIR"
HELIX_DIR_ABS="$(pwd)"

BACKUP_ROOT="backups"
KEEP=7
STAMP="$(date +%Y%m%d-%H%M%S)"
DEST="$BACKUP_ROOT/state-$STAMP"

mkdir -p "$DEST"

# Files that constitute the agent's accumulated self. Missing files are
# skipped (fresh installs won't have all of these yet) — but at least one
# must exist, or something is wrong with HELIX_DIR.
copied=0
for f in beliefs.json episodes.jsonl spend_state.json agent.md logs/chat_history.jsonl; do
    if [ -f "$f" ]; then
        mkdir -p "$DEST/$(dirname "$f")"
        cp -p "$f" "$DEST/$f"
        copied=$((copied + 1))
    fi
done
if [ -d "agents" ]; then
    mkdir -p "$DEST/agents"
    # cp returns non-zero when agents/ is empty — don't fail the backup.
    cp -p agents/*.md "$DEST/agents/" 2>/dev/null || true
    copied=$((copied + 1))
fi

if [ "$copied" -eq 0 ]; then
    echo "backup-state: WARNING: no state files found in $HELIX_DIR — is HELIX_DIR right?" >&2
    rmdir "$DEST" 2>/dev/null || true
    exit 1
fi

# Manifest: what, when, and sizes — for the restore doc's verification step.
{
    echo "# Helix state snapshot $STAMP"
    echo "# host: $(hostname)  dir: $(pwd)"
    du -sh "$DEST" | awk '{print "# size: " $1}'
    echo "# files:"
    cd "$DEST" && find . -type f | sort
} > "$DEST/MANIFEST.txt"

# Rotate: keep the $KEEP most recent, delete older.
# (ls sorts lexically; the YYYYMMDD-HHMMSS stamp sorts chronologically.)
# Use the absolute path — the manifest block above cd's into $DEST.
old=$(ls -d "$HELIX_DIR_ABS/$BACKUP_ROOT"/state-* 2>/dev/null | head -n -"$KEEP" || true)
if [ -n "$old" ]; then
    echo "$old" | xargs rm -rf
    echo "backup-state: rotated out $(echo "$old" | wc -l) old snapshot(s)"
fi

echo "backup-state: snapshot $DEST complete ($copied item(s))"
