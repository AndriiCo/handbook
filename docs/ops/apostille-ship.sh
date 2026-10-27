#!/bin/sh
# apostille-ship — DRAFT FOR AUTHOR REVIEW (prompt 015).
#
# What: install a pre-built release (binary + SPA, shipped by the
# unprivileged `hal` user into /home/hal/apostille/releases/) as the
# locked-down `andrii-ai` service, then restart + smoke + auto-rollback.
# Runs AS ROOT via a sudoers entry (see docs/ops/deploy.md). The
# service itself NEVER runs as root: systemd drops to `andrii-ai`
# with only CAP_NET_BIND_SERVICE (see apostille.service).
#
# Why copy instead of running from hal's home: a binary writable by
# `hal` that root auto-restarts would let `hal` escalate to the
# service user. The installed copy is root-owned; `hal` can only
# stage new sources, never touch the running copy.
#
# Safety properties (verify before granting sudo):
# - `set -eu`; fixed PATH; umask 027; no eval; no $ENV reads.
# - Exactly one argument, strict ^[0-9a-f]{7,64}$ (a git short/full
#   sha). Anything else exits 2 without touching the system.
# - Every path derives from that sha under two fixed roots.
# - Symlink flip is atomic (ln -sfn); smoke failure flips back.
set -eu
export LC_ALL=C PATH=/usr/sbin:/usr/bin:/sbin:/bin
umask 027

VERSION="${1:-}"
case "$VERSION" in
  ''|*[!0-9a-f]*)
    echo "usage: $0 <git-sha>" >&2; exit 2;;
esac
# Length check without bashisms: 7..64 hex chars.
len=$(printf '%s' "$VERSION" | wc -c)
if [ "$len" -lt 7 ] || [ "$len" -gt 64 ]; then
  echo "bad VERSION: $VERSION" >&2; exit 2
fi

SRC=/home/hal/apostille/releases/$VERSION
DST=/srv/apostille/releases/$VERSION
BIN_SRC=$SRC/web
SPA_SRC=$SRC/spa
BIN_DST=$DST/web

[ -f "$BIN_SRC" ] || { echo "missing $BIN_SRC" >&2; exit 1; }
[ -d "$SPA_SRC" ] || { echo "missing $SPA_SRC" >&2; exit 1; }
# Refuse to install a non-ELF (script/zip/empty) as the service binary.
magic=$(od -An -N4 -tx1 "$BIN_SRC" | tr -d ' \n')
[ "$magic" = "7f454c46" ] || { echo "not an ELF binary: $BIN_SRC" >&2; exit 1; }

PREV=$(readlink /srv/apostille/current || true)
# Trust the old link only if it points inside our releases root at a
# hex-named dir (a hand-edited symlink must never steer a rollback).
case "$PREV" in
  /srv/apostille/releases/*) : ;;
  *) PREV="";;
esac
case "${PREV##*/}" in
  ''|*[!0-9a-f]*) PREV="";;
esac
echo "shipping $VERSION (previous: ${PREV:-none})"
logger -t apostille-ship "shipping $VERSION (previous: ${PREV:-none})"

install -d -o root -g andrii-ai -m 755 /srv/apostille/releases "$DST"
install -o root -g andrii-ai -m 755 "$BIN_SRC" "$BIN_DST"
rm -rf "$DST/spa"
cp -r "$SPA_SRC" "$DST/spa"
chown -R root:andrii-ai "$DST/spa"
find "$DST/spa" -type d -exec chmod 755 {} +
find "$DST/spa" -type f -exec chmod 644 {} +

ln -sfn "$DST" /srv/apostille/current
systemctl restart apostille
sleep 3

# Smoke the PUBLIC endpoint (also validates the TLS chain).
if curl -fsS --max-time 15 https://andrii.ai/apostille/healthz >/dev/null; then
  echo "smoke OK: $VERSION live"
  logger -t apostille-ship "smoke OK: $VERSION live"
  # Keep the last 3 releases, prune older (never the live one).
  ls -1t /srv/apostille/releases | tail -n +4 | while read -r old; do
    case "$old" in
      ''|*[!0-9a-f]*|"$VERSION") continue;;
    esac
    rm -rf "/srv/apostille/releases/$old"
  done
  exit 0
fi

echo "smoke FAILED, rolling back to ${PREV:-nothing}" >&2
logger -t apostille-ship "smoke FAILED for $VERSION, rollback to ${PREV:-none}"
if [ -n "$PREV" ] && [ -x "$PREV/web" ]; then
  ln -sfn "$PREV" /srv/apostille/current
  systemctl restart apostille
  sleep 3
  curl -fsS --max-time 15 https://andrii.ai/apostille/healthz >/dev/null \
    && echo "rollback OK" || echo "rollback SMOKE FAILED - investigate" >&2
else
  echo "no previous release - service left stopped-safe (restart manually)" >&2
  systemctl stop apostille || true
fi
exit 1
