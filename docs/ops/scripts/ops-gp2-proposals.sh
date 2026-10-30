#!/bin/bash
# GP2-data proposals: seed vietnam + usa-wa-sos authorities (if missing),
# fetch their watch URLs from prod IPs, draft proposed facts with the live
# Meta model, and print the counts to approve in /review.
# Run as root; re-running is safe (packs-load is exists-safe, fetch-slug
# dedups, analyze-scan advances a watermark and skips seen captures).
#
#   bash /home/hal/apostille/scripts/ops-gp2-proposals.sh
#
# After this script: open /review and accept the proposed facts for
# vietnam and usa-wa-sos (this accept step is also the demo). GP2-data
# passes when each slug has >= 8 accepted facts.
# Paste the whole output back to the agent.
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }
command -v sqlite3 >/dev/null || { echo "ABORT: sqlite3 missing"; exit 1; }

XTASK=/srv/apostille/current/xtask
STORE=/var/lib/apostille
DB="$STORE/handbook.sqlite"
PACKS=/srv/apostille/current/packs
KEK_SRC=/etc/apostille/handbook-kek
CREDS=$STORE/.loop-creds
STAGED=$CREDS/handbook-kek
SEEDDIR=/tmp/gp2-pack-seed
SVC="sudo -u andrii-ai"

[ -x "$XTASK" ] || { echo "ABORT: no live release xtask at $XTASK"; exit 1; }
[ -f "$DB" ] || { echo "ABORT: no prod DB at $DB"; exit 1; }
[ -f "$KEK_SRC" ] || { echo "ABORT: no KEK source at $KEK_SRC (analyze --live needs it)"; exit 1; }
[ "$(stat -c %s "$KEK_SRC")" = 32 ] || { echo "ABORT: KEK source is not 32 bytes"; exit 1; }

counts() {
    for slug in vietnam usa-wa-sos; do
        aid="$($SVC sqlite3 "$DB" "SELECT id FROM authority WHERE slug='$slug';")"
        if [ -z "$aid" ]; then
            printf '%s=(no authority) ' "$slug"
        else
            printf '%s=proposed:%s/accepted:%s ' "$slug" \
                "$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM fact WHERE authority_id=$aid AND status='proposed';")" \
                "$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM fact WHERE authority_id=$aid AND status='accepted';")"
        fi
    done
}

echo "=== 0. prior state (read-only)"
echo "before: $(counts)"

echo "=== 1. packs-load (vietnam-routing + usa-wa only; skipped when seeded)"
VN="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM authority WHERE slug='vietnam';")"
WA="$($SVC sqlite3 "$DB" "SELECT COUNT(*) FROM authority WHERE slug='usa-wa-sos';")"
if [ "$VN" = 1 ] && [ "$WA" = 1 ]; then
    echo "(both authorities present — packs-load skipped)"
else
    [ -f "$PACKS/vietnam-routing.json" ] || { echo "ABORT: no vietnam-routing pack at $PACKS (deploy first)"; exit 1; }
    [ -f "$PACKS/usa-wa.json" ] || { echo "ABORT: no usa-wa pack at $PACKS (deploy first)"; exit 1; }
    rm -rf "$SEEDDIR"
    mkdir -p "$SEEDDIR" || { echo "ABORT: cannot create $SEEDDIR"; exit 1; }
    cp "$PACKS/vietnam-routing.json" "$PACKS/usa-wa.json" "$SEEDDIR/" \
        || { echo "ABORT: cannot stage GP2 packs"; exit 1; }
    chmod 0644 "$SEEDDIR"/*.json
    $SVC "$XTASK" packs-load --packs "$SEEDDIR" --store "$STORE" --actor gp2-proposals \
        || { echo "ABORT: packs-load failed (output above)"; exit 1; }
    rm -rf "$SEEDDIR"
fi

echo "=== 2. fetch-slug from prod IPs (one bad URL never aborts the run)"
for slug in vietnam usa-wa-sos; do
    echo "--- fetch-slug $slug"
    $SVC "$XTASK" fetch-slug --store "$STORE" --slug "$slug" --actor gp2-proposals \
        || echo "WARNING: fetch-slug $slug exited nonzero (output above) — continuing"
done

echo "=== 3. stage andrii-ai-readable KEK copy (deleted after the scans)"
install -d -o andrii-ai -g andrii-ai -m 0700 "$CREDS"
cp "$KEK_SRC" "$STAGED"
chown andrii-ai:andrii-ai "$STAGED"
chmod 0600 "$STAGED"
[ "$(stat -c %s "$STAGED")" = 32 ] || { echo "ABORT: staged KEK is not 32 bytes"; exit 1; }
echo "staged: $STAGED (andrii-ai 0600)"

echo "=== 4. analyze --scan-captures --live per slug (drafts proposed facts)"
for slug in vietnam usa-wa-sos; do
    echo "--- analyze $slug"
    $SVC env CREDENTIALS_DIRECTORY="$CREDS" "$XTASK" analyze --store "$STORE" \
        --scan-captures --slug "$slug" --live --actor gp2-proposals \
        || echo "WARNING: analyze $slug exited nonzero (output above) — continuing"
done
rm -f "$STAGED"
[ -e "$STAGED" ] && { echo "ABORT: staged KEK undeletable"; exit 1; }
rmdir "$CREDS" 2>/dev/null || echo "WARNING: $CREDS not empty, left in place (inspect)"
echo "staged KEK removed"

echo "=== 5. root-residue check + counts after"
ROOT_OWNED="$(find "$STORE" -user root -print || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
echo "after: $(counts)"
echo "DONE gp2 proposals — next: approve the proposed facts for vietnam and usa-wa-sos in /review, then paste this whole output back to the agent"
