#!/bin/bash
# A2 HCCH seed: the ONE status-table URI into prod (packs-load,
# then fetch-url from prod IPs), then prove hcch-poll unblocked.
# Run as root; re-running is safe (load + fetch are exists-safe).
#
#   bash /home/hal/apostille/scripts/ops-hcch-seed.sh
#
# Scope is hcch.json ONLY (a dir-scoped copy): the other packs'
# authorities/tasks stay unseeded until the author calls the full
# seed. The fetch URI is derived from the loaded source row, never
# hardcoded. Paste the whole output back to the agent.
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }

XTASK=/srv/apostille/current/xtask
STORE=/var/lib/apostille
DB=$STORE/handbook.sqlite
PACK=/srv/apostille/current/packs/hcch.json
SEEDDIR=/tmp/hcch-pack-seed
SVC="sudo -u andrii-ai"

[ -x "$XTASK" ] || { echo "ABORT: no live release xtask at $XTASK"; exit 1; }
[ -f "$DB" ] || { echo "ABORT: no prod DB at $DB"; exit 1; }
[ -f "$PACK" ] || { echo "ABORT: no hcch pack at $PACK (deploy first)"; exit 1; }

echo "=== 0. prior state (read-only)"
PRIOR="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM source s JOIN authority a ON a.id = s.authority_id WHERE a.slug = 'hcch' AND s.kind IN ('website_official','website_thirdparty');")"
echo "hcch website sources before: $PRIOR"

echo "=== 1. packs-load (hcch.json only, needs_review rows only)"
rm -rf "$SEEDDIR"
mkdir -p "$SEEDDIR" || { echo "ABORT: cannot create $SEEDDIR"; exit 1; }
cp "$PACK" "$SEEDDIR/hcch.json" || { echo "ABORT: cannot stage hcch.json"; exit 1; }
chmod 0644 "$SEEDDIR/hcch.json"
$SVC "$XTASK" packs-load --packs "$SEEDDIR" --store "$STORE" --actor hcch-seed \
    || { echo "ABORT: packs-load failed"; exit 1; }

echo "=== 2. derive the ONE uri from the loaded source row"
URI="$($SVC sqlite3 "$DB" "SELECT uri FROM source s JOIN authority a ON a.id = s.authority_id WHERE a.slug = 'hcch' AND s.kind = 'website_official' ORDER BY s.id LIMIT 1;")"
case "$URI" in
    https://*) echo "seed uri: $URI" ;;
    *) echo "ABORT: no hcch website_official source row after packs-load"; exit 1 ;;
esac

echo "=== 3. fetch-url (staff capture from prod IPs)"
$SVC "$XTASK" fetch-url --store "$STORE" --slug hcch --url "$URI" \
    --source-kind website_official --trust official --actor hcch-seed \
    || { echo "ABORT: fetch-url failed (network or URI — output above)"; exit 1; }

echo "=== 4. verify: capture row + hcch-poll unblocked"
$SVC sqlite3 "$DB" "SELECT c.id, c.kind, c.bytes_stored, c.trust FROM capture c JOIN authority a ON a.id = c.authority_id WHERE a.slug = 'hcch' ORDER BY c.id DESC LIMIT 3;" \
    || { echo "ABORT: capture probe failed"; exit 1; }
$SVC "$XTASK" hcch-poll --store "$STORE" --actor hcch-seed \
    || { echo "ABORT: hcch-poll still failing after the seed"; exit 1; }

echo "=== 5. cleanup + counts after"
rm -rf "$SEEDDIR"
echo "counts after: authority=$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM authority;") fact=$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM fact;") capture=$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM capture;")"
ROOT_OWNED="$(find "$STORE" -user root -print || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
echo "DONE hcch seed — paste this whole output back to the agent"
