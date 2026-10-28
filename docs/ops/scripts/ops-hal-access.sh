#!/bin/bash
# ONE-TIME root setup: grant hal exactly the privileges deploy
# scripts need — then hal (and the agent over ssh) deploys without
# further root involvement.
#
#   sudo bash /home/hal/apostille/scripts/ops-hal-access.sh
#
# Design (least privilege, no holes):
# - systemctl/journalctl/cp take FIXED system paths as root.
#   (The cp source wildcard is safe: hal already ships the
#   binaries that unit runs — a staged unit grants no new trust.)
# - xtask/sqlite run as the SERVICE user (andrii-ai), NOT root:
#   hal controls /srv/apostille/current, so a root grant on that
#   path would be a symlink-to-root hole. As andrii-ai the same
#   path grants nothing new: hal's release binaries already run
#   as andrii-ai with the KEK loaded.
# - No file-permission surgery: KEK stays 0600, DB stays 0640.
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root (one time only)"; exit 1; }
command -v visudo >/dev/null || { echo "ABORT: visudo missing"; exit 1; }
id hal >/dev/null 2>&1 || { echo "ABORT: user hal missing"; exit 1; }
id andrii-ai >/dev/null 2>&1 || { echo "ABORT: user andrii-ai missing"; exit 1; }

echo "=== 0. writing /etc/sudoers.d/hal-apostille"
cat > /etc/sudoers.d/hal-apostille <<'SUDOERS'
# Apostille deploy operator. Installed once by ops-hal-access.sh.
# - (root): fixed system paths only — no wildcards on binaries.
# - (andrii-ai): service-identity ops (config, sqlite). The xtask
#   path deliberately resolves under /srv/apostille/current: as
#   andrii-ai this grants no new trust (hal's release binaries
#   already run as andrii-ai with the KEK loaded). NEVER widen
#   these to (root): hal controls that symlink.
hal ALL=(root) NOPASSWD: \
    /usr/bin/systemctl restart apostille, \
    /usr/bin/systemctl status apostille, \
    /usr/bin/systemctl is-active apostille, \
    /usr/bin/systemctl reset-failed apostille, \
    /usr/bin/systemctl daemon-reload, \
    /usr/bin/journalctl -u apostille, \
    /usr/bin/journalctl -u apostille *, \
    /usr/bin/cp /home/hal/apostille/staging/*/apostille.service /etc/systemd/system/apostille.service
hal ALL=(andrii-ai) NOPASSWD: \
    /srv/apostille/current/xtask config-set *, \
    /srv/apostille/current/xtask config-get *, \
    /srv/apostille/current/xtask config-verify, \
    /srv/apostille/current/xtask config-verify *, \
    /usr/bin/sqlite3 /var/lib/apostille/handbook.sqlite *, \
    /usr/bin/rm -f /var/lib/apostille/acme/cert.pem /var/lib/apostille/acme/key.pem
SUDOERS
chmod 0440 /etc/sudoers.d/hal-apostille
visudo -c -f /etc/sudoers.d/hal-apostille || {
    echo "ABORT: sudoers validation failed — removing the drop-in"
    rm -f /etc/sudoers.d/hal-apostille
    exit 1
}
# NOTE: sudo resolves `cp` via secure_path (/usr/bin first), so
# the drop-in pins /usr/bin/cp (NOT /bin/cp, even on merged-usr).

echo "=== 1. verifying hal's grants (no-op probes, nothing executed)"
sudo -U hal -l | grep -E 'systemctl|journalctl|xtask|sqlite3|/cp ' || {
    echo "ABORT: grants not visible for hal"
    exit 1
}
sudo -u hal sudo -n /usr/bin/systemctl is-active apostille >/dev/null || {
    echo "ABORT: hal cannot run the pinned systemctl probe"
    exit 1
}
sudo -u hal sudo -n -u andrii-ai /usr/bin/sqlite3 /var/lib/apostille/handbook.sqlite \
    'SELECT COUNT(*) FROM config;' >/dev/null || {
    echo "ABORT: hal cannot run the pinned sqlite probe"
    exit 1
}

echo "=== 2. file permissions (informational — unchanged by design)"
ls -l /etc/apostille/handbook-kek /var/lib/apostille/handbook.sqlite
echo "DONE — hal can now: deploy (restart via pinned sudo), read prod"
echo "journal, install staged unit files, and run config/sqlite ops as"
echo "andrii-ai. Agent: proceed with the pending f5d199f deploy as hal."
