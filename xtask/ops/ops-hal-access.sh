#!/bin/bash
# ONE-TIME root setup: grant hal exactly the privileges deploy
# scripts need — then hal (and the agent over ssh) deploys without
# further root involvement.
#
#   bash /home/hal/apostille/scripts/ops-hal-access.sh
#
# Design (least privilege, no holes):
# - This box's sudo rejects ALL wildcards in command arguments,
#   so every sudoers entry is a FIXED path with FIXED (or no)
#   args. Variable inputs travel via stdin or a pointer file into
#   root-owned validating wrappers (hal cannot modify
#   /usr/local/sbin).
# - xtask/sqlite run as the SERVICE user (andrii-ai), NOT root:
#   hal controls /srv/apostille/current, so a root grant on that
#   path would be a symlink-to-root hole. As andrii-ai it grants
#   no new trust: hal's release binaries already run as
#   andrii-ai with the KEK loaded.
# - No file-permission surgery on secrets: KEK stays 0600, the DB
#   stays 0640; only /srv/apostille + releases/ are group-shared.
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root (one time only)"; exit 1; }
command -v visudo >/dev/null || { echo "ABORT: visudo missing"; exit 1; }
id hal >/dev/null 2>&1 || { echo "ABORT: user hal missing"; exit 1; }
id andrii-ai >/dev/null 2>&1 || { echo "ABORT: user andrii-ai missing"; exit 1; }

SBIN=/usr/local/sbin
[ -d "$SBIN" ] || { echo "ABORT: $SBIN missing"; exit 1; }

echo "=== 0. group + directory bootstrap (idempotent; was runbook §0)"
groupadd -f apostille
usermod -aG apostille hal
install -d -o root -g apostille -m 775 /srv/apostille/releases
# The deploy swap writes current.tmp + renames INSIDE the root, so
# the root itself must be group-writable too. (Fresh ssh sessions
# pick up the group; sudo re-reads groups on every invocation.)
chgrp apostille /srv/apostille
chmod g+w /srv/apostille

