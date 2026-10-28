#!/bin/bash
# Add www.andrii.ai to the ACME certificate (3 SANs total) and
# force an immediate reissue. Runs as hal (pinned grants +
# wrappers from ops-hal-access.sh) or root.
#
#   bash /home/hal/apostille/scripts/ops-acme-add-www.sh
#
# Zero placeholders: sets the canonical 3-name set, deletes the
# persisted pair (keeping account.json — EAB secrets are
# single-use), restarts, and verifies all 3 SANs + www serving.
# Paste the whole output back to the agent.
set -u

SUDO="sudo -n"
SVC="sudo -n -u andrii-ai"
ACME_DIR=/var/lib/apostille/acme
DOMAINS="andrii.ai,admin.andrii.ai,www.andrii.ai"
command -v curl >/dev/null || { echo "ABORT: curl missing"; exit 1; }
command -v openssl >/dev/null || { echo "ABORT: openssl missing"; exit 1; }

echo "=== 0. guards: fixed binary + ACME mode + www DNS"
grep -a -q 'port-80 redirector already serving' /srv/apostille/current/web 2>/dev/null || {
    echo "ABORT: live binary lacks the :80-first fix — deploy 69480d3+ first"
    exit 1
}
[ -f "$ACME_DIR/account.json" ] || {
    echo "ABORT: $ACME_DIR/account.json missing — run ops-acme-enable.sh first"
    exit 1
}
getent hosts www.andrii.ai >/dev/null || {
    echo "ABORT: www.andrii.ai does not resolve here — fix DNS first (http-01 must reach us)"
    exit 1
}
echo "guards ok"

echo "=== 1. set 3-name domains"
# Config via the pinned wrapper (sudoers has no wildcards): stdin
# protocol, executed as the service user so the KEK + DB open
# without weakening file permissions.
printf 'set\nacme.domains\ntext\n%s\n' "$DOMAINS" | $SVC /usr/local/sbin/apostille-config \
    || { echo "ABORT: config-set failed"; exit 1; }
printf 'get\nacme.domains\ntext\n' | $SVC /usr/local/sbin/apostille-config \
    || { echo "ABORT: config-get failed"; exit 1; }

echo "=== 2. drop the persisted pair (account.json stays!) + restart"
# Fixed paths — matches the sudoers pin literally (no wildcards).
$SVC /usr/bin/rm -f "$ACME_DIR/cert.pem" "$ACME_DIR/key.pem" || { echo "ABORT: rm failed"; exit 1; }
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

echo "=== 3. verify 3 SANs + www serving"
echo | openssl s_client -connect 127.0.0.1:443 -servername www.andrii.ai 2>/dev/null \
    | openssl x509 -noout -ext subjectAltName 2>/dev/null
echo "www healthz: $(curl -s -m 10 -o /dev/null -w '%{http_code}' https://www.andrii.ai/apostille/healthz)"
echo "admin page:  $(curl -s -m 10 -o /dev/null -w '%{http_code}' https://admin.andrii.ai/admin)"
echo '-5 min' | $SUDO /usr/local/sbin/apostille-journal \
    | grep -aiE 'acme|order|challenge|issu|renew|refusing|listening' | tail -8
echo "DONE — paste this whole output back to the agent"
