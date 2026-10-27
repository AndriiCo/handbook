#!/bin/sh
# apostille-cert-deploy — DRAFT FOR AUTHOR REVIEW (prompt 016).
#
# What: certbot --deploy-hook (runs AS ROOT after every issuance or
# renewal). Installs the new fullchain/privkey where the unit expects
# them and restarts the service to load them.
# Runs with $RENEWED_LINEAGE set by certbot
# (e.g. /etc/letsencrypt/live/andrii.ai).
#
# Install (as root, once): copy to
# /usr/local/sbin/apostille-cert-deploy, root-owned 755,
# `chattr +i` recommended. Safety: fixed destination paths, no
# arguments read, lineage path validated before use.
set -eu
export LC_ALL=C PATH=/usr/sbin:/usr/bin:/sbin:/bin
umask 077

LINEAGE="${RENEWED_LINEAGE:-}"
case "$LINEAGE" in
  /etc/letsencrypt/live/*) : ;;
  *) echo "refusing unexpected lineage: $LINEAGE" >&2; exit 1;;
esac
[ -f "$LINEAGE/fullchain.pem" ] || { echo "missing fullchain" >&2; exit 1; }
[ -f "$LINEAGE/privkey.pem" ] || { echo "missing privkey" >&2; exit 1; }

DST=/var/lib/apostille/tls
install -d -o andrii-ai -g andrii-ai -m 750 "$DST"
install -o andrii-ai -g andrii-ai -m 644 "$LINEAGE/fullchain.pem" "$DST/cert.pem"
install -o andrii-ai -g andrii-ai -m 600 "$LINEAGE/privkey.pem" "$DST/key.pem"

# First issuance happens before the first ship: restart + smoke
# only when a service binary is actually installed.
if [ ! -x /srv/apostille/current/web ]; then
  logger -t apostille-cert-deploy "installed certs from $LINEAGE (no service shipped yet)"
  exit 0
fi
logger -t apostille-cert-deploy "installed certs from $LINEAGE, restarting"
systemctl restart apostille
sleep 3
curl -fsS --max-time 15 https://andrii.ai/apostille/healthz >/dev/null \
  && logger -t apostille-cert-deploy "post-renew smoke OK" \
  || { logger -t apostille-cert-deploy "post-renew smoke FAILED"; exit 1; }
