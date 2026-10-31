#!/bin/bash
# A5 eval: grade live drafts on the 26-case gold set, write quality.json.
# Run as the deploy user (hal), never root, AFTER the deploy (needs the
# eval subcommand); re-running overwrites the summary. Uses the sealed
# Meta key via a staged KEK copy (deleted after the run). Paste the whole
# output back to the agent.
#
#   bash /home/hal/apostille/scripts/ops-a5-eval.sh
#
set -u

[ "$(id -u)" != 0 ] || { echo "ABORT: run as the deploy user, not root"; exit 1; }

XTASK=/srv/apostille/current/xtask
STORE=/var/lib/apostille
KEK_SRC=/etc/apostille/handbook-kek
CREDS=$STORE/.loop-creds
STAGED=$CREDS/handbook-kek

[ -x "$XTASK" ] || { echo "ABORT: no live release xtask at $XTASK"; exit 1; }
[ -f "$KEK_SRC" ] || { echo "ABORT: no KEK source at $KEK_SRC"; exit 1; }
[ "$(stat -c %s "$KEK_SRC")" = 32 ] || { echo "ABORT: KEK source is not 32 bytes"; exit 1; }
"$XTASK" --help 2>/dev/null | grep -q "^  eval " || { echo "ABORT: this xtask has no eval subcommand (deploy first)"; exit 1; }

echo "=== 0. prior quality (read-only)"
cat "$STORE/quality.json" 2>/dev/null || echo "(no quality.json yet)"

echo "=== 1. stage KEK copy (deleted after the run)"
install -d -m 0700 "$CREDS"
cp "$KEK_SRC" "$STAGED"
chmod 0600 "$STAGED"

echo "=== 2. live eval (one draft call per gold case)"
CREDENTIALS_DIRECTORY="$CREDS" "$XTASK" eval --live --store "$STORE" --evals /srv/apostille/current/evals \
    || { echo "ABORT: eval failed (output above)"; rm -f "$STAGED"; exit 1; }

echo "=== 3. cleanup + verify"
rm -f "$STAGED"
[ -e "$STAGED" ] && { echo "ABORT: staged KEK undeletable"; exit 1; }
rmdir "$CREDS" 2>/dev/null || echo "WARNING: $CREDS not empty, left in place (inspect)"
cat "$STORE/quality.json" || { echo "ABORT: quality.json missing after eval"; exit 1; }
ROOT_OWNED="$(find "$STORE" -user root -print -o -path "$STORE/.loop-creds" -prune 2>/dev/null || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
echo "DONE a5 eval — paste this whole output back to the agent"
