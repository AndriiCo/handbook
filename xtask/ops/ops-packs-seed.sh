#!/bin/bash
# Full packs seed: load ALL packs in the live release into prod.
# Load ONLY — never fetch; the loop timers pick up polling on
# cadence, so there is no burst. Run as root; re-running is safe
# (packs-load is exists-safe: INSERT OR IGNORE + task dedup keys).
#
#   bash /home/hal/apostille/scripts/ops-packs-seed.sh
#
# Paste the whole output back to the agent.
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }

XTASK=/srv/apostille/current/xtask
STORE=/var/lib/apostille
DB="$STORE/handbook.sqlite"
PACKS=/srv/apostille/current/packs
SVC="sudo -u andrii-ai"

[ -x "$XTASK" ] || { echo "ABORT: no live release xtask at $XTASK"; exit 1; }
[ -d "$PACKS" ] || { echo "ABORT: no live release packs at $PACKS (deploy first)"; exit 1; }
[ -f "$DB" ] || { echo "ABORT: no prod DB at $DB"; exit 1; }
[ -n "$(ls "$PACKS"/*.json 2>/dev/null)" ] || { echo "ABORT: no pack files in $PACKS"; exit 1; }

echo "=== 0. prior state (read-only)"
BEFORE_AUTHORITY="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM authority;")"
BEFORE_DOMAIN="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM domain;")"
BEFORE_SOURCE="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM source;")"
BEFORE_TASK="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM task;")"
echo "before: authority=$BEFORE_AUTHORITY domain=$BEFORE_DOMAIN source=$BEFORE_SOURCE task=$BEFORE_TASK"

echo "=== 1. packs-load (all live packs; self-validates, refuses on invalid)"
$SVC "$XTASK" packs-load --packs "$PACKS" --store "$STORE" --actor packs-seed \
    || { echo "ABORT: packs-load failed (output above)"; exit 1; }

echo "=== 2. verify: counts after (must be >= before on every table)"
AFTER_AUTHORITY="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM authority;")"
AFTER_DOMAIN="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM domain;")"
AFTER_SOURCE="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM source;")"
AFTER_TASK="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM task;")"
echo "after: authority=$AFTER_AUTHORITY domain=$AFTER_DOMAIN source=$AFTER_SOURCE task=$AFTER_TASK"
[ "$AFTER_AUTHORITY" -ge "$BEFORE_AUTHORITY" ] || { echo "ABORT: authority count went backwards"; exit 1; }
[ "$AFTER_DOMAIN" -ge "$BEFORE_DOMAIN" ] || { echo "ABORT: domain count went backwards"; exit 1; }
[ "$AFTER_SOURCE" -ge "$BEFORE_SOURCE" ] || { echo "ABORT: source count went backwards"; exit 1; }
[ "$AFTER_TASK" -ge "$BEFORE_TASK" ] || { echo "ABORT: task count went backwards"; exit 1; }

echo "=== 3. root-residue check"
ROOT_OWNED="$(find "$STORE" -user root -print || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
echo "DONE packs seed: before authority=$BEFORE_AUTHORITY domain=$BEFORE_DOMAIN source=$BEFORE_SOURCE task=$BEFORE_TASK after authority=$AFTER_AUTHORITY domain=$AFTER_DOMAIN source=$AFTER_SOURCE task=$AFTER_SOURCE — paste this whole output back to the agent"
