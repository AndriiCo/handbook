#!/bin/bash
# First prod-loop run (step-3 unpark) + pinned hal grant for re-runs.
# Run as root; re-running re-executes the loop body (idempotent).
#
#   bash /home/hal/apostille/scripts/ops-loop-first-run.sh
#
# Installs /usr/local/sbin/apostille-loop (root-owned, zero args):
# ONE fixed command as andrii-ai against --store /var/lib/apostille
# with a staged KEK the wrapper deletes on exit (trap). Appends the
# (andrii-ai) sudoers grant (visudo-gated, backup kept). Preflights
# the 076 step-3 set, runs the sweep pair as andrii-ai, runs the
# fixed KEK-gated command via the hal path, then verifies the staged
# KEK is gone and no root-owned files remain. Paste the whole output
# back to the agent.
#
# Trust note: hal's release binaries already run as andrii-ai with
# the KEK loaded (ops-hal-access.sh), so a fixed loop command as
# andrii-ai grants no new trust — it unparks the first run without
# a root shell on the DB (root execution would risk root-owned
# WAL/sidecar files the service then can't write).
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }
command -v visudo >/dev/null || { echo "ABORT: visudo missing"; exit 1; }
id hal >/dev/null 2>&1 || { echo "ABORT: user hal missing"; exit 1; }
id andrii-ai >/dev/null 2>&1 || { echo "ABORT: user andrii-ai missing"; exit 1; }
command -v sqlite3 >/dev/null || { echo "ABORT: sqlite3 missing"; exit 1; }

SBIN=/usr/local/sbin
XTASK=/srv/apostille/current/xtask
STORE=/var/lib/apostille
DB=$STORE/handbook.sqlite
KEK_SRC=/etc/apostille/handbook-kek
CREDS=$STORE/.loop-creds
STAGED=$CREDS/handbook-kek
DROPIN=/etc/sudoers.d/hal-apostille

[ -x "$XTASK" ] || { echo "ABORT: no live release xtask at $XTASK"; exit 1; }
[ -f "$DB" ] || { echo "ABORT: no prod DB at $DB"; exit 1; }
[ -f "$KEK_SRC" ] || { echo "ABORT: no KEK source at $KEK_SRC"; exit 1; }
[ "$(stat -c %s "$KEK_SRC")" = 32 ] || { echo "ABORT: KEK source is not 32 bytes"; exit 1; }
[ -f "$DROPIN" ] || { echo "ABORT: $DROPIN missing (run ops-hal-access.sh first)"; exit 1; }

echo "=== 1. install /usr/local/sbin/apostille-loop (root-owned, fixed, zero args)"
cat > "$SBIN/apostille-loop" <<'WRAPPER'
#!/bin/bash
# ONE fixed loop command as andrii-ai: Meta key validity probe
# against the prod store. Zero args (sudoers pins the path with no
# wildcards). Needs the root-staged KEK copy, deleted on exit
# (trap) either way.
set -u
[ "$#" = 0 ] || { echo "ABORT: apostille-loop takes no arguments" >&2; exit 1; }
XTASK=/srv/apostille/current/xtask
STORE=/var/lib/apostille
CREDS=/var/lib/apostille/.loop-creds
STAGED=$CREDS/handbook-kek
[ -r "$STAGED" ] || { echo "ABORT: no staged KEK (root stages it per run)" >&2; exit 1; }
trap 'rm -f "$STAGED"' EXIT
export CREDENTIALS_DIRECTORY="$CREDS"
exec "$XTASK" analyze --store "$STORE" --check
WRAPPER
chmod 0755 "$SBIN/apostille-loop"
chown root:root "$SBIN/apostille-loop"
bash -n "$SBIN/apostille-loop" || { echo "ABORT: apostille-loop syntax"; exit 1; }
echo "wrapper installed"

echo "=== 2. grant hal the (andrii-ai) loop entry (append = union, idempotent)"
if grep -qx 'hal ALL=(andrii-ai) NOPASSWD: /usr/local/sbin/apostille-loop' "$DROPIN"; then
    echo "(grant already present)"
else
    cp "$DROPIN" /root/hal-apostille.sudoers.bak.loop || { echo "ABORT: cannot back up $DROPIN"; exit 1; }
    printf '%s\n' 'hal ALL=(andrii-ai) NOPASSWD: /usr/local/sbin/apostille-loop' >> "$DROPIN"
    visudo -c -f "$DROPIN" || {
        echo "ABORT: new drop-in invalid — restoring backup"
        cp /root/hal-apostille.sudoers.bak.loop "$DROPIN"
        exit 1
    }
    visudo -c || {
        echo "ABORT: full sudoers invalid — restoring backup"
        cp /root/hal-apostille.sudoers.bak.loop "$DROPIN"
        exit 1
    }
    echo "grant appended (backup: /root/hal-apostille.sudoers.bak.loop)"
