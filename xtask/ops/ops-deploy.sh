#!/bin/bash
# Deploy a staged release + install its unit file. Runs as the deploy
# user hal, never root (group perms from ops-rootless-access.sh; restart
# via its pinned sudoers grant; unit install via the root-owned wrapper).
#
#   bash /home/hal/apostille/scripts/ops-deploy.sh /home/hal/apostille/staging/<version>
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
#
# Migration notes: when the release crosses a schema migration, the
# agent stages a one-line `$STAGE/MIGRATION_NOTES` file; its content
# is forwarded as `xtask deploy --migration-notes` into the release
# manifest (and echoed below for the audit trail).
set -u

STAGE="${1:?usage: $0 <staging-dir> [--skip-accept] [--allow-today-overlap] (e.g. /home/hal/apostille/staging/69480d3)}"
SKIP_ACCEPT=""
OVERLAP=""
for flag in "$@"; do
    case "$flag" in
        --skip-accept) SKIP_ACCEPT="--skip-accept" ;;
        --allow-today-overlap) OVERLAP="--allow-today-overlap" ;;
    esac
done
[ -n "$OVERLAP" ] && echo "WARNING: day gate OVERRIDDEN by explicit flag (cap lifted until 2026-11-10; operator-ordered only after)"
# sudo -n fails fast when a grant is missing (never hangs on a
# prompt); root passes everything through.
SUDO="sudo -n"
[ "$(id -u)" != 0 ] || { echo "ABORT: run as the deploy user, not root"; exit 1; }
command -v sqlite3 >/dev/null || { echo "ABORT: sqlite3 missing"; exit 1; }
[ -d "$STAGE" ] || { echo "ABORT: no such dir: $STAGE"; exit 1; }
# Canonical absolute path: the unit wrapper requires an absolute
# staging path, and the deploy below cds into it.
STAGE="$(cd "$STAGE" && pwd)"
case "$STAGE" in
    /home/hal/apostille/staging/*) ;;
    *) echo "ABORT: staging dir must be under /home/hal/apostille/staging/"; exit 1 ;;
esac
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
# Same-version redeploy into a root-owned target fails deep inside
# xtask with a bare "Permission denied" — catch it here with the
# exact remedy instead.
TARGET="/srv/apostille/releases/$VERSION"
if [ -e "$TARGET" ] && { [ ! -d "$TARGET" ] || [ ! -w "$TARGET" ]; }; then
    echo "ABORT: $TARGET exists but is not writable by $USER (root-owned from an earlier root deploy?)"
    echo "remedy as root: rm -rf $TARGET   # then re-run this script"
    exit 1
fi

echo "=== 0. install unit file (showing diff first)"
diff -u /etc/systemd/system/apostille.service "$STAGE/apostille.service" || true
# The wrapper (root-owned, content-validated) installs + reloads;
# the pointer names the staged source (sudoers has no wildcards).
echo "$STAGE/apostille.service" > /home/hal/apostille/.pending-unit \
    || { echo "ABORT: cannot write unit pointer"; exit 1; }
$SUDO /usr/local/sbin/apostille-install-unit \
    || { echo "ABORT: unit install failed"; exit 1; }
$SUDO /usr/bin/systemctl reset-failed apostille 2>/dev/null || true
grep -H StartLimit /etc/systemd/system/apostille.service \
    || echo "(note: no StartLimit lines in installed unit)"

echo "=== 1. accept gate"
if [ -n "$SKIP_ACCEPT" ]; then
    echo "WARNING: accept gate SKIPPED by explicit flag (builder-run deploys announce it here)"
    GATE_ARGS="--skip-accept"
else
    N="$(sqlite3 /var/lib/apostille/handbook.sqlite "SELECT build_no FROM build ORDER BY build_no DESC LIMIT 1;" 2>/dev/null || true)"
    [ -n "$N" ] || {
        echo "ABORT: no build row in prod DB — refusing to --skip-accept silently; confirm with agent"
        exit 1
    }
    echo "build N=$N"
    GATE_ARGS="--expect-build $N"
fi

echo "=== 2. deploy $VERSION ($SHA)"
cd "$STAGE" || { echo "ABORT: cannot cd $STAGE"; exit 1; }
# Optional manifest notes: the agent writes one line to
# $STAGE/MIGRATION_NOTES at stage time when the release crosses a
# schema migration (rollback-safe or not — the manifest is the audit
# trail; absent file keeps the old empty-notes behavior).
ARGS=(--version "$VERSION" --sha "$SHA" --src . --store /var/lib/apostille)
# shellcheck disable=SC2086
ARGS+=($OVERLAP)
# shellcheck disable=SC2086
ARGS+=($GATE_ARGS)
if [ -f "$STAGE/MIGRATION_NOTES" ]; then
    NOTES="$(cat "$STAGE/MIGRATION_NOTES")"
    echo "migration notes: $NOTES"
    ARGS+=(--migration-notes "$NOTES")
fi
./xtask deploy "${ARGS[@]}" || {
    echo "ABORT: deploy failed (smoke failure auto-rolls-back — see output above)"
    exit 1
}

echo "=== 3. verify stamp (bounded /healthz poll, no bare sleeps)"
TRIES="${VERIFY_TRIES:-18}"
INTERVAL="${VERIFY_INTERVAL:-5}"
STAMP=""
for ((i = 1; i <= TRIES; i++)); do
    STAMP="$(curl -s -m 10 https://andrii.ai/apostille/healthz 2>/dev/null || true)"
    case "$STAMP" in
        *"\"version\":\"0.1.0+$VERSION\""*)
            echo "$STAMP"
            echo "stamp OK: serving $VERSION (try $i/$TRIES)"
            STAMP_OK=1
            break ;;
    esac
    [ "$i" -lt "$TRIES" ] && sleep "$INTERVAL"
done
[ "${STAMP_OK:-0}" = 1 ] || {
    echo "ABORT: serving stamp does not name $VERSION after $TRIES tries (${INTERVAL}s apart; stale binary or stale process?)"
    echo "last body: $STAMP"
    echo "remedy: check for orphan web procs: ps -o pid,user,lstart,cmd -C web"
    exit 1
}
echo "=== 4. releases (xtask deploy keeps the live one plus the newest 3)"
LIVE="$(readlink /srv/apostille/current)"
echo "releases now: $(ls /srv/apostille/releases/ | tr '\n' ' ')(live: $LIVE)"
echo "DONE deploy $VERSION — paste this whole output back to the agent"
