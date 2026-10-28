#!/bin/bash
# Deploy a staged release + install its unit file. Run as root.
#
#   sudo bash ~/apostille/scripts/ops-deploy.sh ~/apostille/staging/<version>
#
# Zero placeholders: VERSION comes from the staging dir name, SHA from
# the staging/SHA file (written by the agent at stage time), and the
# accept-gate build number is read from the live DB. Every failure
# aborts with a clear message; deploy itself auto-rolls-back on a
# failed smoke. Paste the whole output back to the agent.
#
# Escape hatch (NOT silent): `--skip-accept` bypasses the handbook
# facts gate. Only for releases with no content/schema surface when
# prod has no build row to check against — the agent confirms this
# per release; the skip is always announced in the output.
set -u

STAGE="${1:?usage: $0 <staging-dir> [--skip-accept] (e.g. ~/apostille/staging/69480d3)}"
SKIP_ACCEPT=""
[ "${2:-}" = "--skip-accept" ] && SKIP_ACCEPT="--skip-accept"
[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }
[ -d "$STAGE" ] || { echo "ABORT: no such dir: $STAGE"; exit 1; }
VERSION="$(basename "$STAGE")"
SHA="$(cat "$STAGE/SHA" 2>/dev/null || true)"
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || {
    echo "ABORT: $STAGE/SHA missing or not a full 40-hex sha"
    exit 1
}
for f in web xtask apostille.service; do
    [ -f "$STAGE/$f" ] || { echo "ABORT: $STAGE/$f missing"; exit 1; }
done
for d in spa spa-admin static; do
    [ -d "$STAGE/$d" ] || { echo "ABORT: $STAGE/$d missing"; exit 1; }
done

echo "=== 0. install unit file (showing diff first)"
diff -u /etc/systemd/system/apostille.service "$STAGE/apostille.service" || true
cp "$STAGE/apostille.service" /etc/systemd/system/apostille.service \
    || { echo "ABORT: unit install failed"; exit 1; }
systemctl daemon-reload || { echo "ABORT: daemon-reload failed"; exit 1; }
systemctl reset-failed apostille 2>/dev/null || true
grep -H StartLimit /etc/systemd/system/apostille.service \
    || echo "(note: no StartLimit lines in installed unit)"

echo "=== 1. accept gate"
if [ -n "$SKIP_ACCEPT" ]; then
    echo "WARNING: accept gate SKIPPED by explicit operator flag"
    GATE_ARGS="--skip-accept"
else
    N="$(sqlite3 /var/lib/apostille/handbook.sqlite \
        "SELECT build_no FROM build ORDER BY build_no DESC LIMIT 1;" 2>/dev/null || true)"
    [ -n "$N" ] || {
        echo "ABORT: no build row in prod DB — refusing to --skip-accept silently; confirm with agent"
        exit 1
    }
    echo "build N=$N"
    GATE_ARGS="--expect-build $N"
fi

echo "=== 2. deploy $VERSION ($SHA)"
cd "$STAGE" || { echo "ABORT: cannot cd $STAGE"; exit 1; }
# shellcheck disable=SC2086
./xtask deploy --version "$VERSION" --sha "$SHA" --src . \
    --store /var/lib/apostille $GATE_ARGS || {
    echo "ABORT: deploy failed (smoke failure auto-rolls-back — see output above)"
    exit 1
}

echo "=== 3. verify stamp"
curl -s -m 10 https://andrii.ai/apostille/healthz; echo
echo "DONE deploy $VERSION — paste this whole output back to the agent"
