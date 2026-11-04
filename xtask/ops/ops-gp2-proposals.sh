#!/bin/bash
# GP2-data proposals: seed vietnam + usa-wa-sos authorities (if missing),
# fetch their watch URLs from prod IPs, draft proposed facts with the live
# Meta model, and print the counts to approve in /review.
# Run as the deploy user (hal), never root; re-running is safe
# (packs-load is exists-safe, fetch-slug dedups, analyze-scan advances
# a watermark and skips seen captures). Needs the group perms and KEK
# group-read that `xtask provision` sets up.
#
#   bash /home/hal/apostille/scripts/ops-gp2-proposals.sh
#
# After this script: open /review and accept the proposed facts for
# vietnam and usa-wa-sos (this accept step is also the demo). GP2-data
# passes when each slug has >= 8 accepted facts.
# Paste the whole output back to the agent.
set -u

[ "$(id -u)" != 0 ] || { echo "ABORT: run as the deploy user, not root"; exit 1; }
command -v sqlite3 >/dev/null || { echo "ABORT: sqlite3 missing"; exit 1; }

XTASK=/srv/apostille/current/xtask
STORE=/var/lib/apostille
DB="$STORE/handbook.sqlite"
PACKS=/srv/apostille/current/packs
KEK_SRC=/etc/apostille/handbook-kek
CREDS="$(mktemp -d /tmp/apostille-kek.XXXXXX)"
STAGED=$CREDS/handbook-kek
cleanup() { rm -rf "$CREDS"; }
trap cleanup EXIT
SEEDDIR=/tmp/gp2-pack-seed

[ -x "$XTASK" ] || { echo "ABORT: no live release xtask at $XTASK"; exit 1; }
[ -f "$DB" ] || { echo "ABORT: no prod DB at $DB"; exit 1; }
[ -f "$KEK_SRC" ] || { echo "ABORT: no KEK source at $KEK_SRC (analyze --live needs it)"; exit 1; }
[ "$(stat -c %s "$KEK_SRC")" = 32 ] || { echo "ABORT: KEK source is not 32 bytes"; exit 1; }

counts() {
    for slug in vietnam usa-wa-sos; do
        aid="$(sqlite3 "$DB" "SELECT id FROM authority WHERE slug='$slug';")"
        if [ -z "$aid" ]; then
            printf '%s=(no authority) ' "$slug"
        else
            printf '%s=proposed:%s/accepted:%s ' "$slug" \
                "$(sqlite3 "$DB" "SELECT COUNT(*) FROM fact WHERE authority_id=$aid AND status='proposed';")" \
                "$(sqlite3 "$DB" "SELECT COUNT(*) FROM fact WHERE authority_id=$aid AND status='accepted';")"
        fi
    done
}

echo "=== 0. prior state (read-only)"
echo "before: $(counts)"

echo "=== 1. packs-load (vietnam-routing + usa-wa; always: exists-safe)"
[ -f "$PACKS/vietnam-routing.json" ] || { echo "ABORT: no vietnam-routing pack at $PACKS (deploy first)"; exit 1; }
[ -f "$PACKS/usa-wa.json" ] || { echo "ABORT: no usa-wa pack at $PACKS (deploy first)"; exit 1; }
rm -rf "$SEEDDIR"
mkdir -p "$SEEDDIR" || { echo "ABORT: cannot create $SEEDDIR"; exit 1; }
cp "$PACKS/vietnam-routing.json" "$PACKS/usa-wa.json" "$SEEDDIR/" \
    || { echo "ABORT: cannot stage GP2 packs"; exit 1; }
