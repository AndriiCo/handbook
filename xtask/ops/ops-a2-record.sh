#!/bin/bash
# A2 demo-precheck recording: run live SAM 3.1 + Muse Spark ONCE per
# synthetic sample, install the verdict JSONs into prod. Re-running
# replays the cached answers (same verdicts, no spend); export
# META_CACHE_MODE=refresh for fresh boxes/verdicts.
# The public endpoint serves these files verbatim: zero marginal model
# cost, no key on the serving path.
#
# Needs META_API_KEY (the Model API key, never in the repo, never on
# a command line). With a TTY it prompts securely; otherwise export it
# in this shell only. Run as the deploy user (hal), never root; paste the
# whole output back to the agent.
#
#   bash /home/hal/apostille/scripts/ops-a2-record.sh
#
set -u

[ "$(id -u)" != 0 ] || { echo "ABORT: run as the deploy user, not root"; exit 1; }
if [ -z "${META_API_KEY:-}" ]; then
    [ -t 0 ] || { echo "ABORT: META_API_KEY is not set and stdin is not a TTY (export it in this shell only)"; exit 1; }
    read -rs -p "Model key: " META_API_KEY; echo
    [ -n "$META_API_KEY" ] || { echo "ABORT: empty key"; exit 1; }
fi

STORE=/var/lib/apostille
DESTDIR="$STORE/demo_precheck"
RELEASE=/srv/apostille/current
# Thumbs are served from the built SPA ($RELEASE/spa/demo); in the
# release layout `web` is the binary, so web/static/demo does not exist.
PNGDIR="${A2_PNG_DIR:-$RELEASE/spa/demo}"
# The recorder is `xtask a2-record` from the live release (Rust; same
# call cache and SAM parser as the app).
XTASK="$RELEASE/xtask"
OUTDIR=/tmp/a2-record-out

[ -x "$XTASK" ] || { echo "ABORT: no xtask binary at $XTASK (deploy first)"; exit 1; }
[ -d "$PNGDIR" ] || { echo "ABORT: no sample PNGs at $PNGDIR (deploy first, or set A2_PNG_DIR)"; exit 1; }
[ -n "$(ls "$PNGDIR"/syn-doc-*.png 2>/dev/null)" ] || { echo "ABORT: no syn-doc-*.png in $PNGDIR"; exit 1; }

echo "=== 0. prior state (read-only)"
echo "recordings before: $(ls "$DESTDIR"/*.json 2>/dev/null | wc -l)"

echo "=== 1. live recording (one SAM + one Spark call per sample)"
rm -rf "$OUTDIR"
mkdir -p "$OUTDIR" || { echo "ABORT: cannot create $OUTDIR"; exit 1; }
# Call cache (docs/ops/call-cache.md): a rerun replays recorded SAM and
# Spark answers instead of re-spending. hal-owned, so it never fights
# the service user over /var/lib/apostille/meta-cache permissions.
A2_CACHE="${META_CACHE_DIR:-$HOME/apostille-cache/meta-cache}"
META_API_KEY="$META_API_KEY" "$XTASK" a2-record --png-dir "$PNGDIR" --out-dir "$OUTDIR" \
    --meta-cache "$A2_CACHE" \
    || { echo "ABORT: recording failed (output above; nothing installed)"; exit 1; }

echo "=== 2. validate recordings (one per sample, a pass/retake verdict each)"
# The recorder writes only pass/retake verdicts with in-image boxes;
# this guards against a partial run reaching the endpoint.
PNGS=$(ls "$PNGDIR"/syn-doc-*.png | wc -l)
JSONS=$(ls "$OUTDIR"/*.json 2>/dev/null | wc -l)
[ "$PNGS" = "$JSONS" ] || { echo "ABORT: $JSONS recording(s) for $PNGS sample(s)"; exit 1; }
for f in "$OUTDIR"/*.json; do
    grep -Eq '"verdict": "(pass|retake)"' "$f" || { echo "ABORT: bad recording $f"; exit 1; }
    echo "ok $f"
done

echo "=== 3. install (0750 dir group-read for the app group, 0644 files)"
install -d -m 0750 "$DESTDIR"
# The app runs as its own user (prod: andrii-ai, in group apostille);
# without the group bit a 0750 hal-owned dir is EACCES -> endpoint 503.
chgrp apostille "$DESTDIR" 2>/dev/null || true
install -m 0644 "$OUTDIR"/*.json "$DESTDIR/" \
    || { echo "ABORT: install failed"; exit 1; }
rm -rf "$OUTDIR"

echo "=== 4. verify + residue"
echo "recordings after: $(ls "$DESTDIR"/*.json | wc -l)"
ls -l "$DESTDIR"
ROOT_OWNED="$(find "$STORE" -user root -print || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
echo "DONE a2 recording — paste this whole output back to the agent"
