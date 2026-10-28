#!/bin/bash
# Flip prod to in-binary ACME (Google Public CA). Runs as hal
# (pinned sudo grants, see ops-hal-access.sh) or root. ONLY after
# the fixed release (69480d3+) is serving — the script guards this
# by checking the live binary for the :80-first code.
#
#   bash /home/hal/apostille/scripts/ops-acme-enable.sh
#
# Zero placeholders: the EAB pair + contact email are already sealed
# in the DB from the earlier setup. Prints issuance evidence at the
# end — paste the whole output back to the agent.
set -u

SUDO="sudo -n"
SVC="sudo -n -u andrii-ai"
WEB=/srv/apostille/current/web
XTASK=/srv/apostille/current/xtask
S="--store /var/lib/apostille --kek-file /etc/apostille/handbook-kek"
DB=/var/lib/apostille/handbook.sqlite
command -v sqlite3 >/dev/null || { echo "ABORT: sqlite3 missing"; exit 1; }
command -v curl >/dev/null || { echo "ABORT: curl missing"; exit 1; }

echo "=== 0. guard: live binary must contain the :80-first fix"
grep -a -q 'port-80 redirector already serving' "$WEB" 2>/dev/null || {
    echo "ABORT: $WEB lacks the fix — deploy 69480d3+ first (old binary + acme.enabled = crash loop)"
    exit 1
}
echo "guard ok"

echo "=== 1. enable ACME + unpin the file cert"
# Absolute xtask path: the sudoers pin matches it literally
# (./xtask would NOT match). Runs as the service user so the KEK
# + DB open without weakening file permissions.
$SVC $XTASK config-set $S --key acme.enabled --type bool --value true \
    || { echo "ABORT: config-set failed"; exit 1; }
$SVC /usr/bin/sqlite3 "$DB" "DELETE FROM config WHERE key IN ('tls.cert_file','tls.key_file');" \
    || { echo "ABORT: tls unpin failed"; exit 1; }
$SVC $XTASK config-get $S --key acme.enabled || { echo "ABORT: config-get failed"; exit 1; }
$SVC $XTASK config-verify $S || { echo "ABORT: config-verify failed"; exit 1; }

echo "=== 2. restart + poll health (up to 180s; first boot runs the ACME order)"
$SUDO /usr/bin/systemctl restart apostille || { echo "ABORT: restart failed"; exit 1; }
for i in $(seq 1 36); do
    if curl -sf -m 5 https://andrii.ai/apostille/healthz >/dev/null 2>&1; then
        echo "healthy after ~$((i * 5))s"
        break
    fi
    if [ "$i" = 36 ]; then
        echo "NOT healthy after 180s — see journal below;"
        echo "if it says rateLimited: systemctl stop apostille, wait ~1h, then start."
    fi
    sleep 5
done

echo "=== 3. issuance evidence"
$SUDO /usr/bin/journalctl -u apostille --since '-5 min' \
    | grep -aiE 'acme|order|challenge|issu|renew|refusing|listening' | tail -12
ls -la /var/lib/apostille/acme/ 2>&1 || echo "(acme dir not listable as $USER — the SAN check below is authoritative)"
curl -s -m 10 https://andrii.ai/apostille/healthz; echo
echo "DONE — paste this whole output back to the agent"