fi
sudo -U hal -l | grep -q 'apostille-loop' || { echo "ABORT: grant not visible for hal"; exit 1; }
echo "grant visible"

echo "=== 3. preflight: packs + counts + sealed rows (read-only)"
if [ -d /srv/apostille/current/packs ]; then
    sudo -u andrii-ai "$XTASK" packs-validate --packs /srv/apostille/current/packs \
        || { echo "ABORT: packs not clean"; exit 1; }
else
    echo "(note: no packs/ in this release — packs gate skipped)"
fi
counts() {
    for t in authority fact capture; do
        if [ "$(sudo -u andrii-ai sqlite3 "$DB" "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='$t';")" = 1 ]; then
            printf '%s=%s ' "$t" "$(sudo -u andrii-ai sqlite3 "$DB" "SELECT COUNT(*) FROM \"$t\";")"
        else
            printf '%s=(no table) ' "$t"
        fi
    done
}
echo "counts before: $(counts)"
KEYS="$(sudo -u andrii-ai sqlite3 "$DB" "SELECT key FROM config WHERE key IN ('meta.api_key','media.terms_read');")"
echo "$KEYS" | grep -qx 'meta.api_key' || { echo "ABORT: sealed meta.api_key row missing"; exit 1; }
echo "$KEYS" | grep -qx 'media.terms_read' || { echo "ABORT: media.terms_read row missing"; exit 1; }
echo "sealed rows present: meta.api_key + media.terms_read"

echo "=== 4. mutating-path proof: watch-sweep + hcch-poll as andrii-ai"
sudo -u andrii-ai "$XTASK" watch-sweep --store "$STORE" --actor loop-first-run \
    || { echo "ABORT: watch-sweep failed"; exit 1; }
# hcch-poll exit 2 is NoSource by construction (fixed flags, no
# other exit-2 path in run_hcch): the ONE status-table URI is not
# seeded yet (A2 follow-up: packs-load, then fetch-url). Announced
# skip — any other failure still aborts.
HCCH_OUT="$(sudo -u andrii-ai "$XTASK" hcch-poll --store "$STORE" --actor loop-first-run 2>&1)"
HCCH_CODE=$?
echo "$HCCH_OUT"
if [ "$HCCH_CODE" = 2 ]; then
    echo "(note: hcch unseeded — continuing; seed it via packs-load + fetch-url afterwards)"
elif [ "$HCCH_CODE" != 0 ]; then
    echo "ABORT: hcch-poll failed (exit $HCCH_CODE)"
    exit 1
fi

echo "=== 5. stage andrii-ai-readable KEK copy (deleted after the run)"
install -d -o andrii-ai -g andrii-ai -m 0700 "$CREDS"
cp "$KEK_SRC" "$STAGED"
chown andrii-ai:andrii-ai "$STAGED"
chmod 0600 "$STAGED"
[ "$(stat -c %s "$STAGED")" = 32 ] || { echo "ABORT: staged KEK is not 32 bytes"; exit 1; }
echo "staged: $STAGED (andrii-ai 0600)"

echo "=== 6. KEK-gated fixed command via the hal path"
sudo -u hal sudo -n -u andrii-ai /usr/local/sbin/apostille-loop \
    || { echo "ABORT: loop fixed command failed (output above)"; exit 1; }

echo "=== 7. verify: staged KEK gone, no root-owned residue, counts after"
if [ -e "$STAGED" ]; then
    echo "WARNING: staged KEK survived the wrapper trap — deleting now"
    rm -f "$STAGED"
fi
rmdir "$CREDS" 2>/dev/null || echo "WARNING: $CREDS not empty, left in place (inspect)"
ROOT_OWNED="$(find "$STORE" -user root -print || true)"
[ -z "$ROOT_OWNED" ] || { echo "ABORT: root-owned files under $STORE:"; echo "$ROOT_OWNED"; exit 1; }
echo "no root-owned files under $STORE"
ls -l "$DB" "$STORE"/handbook.sqlite-wal "$STORE"/handbook.sqlite-shm 2>/dev/null || true
echo "counts after: $(counts)"
echo "DONE loop first run — paste this whole output back to the agent"