echo "=== 1. installing root-owned wrappers (hal cannot modify these)"
# Wrapper 1: install a staged unit file. Source comes from hal's
# pointer file, but CONTENT is constrained: the unit may only
# ever run hal's release binaries as andrii-ai (today's trust).
cat > "$SBIN/apostille-install-unit" <<'WRAPPER'
#!/bin/bash
# Install the staged unit named by hal's pointer file. No args
# (sudoers has no wildcards on this box). Content-validated: may
# only run /srv/apostille/current/web as andrii-ai.
set -u
PTR=/home/hal/apostille/.pending-unit
SRC="$(cat "$PTR" 2>/dev/null || true)"
case "$SRC" in
    /home/hal/apostille/staging/*) ;;
    *) echo "ABORT: bad pointer" >&2; exit 1 ;;
esac
case "$SRC" in
    *[!a-zA-Z0-9/._-]*) echo "ABORT: bad pointer chars" >&2; exit 1 ;;
esac
[ -f "$SRC" ] && [ -s "$SRC" ] || { echo "ABORT: no staged unit" >&2; exit 1; }
grep -qx 'User=andrii-ai' "$SRC" || { echo "ABORT: unit must run as andrii-ai" >&2; exit 1; }
grep -qx 'Group=andrii-ai' "$SRC" || { echo "ABORT: unit must run as andrii-ai" >&2; exit 1; }
grep -qx 'ExecStart=/srv/apostille/current/web' "$SRC" || { echo "ABORT: unexpected ExecStart" >&2; exit 1; }
grep -q '^User=root\|^Group=root' "$SRC" && { echo "ABORT: root unit refused" >&2; exit 1; }
diff -u /etc/systemd/system/apostille.service "$SRC" || true
/usr/bin/cp "$SRC" /etc/systemd/system/apostille.service
/usr/bin/systemctl daemon-reload
grep -H StartLimit /etc/systemd/system/apostille.service || echo "(no StartLimit lines)"
rm -f "$PTR"
WRAPPER
# Wrapper 2: bounded journal tail. Window comes on stdin, strictly
# validated; unit is fixed so output cannot escape to other units.
cat > "$SBIN/apostille-journal" <<'WRAPPER'
#!/bin/bash
# Usage: echo '-5 min' | apostille-journal. Empty stdin = last 50 lines.
set -u
IFS= read -r WINDOW || WINDOW=""
if [ -z "$WINDOW" ]; then
    exec /usr/bin/journalctl -u apostille --no-pager -n 50
fi
case "$WINDOW" in
    -[0-9]*\ min|-[0-9]*\ h|-[0-9]*\ d) ;;
    *) echo "ABORT: window must look like '-5 min'" >&2; exit 1 ;;
esac
exec /usr/bin/journalctl -u apostille --no-pager --since "$WINDOW"
WRAPPER
# Wrapper 3: sealed-config ops as the service user. Protocol on
# stdin (4 lines: op, key, type, value-to-EOF; type/value ignored
# for get, key/type ignored for verify). Runs hal-controlled
# xtask as andrii-ai — no new trust (hal's release binaries
# already run as andrii-ai).
cat > "$SBIN/apostille-config" <<'WRAPPER'
#!/bin/bash
# Usage: printf 'set\nacme.enabled\nbool\ntrue\n' | apostille-config
set -u
STORE="--store /var/lib/apostille --kek-file /etc/apostille/handbook-kek"
IFS= read -r OP || exit 1
IFS= read -r KEY || KEY=""
IFS= read -r TYPE || TYPE=""
VALUE="$(cat)"
case "$OP" in set|get|verify) ;; *) echo "ABORT: op must be set|get|verify" >&2; exit 1 ;; esac
if [ "$OP" != verify ]; then
    case "$KEY" in
        ""|*[!a-z0-9._-]*) echo "ABORT: bad key" >&2; exit 1 ;;
    esac
fi
if [ "$OP" = set ] && [ "$TYPE" != bool ] && [ "$TYPE" != text ] && [ "$TYPE" != secret ]; then
    echo "ABORT: type must be bool|text|secret" >&2; exit 1
fi
# shellcheck disable=SC2086 (STORE is fixed words, never user input)
case "$OP" in
    set) printf '%s' "$VALUE" | exec /srv/apostille/current/xtask config-set $STORE --key "$KEY" --type "$TYPE" --value-from-stdin ;;
    get) exec /srv/apostille/current/xtask config-get $STORE --key "$KEY" ;;
    verify) exec /srv/apostille/current/xtask config-verify $STORE ;;
esac
WRAPPER
# Wrapper 4: sqlite on the fixed DB path (SQL on stdin, passed
# through). Full SQL power as andrii-ai = intended operator
# power; secrets stay sealed (KEK never leaves root/andrii-ai).
cat > "$SBIN/apostille-sqlite" <<'WRAPPER'
#!/bin/bash
# Usage: echo "SELECT ..." | apostille-sqlite
exec /usr/bin/sqlite3 /var/lib/apostille/handbook.sqlite
WRAPPER
# Wrapper 5: staff client-cert lifecycle as the service user.
# Protocol on stdin (3 lines: op, arg1, arg2):
#   issue:  cn, days(1..825)   → prints paths + fenced PEM delivery blocks
#   verify: fp64, expect-cn     → exit 0/1
#   revoke: cn-or-fp64, (ignored)
# Trust note: hal already holds full sqlite-as-andrii-ai (can hand-make
# these rows) plus release deploy (arbitrary code as andrii-ai), so these
# validating wrappers grant NO new power — they make enrolment safe and
# root-free instead of hand-SQL. Run hal-controlled xtask as andrii-ai.
cat > "$SBIN/apostille-cert" <<'WRAPPER'
#!/bin/bash
# Usage: printf 'issue\njdoe\n365\n' | apostille-cert
set -u
STORE=/var/lib/apostille
CERTS=$STORE/admin-certs
IFS= read -r OP || exit 1
IFS= read -r A1 || A1=""
IFS= read -r A2 || A2=""
case "$OP" in issue|verify|revoke) ;; *) echo "ABORT: op must be issue|verify|revoke" >&2; exit 1 ;; esac
is_cn() { case "$1" in ""|*[!A-Za-z0-9._-]*) return 1 ;; esac; [ "${#1}" -le 64 ]; }
is_fp() { [ "${#1}" -eq 64 ] && case "$1" in *[!0-9a-fA-F]*) return 1 ;; *) return 0 ;; esac; }
is_days() { case "$1" in ""|*[!0-9]*) return 1 ;; esac; [ "$1" -ge 1 ] && [ "$1" -le "$2" ]; }
XTASK="/srv/apostille/current/xtask"
case "$OP" in
issue)
    is_cn "$A1" || { echo "ABORT: bad cn" >&2; exit 1; }
    is_days "$A2" 825 || { echo "ABORT: days must be 1..825" >&2; exit 1; }
    OUT="$("$XTASK" admin-cert-issue --store $STORE --cn "$A1" --actor hal --days "$A2")" || { echo "$OUT" >&2; exit 1; }
    echo "$OUT"
    CERT="$(echo "$OUT" | sed -n 's/^  cert: //p')"
    KEY="$(echo "$OUT" | sed -n 's/^  key: //p')"
    # Only cat paths matching the strict issued-file shape (never raw output).
    case "$CERT" in "$CERTS"/client-"$A1"-*.pem) ;; *) echo "ABORT: unexpected cert path" >&2; exit 1 ;; esac
    case "$KEY" in "$CERTS"/client-"$A1"-*.key.pem) ;; *) echo "ABORT: unexpected key path" >&2; exit 1 ;; esac
    echo "--- BEGIN DELIVERY cert ---"; cat "$CERT"; echo "--- END DELIVERY ---"
    echo "--- BEGIN DELIVERY key ---"; cat "$KEY"; echo "--- END DELIVERY ---"
    ;;
verify)
    is_fp "$A1" || { echo "ABORT: fp must be 64 hex" >&2; exit 1; }
    is_cn "$A2" || { echo "ABORT: bad expect-cn" >&2; exit 1; }
    exec "$XTASK" admin-cert-verify --store $STORE --fp "$A1" --expect-cn "$A2"
    ;;
revoke)
    if is_fp "$A1"; then
        exec "$XTASK" admin-cert-revoke --store $STORE --fingerprint "$A1" --actor hal
    elif is_cn "$A1"; then
        exec "$XTASK" admin-cert-revoke --store $STORE --cn "$A1" --actor hal
    else
        echo "ABORT: revoke needs a cn or 64-hex fp" >&2; exit 1
    fi
    ;;
esac
WRAPPER
# Wrapper 6: staff enrolment invites as the service user. Protocol on
# stdin (4 lines: role, cn, days, origin-or-empty). Prints the
# single-use URL ONCE (token hashed at rest, never retrievable).
cat > "$SBIN/apostille-invite" <<'WRAPPER'
#!/bin/bash
# Usage: printf 'staff\njdoe\n7\n\n' | apostille-invite
set -u
STORE=/var/lib/apostille
IFS= read -r ROLE || exit 1
IFS= read -r CN || CN=""
IFS= read -r DAYS || DAYS=""
IFS= read -r ORIGIN || ORIGIN=""
case "$ROLE" in staff|admin) ;; *) echo "ABORT: role must be staff|admin" >&2; exit 1 ;; esac
case "$CN" in ""|*[!A-Za-z0-9._-]*) echo "ABORT: bad cn" >&2; exit 1 ;; esac
[ "${#CN}" -le 64 ] || { echo "ABORT: bad cn" >&2; exit 1; }
case "$DAYS" in ""|*[!0-9]*) echo "ABORT: days must be 1..30" >&2; exit 1 ;; esac
{ [ "$DAYS" -ge 1 ] && [ "$DAYS" -le 30 ]; } || { echo "ABORT: days must be 1..30" >&2; exit 1; }
case "$ORIGIN" in ""|https://*|http://localhost*) ;; *) echo "ABORT: bad origin" >&2; exit 1 ;; esac
ARGS="--store $STORE --role $ROLE --cn $CN --actor hal --days $DAYS"
[ -n "$ORIGIN" ] && ARGS="$ARGS --origin $ORIGIN"
# shellcheck disable=SC2086 (ARGS is fixed words + validated values)
exec /srv/apostille/current/xtask admin-invite $ARGS
WRAPPER
chmod 0755 "$SBIN"/apostille-install-unit "$SBIN"/apostille-journal \
    "$SBIN"/apostille-config "$SBIN"/apostille-sqlite \
    "$SBIN"/apostille-cert "$SBIN"/apostille-invite
chown root:root "$SBIN"/apostille-install-unit "$SBIN"/apostille-journal \
    "$SBIN"/apostille-config "$SBIN"/apostille-sqlite \
    "$SBIN"/apostille-cert "$SBIN"/apostille-invite
for w in apostille-install-unit apostille-journal apostille-config apostille-sqlite apostille-cert apostille-invite; do
    bash -n "$SBIN/$w" || { echo "ABORT: $w syntax"; exit 1; }
done

echo "=== 2. writing /etc/sudoers.d/hal-apostille (zero wildcards)"
cat > /etc/sudoers.d/hal-apostille <<'SUDOERS'
# Apostille deploy operator. Installed once by ops-hal-access.sh.
# NO wildcards anywhere: this sudo build rejects them in command
# arguments, so variable inputs travel via stdin/pointer files
# into the fixed-path wrappers above (validated inside).
# - (root): fixed systemctl/journal/unit-install paths.
# - (andrii-ai): config/sqlite/pair-rm. xtask runs as the SERVICE
#   user deliberately (hal controls /srv/apostille/current; as
#   root that path would be a symlink-to-root hole — as
#   andrii-ai it grants no new trust). NEVER widen to (root).
hal ALL=(root) NOPASSWD: \
    /usr/bin/systemctl restart apostille, \
    /usr/bin/systemctl status apostille, \
    /usr/bin/systemctl is-active apostille, \
    /usr/bin/systemctl reset-failed apostille, \
    /usr/bin/systemctl daemon-reload, \
    /usr/local/sbin/apostille-install-unit, \
    /usr/local/sbin/apostille-journal
hal ALL=(andrii-ai) NOPASSWD: \
    /usr/local/sbin/apostille-config, \
    /usr/local/sbin/apostille-sqlite, \
    /usr/local/sbin/apostille-cert, \
    /usr/local/sbin/apostille-invite, \
    /usr/bin/rm -f /var/lib/apostille/acme/cert.pem /var/lib/apostille/acme/key.pem
SUDOERS
chmod 0440 /etc/sudoers.d/hal-apostille
visudo -c -f /etc/sudoers.d/hal-apostille || {
    echo "ABORT: sudoers validation failed — removing the drop-in"
    rm -f /etc/sudoers.d/hal-apostille
    exit 1
}
visudo -c || { echo "ABORT: full sudoers no longer valid"; exit 1; }

echo "=== 3. retiring the legacy restart/status drop-in (it is subsumed)"
if [ -f /etc/sudoers.d/apostille-hal ]; then
    echo "--- legacy contents (backed up to /root/apostille-hal.sudoers.bak):"
    cat /etc/sudoers.d/apostille-hal
    cp /etc/sudoers.d/apostille-hal /root/apostille-hal.sudoers.bak
    rm -f /etc/sudoers.d/apostille-hal
    visudo -c || { echo "ABORT: full sudoers invalid after retirement"; exit 1; }
else
    echo "(no legacy drop-in present)"
fi

echo "=== 4. verifying hal's grants (read-only probes)"
sudo -U hal -l | grep -E 'systemctl|apostille-' || {
    echo "ABORT: grants not visible for hal"
    exit 1
}
if sudo -U hal -l 2>/dev/null | grep -q 'apostille-ship'; then
    echo "(note: legacy apostille-ship grant still present — its wildcard"
    echo " form cannot validate on this sudo; flag it to the agent)"
fi
sudo -u hal touch /srv/apostille/releases/.w \
    && sudo -u hal rm /srv/apostille/releases/.w && echo WRITE-OK \
    || { echo "ABORT: hal cannot write releases/"; exit 1; }
sudo -u hal touch /srv/apostille/.w \
    && sudo -u hal rm /srv/apostille/.w && echo FLIP-OK \
    || { echo "ABORT: hal cannot write the swap dir"; exit 1; }
sudo -u hal sudo -n /usr/bin/systemctl is-active apostille >/dev/null \
    && echo SYSTEMCTL-OK \
    || { echo "ABORT: hal cannot run the pinned systemctl probe"; exit 1; }
echo 'SELECT 1;' | sudo -u hal sudo -n -u andrii-ai /usr/local/sbin/apostille-sqlite >/dev/null \
    && echo SQL-OK \
    || { echo "ABORT: hal cannot run the pinned sqlite probe"; exit 1; }
printf '\n' | sudo -u hal sudo -n /usr/local/sbin/apostille-journal >/dev/null \
    && echo JOURNAL-OK \
    || { echo "ABORT: hal cannot run the pinned journal probe"; exit 1; }
printf 'get\nacme.enabled\ntext\n' \
    | sudo -u hal sudo -n -u andrii-ai /usr/local/sbin/apostille-config >/dev/null \
    && echo CONFIG-OK \
    || echo "(note: config probe needs a live release at /srv/apostille/current)"
# Cert wrapper: end-to-end read-only probe (verify of a dummy fp runs
# the full chain — sudo grant + xtask exec — and mutates nothing).
printf 'verify\n0000000000000000000000000000000000000000000000000000000000000000\nprobe-noexist\n' \
    | sudo -u hal sudo -n -u andrii-ai /usr/local/sbin/apostille-cert >/dev/null 2>&1;
[ $? -ne 0 ] && echo CERT-OK \
    || echo "(note: cert probe unexpectedly passed — inspect)"
# Invite wrapper: rejection probe (bad role dies in validation before
# any exec — proves the wrapper + grant without minting an invite).
printf 'bogus\n\n\n\n' \
    | sudo -u hal sudo -n -u andrii-ai /usr/local/sbin/apostille-invite >/dev/null 2>&1;
[ $? -ne 0 ] && echo INVITE-OK \
    || { echo "ABORT: invite wrapper accepted a bad role"; exit 1; }
echo "(note: install-unit is NOT probed — it installs for real;"
echo " the next deploy exercises it)"

echo "=== 5. secret file permissions (informational — unchanged by design)"
ls -l /etc/apostille/handbook-kek /var/lib/apostille/handbook.sqlite
echo "DONE — hal can now deploy, read the prod journal, install"
echo "validated unit files, and run config/sqlite/cert/invite ops."
echo "Re-running this script is idempotent (wrappers + drop-in are"
echo "overwritten, then re-verified). Paste this whole output back"
echo "to the agent."