chmod 0644 "$SEEDDIR"/*.json
"$XTASK" packs-load --packs "$SEEDDIR" --store "$STORE" --actor gp2-proposals \
    || { echo "ABORT: packs-load failed (output above)"; exit 1; }

echo "=== 1b. retire sources whose URI left the packs"
# A pack URI change (e.g. http->https) orphans the old store row, and
# fetch-slug would keep polling it; facts approved from a stale capture
# would fail A1 for good. Scoped to the two GP2 slugs only. Assumed:
# their store sources come from these packs alone. A source that has
# captures is kept and listed: deleting it would leave capture.source_id
# dangling and strip the citation from every fact drawn from it.
command -v python3 >/dev/null || { echo "ABORT: python3 missing (retire step needs it)"; exit 1; }
python3 - "$SEEDDIR" "$DB" <<'EOF' || { echo "ABORT: retire step failed (output above)"; exit 1; }
import json, sqlite3, sys, glob
packdir, db = sys.argv[1], sys.argv[2]
wanted = {}
for path in glob.glob(packdir + '/*.json'):
    pack = json.load(open(path))
    for seed in pack.get('watch', []):
        wanted.setdefault(seed['authority'], set()).add(seed['uri'])
conn = sqlite3.connect(db)
# Write lock before the counts: no capture can land between a count of
# zero and its delete (the watch-sweep timer writes captures).
conn.execute("BEGIN IMMEDIATE")
retire, kept = [], 0
for slug, uris in wanted.items():
    aid = conn.execute("SELECT id FROM authority WHERE slug=?", (slug,)).fetchone()
    if not aid:
        continue
    rows = conn.execute("SELECT id, uri FROM source WHERE authority_id=?", (aid[0],)).fetchall()
    for sid, uri in rows:
        if uri in uris:
            continue
        n = conn.execute("SELECT COUNT(*) FROM capture WHERE source_id=?", (sid,)).fetchone()[0]
        if n:
            print(f"keep source {sid} {uri} ({n} captures)")
            kept += 1
        else:
            retire.append((sid, uri))
for sid, uri in retire:
    print(f"retire source {sid} {uri} (0 captures)")
    conn.execute("DELETE FROM source WHERE id=?", (sid,))
conn.commit()
print(f"retire: done ({len(retire)} retired, {kept} kept)")
EOF
rm -rf "$SEEDDIR"

echo "=== 2. fetch-slug from prod IPs (one bad URL never aborts the run)"
for slug in vietnam usa-wa-sos; do
    echo "--- fetch-slug $slug"
    "$XTASK" fetch-slug --store "$STORE" --slug "$slug" --actor gp2-proposals \
        || echo "WARNING: fetch-slug $slug exited nonzero (output above) — continuing"
done

echo "=== 3. stage KEK copy (mktemp 0700 dir, 0600 file, deleted after the scans)"
cp "$KEK_SRC" "$STAGED"
chmod 0600 "$STAGED"
[ "$(stat -c %s "$STAGED")" = 32 ] || { echo "ABORT: staged KEK is not 32 bytes"; exit 1; }
echo "staged: $STAGED (0600, owned by $USER)"

echo "=== 4. analyze --scan-captures --live per slug (drafts proposed facts)"
analyze_slug() {
    slug="$1"
    shift
    echo "--- analyze $slug"
    CREDENTIALS_DIRECTORY="$CREDS" "$XTASK" analyze --store "$STORE" \
        --scan-captures --slug "$slug" --live --actor gp2-proposals "$@" \
        || echo "WARNING: analyze $slug exited nonzero (output above) — continuing"
}
analyze_slug vietnam
# 24 records per draft call: the WA division contact page alone lists
# ~19 values, past the default 8 (vietnam keeps the default).
analyze_slug usa-wa-sos --max-records 24
rm -rf "$CREDS"
trap - EXIT
[ -e "$STAGED" ] && { echo "ABORT: staged KEK undeletable"; exit 1; }
echo "staged KEK removed"

echo "=== 5. root-residue check + counts after"
ROOT_OWNED="$(find "$STORE" -user root -print || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
echo "after: $(counts)"
echo "DONE gp2 proposals — next: approve the proposed facts for vietnam and usa-wa-sos in /review, then paste this whole output back to the agent"
