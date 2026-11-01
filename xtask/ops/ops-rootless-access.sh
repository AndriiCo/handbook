#!/usr/bin/env bash
# One-time root step: rootless app operations (operator directive
# 2026-10-31 — all application work runs as unprivileged users; root
# stays only for true system duties). Run once as root:
#   ssh andrii.ai
#   sudo -i
#   bash ~/apostille/scripts/ops-rootless-access.sh
# Idempotent: safe to re-run. Prints evidence, ABORTs on any failure.
set -u

fail() { echo "ABORT: $1"; exit 1; }

[ "$(id -u)" = "0" ] || fail "run as root (sudo -i first)"

echo "== 1. group memberships (apostille gid 9001 carries app data)"
getent group apostille >/dev/null || fail "group apostille missing"
usermod -aG apostille andrii-ai || fail "usermod andrii-ai"
id andrii-ai | grep -q apostille || fail "andrii-ai not in apostille"
id hal | grep -q apostille || fail "hal not in apostille"
echo "members OK"

echo "== 2. data trees group-owned (setgid keeps new files in group)"
for d in /srv/apostille /var/lib/apostille; do
    [ -d "$d" ] || fail "$d missing"
    chgrp -R apostille "$d" || fail "chgrp $d"
    chmod -R g+rwX "$d" || fail "chmod $d"
    find "$d" -type d -exec chmod g+s {} + || fail "setgid $d"
done
ls -ld /srv/apostille /var/lib/apostille

echo "== 2b. stale per-run credential dirs (any owner — root can)"
# Operator scripts stage KEK copies under mktemp since P165; a fixed
# $STORE/.loop-creds left by an older run (0700, other owner) breaks
# every later run for hal. Remove it unconditionally.
rm -rf /var/lib/apostille/.loop-creds || fail "rm .loop-creds"
[ -e /var/lib/apostille/.loop-creds ] && fail ".loop-creds undeletable"
echo "stale creds gone"

echo "== 3. restart grant (pinned, no shell, no install wrapper)"
cat > /etc/sudoers.d/apostille-restart <<'EOF'
# Builder-managed (ops-rootless-access.sh). The deploy user may bounce
# the service only; everything else still needs a real root login.
hal ALL=(root) NOPASSWD: /usr/bin/systemctl restart apostille, /usr/bin/systemctl reload apostille, /usr/bin/systemctl status apostille, /usr/bin/systemctl is-active apostille
EOF
chmod 440 /etc/sudoers.d/apostille-restart
visudo -c -q || fail "sudoers invalid"
sudo -U hal -l | grep -q "systemctl restart apostille" || fail "grant missing for hal"
echo "grant OK"

echo "== 4. KEK readable to the group (app scripts stage their own copy)"
# /etc/apostille is root:andrii-ai 2770; the KEK itself was 0600, so
# only andrii-ai could read it. Group-read lets the deploy user stage
# short-lived copies without root. The bytes never leave the host.
chgrp apostille /etc/apostille/handbook-kek || fail "chgrp KEK"
chmod 0640 /etc/apostille/handbook-kek || fail "chmod KEK"
ls -l /etc/apostille/handbook-kek

echo "== 5. ACME stays application-managed"
echo "The app serves /.well-known/acme-challenge and renews via its own"
echo "acme.enabled config (no certbot, no root cron). Challenge/cert dirs"
echo "live under /var/lib/apostille, now group-writable above."
ls -ld /var/lib/apostille/acme 2>/dev/null || echo "(no acme dir yet — created on first enable)"

echo "DONE rootless access — app ops no longer need a root shell"
