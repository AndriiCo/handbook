#!/bin/bash
# A2 demo-precheck recording: run live SAM 3.1 + Muse Spark ONCE per
# synthetic sample, install the verdict JSONs into prod. Re-running
# overwrites the recordings (same samples, fresh boxes/verdicts).
# The public endpoint serves these files verbatim: zero marginal model
# cost, no key on the serving path.
#
# Needs META_API_KEY (the Model API key, never in the repo, never on
# a command line). With a TTY it prompts securely; otherwise export it
# in this shell only. Run as root; paste the whole output back to the agent.
#
#   bash /home/hal/apostille/scripts/ops-a2-record.sh
#
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }
if [ -z "${META_API_KEY:-}" ]; then
    [ -t 0 ] || { echo "ABORT: META_API_KEY is not set and stdin is not a TTY (export it in this shell only)"; exit 1; }
    read -rs -p "Model key: " META_API_KEY; echo
    [ -n "$META_API_KEY" ] || { echo "ABORT: empty key"; exit 1; }
fi
command -v python3 >/dev/null || { echo "ABORT: python3 missing"; exit 1; }

STORE=/var/lib/apostille
DESTDIR="$STORE/demo_precheck"
RELEASE=/srv/apostille/current
# Thumbs are served from the built SPA ($RELEASE/spa/demo); in the
# release layout `web` is the binary, so web/static/demo does not exist.
PNGDIR="${A2_PNG_DIR:-$RELEASE/spa/demo}"
SCRIPTDIR="$(dirname "$0")"
RECORDER="$SCRIPTDIR/a2-record.py"
SVC="sudo -u andrii-ai"
OUTDIR=/tmp/a2-record-out

[ -f "$RECORDER" ] || { echo "ABORT: recorder missing at $RECORDER (sync docs/ops/scripts first)"; exit 1; }
[ -d "$PNGDIR" ] || { echo "ABORT: no sample PNGs at $PNGDIR (deploy first, or set A2_PNG_DIR)"; exit 1; }
[ -n "$(ls "$PNGDIR"/syn-doc-*.png 2>/dev/null)" ] || { echo "ABORT: no syn-doc-*.png in $PNGDIR"; exit 1; }
python3 -c "import json,urllib.request" || { echo "ABORT: python3 stdlib incomplete"; exit 1; }

echo "=== 0. prior state (read-only)"
echo "recordings before: $(ls "$DESTDIR"/*.json 2>/dev/null | wc -l)"

echo "=== 1. live recording (one SAM + one Spark call per sample)"
rm -rf "$OUTDIR"
mkdir -p "$OUTDIR" || { echo "ABORT: cannot create $OUTDIR"; exit 1; }
META_API_KEY="$META_API_KEY" python3 "$RECORDER" --png-dir "$PNGDIR" --out-dir "$OUTDIR" \
    || { echo "ABORT: recording failed (output above; nothing installed)"; exit 1; }

echo "=== 2. validate recordings (contract shape for the endpoint)"
for f in "$OUTDIR"/*.json; do
    python3 - "$f" <<'EOF' || { echo "ABORT: bad recording $f"; exit 1; }
import json, sys
r = json.load(open(sys.argv[1]))
assert r["verdict"] in ("pass", "retake"), "verdict"
assert isinstance(r.get("issues"), list), "issues"
assert isinstance(r.get("boxes"), list), "boxes"
for b in r["boxes"]:
    assert 0 <= b["x"] <= 1 and 0 <= b["y"] <= 1 and b["w"] > 0 and b["h"] > 0, "box range"
assert "sam-3.1" in r.get("models", []) and any("muse-spark" in m for m in r["models"]), "models"
print(f"ok {sys.argv[1]}: {r['verdict']} {len(r['boxes'])} box(es)")
EOF
done

echo "=== 3. install as andrii-ai (0600 dir, 0644 files)"
install -d -o andrii-ai -g andrii-ai -m 0700 "$DESTDIR"
install -o andrii-ai -g andrii-ai -m 0644 "$OUTDIR"/*.json "$DESTDIR/" \
    || { echo "ABORT: install failed"; exit 1; }
rm -rf "$OUTDIR"

echo "=== 4. verify + residue"
echo "recordings after: $(ls "$DESTDIR"/*.json | wc -l)"
ls -l "$DESTDIR"
ROOT_OWNED="$(find "$STORE" -user root -print || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
echo "DONE a2 recording — paste this whole output back to the agent"
