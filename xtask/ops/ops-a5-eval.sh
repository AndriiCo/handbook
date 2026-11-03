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
# Fresh private dir per run (mktemp): a fixed path under the store
# collides with leftovers owned by other users (0700, unfixable
# without root) and fails the whole run late. Deleted on exit.
CREDS="$(mktemp -d /tmp/apostille-kek.XXXXXX)"
STAGED=$CREDS/handbook-kek
cleanup() { rm -rf "$CREDS"; }
trap cleanup EXIT

[ -x "$XTASK" ] || { echo "ABORT: no live release xtask at $XTASK"; exit 1; }
[ -f "$KEK_SRC" ] || { echo "ABORT: no KEK source at $KEK_SRC"; exit 1; }
[ "$(stat -c %s "$KEK_SRC")" = 32 ] || { echo "ABORT: KEK source is not 32 bytes"; exit 1; }
"$XTASK" --help 2>/dev/null | grep -q "^  eval " || { echo "ABORT: this xtask has no eval subcommand (deploy first)"; exit 1; }
touch "$STORE/.w" 2>/dev/null && rm -f "$STORE/.w" || {
    echo "ABORT: store $STORE not writable by $USER (eval writes quality.json there)"
    echo "remedy: re-run xtask provision (cargo xtask setup --from provision)"
    ls -ld "$STORE"
    exit 1
}

echo "=== 0. prior quality (read-only)"
cat "$STORE/quality.json" 2>/dev/null || echo "(no quality.json yet)"

echo "=== 1. stage KEK copy (deleted after the run)"
cp "$KEK_SRC" "$STAGED"
chmod 0600 "$STAGED"

echo "=== 2. live eval (one draft call per gold case)"
# Gold cases live in evals/authorities/ (xtask reads *.json flat, never
# recursive) — passing evals/ itself aborts with "no gold files".
CREDENTIALS_DIRECTORY="$CREDS" "$XTASK" eval --live --store "$STORE" --evals /srv/apostille/current/evals/authorities \
    || { echo "ABORT: eval failed (output above)"; rm -f "$STAGED"; exit 1; }

echo "=== 3. cleanup + verify"
rm -rf "$CREDS"
trap - EXIT
[ -e "$STAGED" ] && { echo "ABORT: staged KEK undeletable"; exit 1; }
cat "$STORE/quality.json" || { echo "ABORT: quality.json missing after eval"; exit 1; }
ROOT_OWNED="$(find "$STORE" -user root -print -o -path "$STORE/.loop-creds" -prune 2>/dev/null || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
echo "DONE a5 eval — paste this whole output back to the agent"
