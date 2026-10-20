#!/usr/bin/env bash
# push.sh — push Helix to GitHub without the live config.toml getting in the way.
#
# The live config.toml carries secrets and must never be committed. This script:
#   1. Refuses to run if config.toml is already inside an unpushed commit —
#      hiding the file cannot scrub commit history, and GitHub scans commits.
#   2. Untracks config.toml if it ever got added to the index (file stays on disk).
#   3. Moves config.toml aside, pushes the current branch, and always moves it back.
set -euo pipefail

cd "$(dirname "$0")"

CONFIG="config.toml"
HIDDEN="$HOME/.config.toml.pushhidden"
BRANCH="$(git branch --show-current)"

fail() { echo "push.sh: $*" >&2; exit 1; }

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || fail "not inside a git repo"

# 1. Is config.toml hiding inside commits GitHub has not seen yet?
if git log HEAD --not --remotes --oneline -- "$CONFIG" 2>/dev/null | grep -q .; then
  fail "config.toml is inside unpushed commits. Hiding the file won't help —
GitHub scans the commits themselves. That history needs the reset cleanup
(back up the live config, reset to origin, restore it). Ask Cobble jr."
fi

# 2. Untrack config.toml if it is in the index (the file on disk is untouched).
if git ls-files --error-unmatch "$CONFIG" >/dev/null 2>&1; then
  echo "push.sh: config.toml is tracked — untracking it (file stays on disk)."
  git rm --cached --quiet "$CONFIG"
  echo "push.sh: note — the untracking takes effect on your next commit."
fi

# 3. Hide the live config, and restore it no matter how the push ends.
restore() {
  if [ -f "$HIDDEN" ]; then
    mv "$HIDDEN" "$CONFIG"
    echo "push.sh: config.toml restored."
  fi
}
trap restore EXIT

if [ -f "$CONFIG" ]; then
  [ ! -f "$HIDDEN" ] || fail "$HIDDEN already exists — a previous run may not have restored. Check it before rerunning."
  mv "$CONFIG" "$HIDDEN"
  echo "push.sh: config.toml hidden."
fi

# 4. Push the current branch.
echo "push.sh: pushing $BRANCH..."
git push origin "$BRANCH"
echo "push.sh: done."
