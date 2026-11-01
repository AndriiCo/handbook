#!/bin/bash
# First nightly backup + enable the backup timer. Run as root AFTER
# ops-units-reinstall.sh (which leaves the backup timer disabled by
# design — this script is the enable decision, executed with proof).
#
#   bash /home/hal/apostille/scripts/ops-backup-enable.sh
#
# Starts apostille-backup.service once (skipped if today's daily
# already exists), verifies the daily (exact file set +
# `sha256sum -c`), proves the restore side with `backup-verify`
# into a scratch dir (drill-3 leg 1, deleted afterwards), then
# enables the timer. Paste the whole output back to the agent.
set -u

[ "$(id -u)" = 0 ] || { echo "ABORT: run as root"; exit 1; }

XTASK=/srv/apostille/current/xtask
DEST=/var/lib/apostille-backups
TODAY="$(date -u +%F)"
DAILY=$DEST/daily/$TODAY

echo "=== 0. preconditions"
[ -x "$XTASK" ] || { echo "ABORT: no live release xtask at $XTASK"; exit 1; }
"$XTASK" help | grep -q 'backup-verify' || { echo "ABORT: live xtask has no backup tasks (deploy first)"; exit 1; }
/usr/bin/systemctl cat apostille-backup.service >/dev/null 2>&1 \
    || { echo "ABORT: apostille-backup.service not installed (run ops-units-reinstall.sh first)"; exit 1; }
/usr/bin/systemctl cat apostille-backup.timer >/dev/null 2>&1 \
    || { echo "ABORT: apostille-backup.timer not installed (run ops-units-reinstall.sh first)"; exit 1; }
echo "preconditions OK (today: $TODAY)"

echo "=== 1. first backup run (once)"
if [ -d "$DAILY" ]; then
    echo "(note: $DAILY already exists — start skipped, verifying it instead)"
else
    /usr/bin/systemctl start apostille-backup.service \
        || { echo "ABORT: backup service failed (journal below)"; /usr/bin/journalctl -u apostille-backup.service -n 20 --no-pager || true; exit 1; }
    echo "backup service ran once"
fi
/usr/bin/journalctl -u apostille-backup.service -n 8 --no-pager --output=short 2>/dev/null | grep -E 'backup id|sha256' || true

echo "=== 2. verify the daily (file set + sha256sums)"
[ -d "$DAILY" ] || { echo "ABORT: no daily at $DAILY after the run"; exit 1; }
FILES="$(ls "$DAILY")"
echo "$FILES"
for f in handbook.sqlite p2.sqlite files.tar.gz manifest.json sha256sums; do
    [ -f "$DAILY/$f" ] || { echo "ABORT: $DAILY missing $f"; exit 1; }
done
[ "$(ls "$DAILY" | wc -l)" = 5 ] || { echo "ABORT: $DAILY holds extra files (WAL residue?)"; exit 1; }
(cd "$DAILY" && sudo -u andrii-ai sha256sum -c sha256sums) \
    || { echo "ABORT: sha256sums failed in $DAILY"; exit 1; }
echo "daily $TODAY: file set exact, sums OK"

echo "=== 3. restore proof (backup-verify into scratch, drill-3 leg)"
SCRATCH=/tmp/apostille-verify-$TODAY
[ -e "$SCRATCH" ] && { echo "ABORT: $SCRATCH already exists (refusing to mix restores)"; exit 1; }
sudo -u andrii-ai "$XTASK" backup-verify --backup "$DAILY" --scratch "$SCRATCH" \
    || { echo "ABORT: backup-verify failed (scratch left at $SCRATCH for inspection)"; exit 1; }
rm -rf "$SCRATCH"
echo "restore proof green (scratch removed)"

echo "=== 4. enable the timer"
ST="$(/usr/bin/systemctl is-enabled apostille-backup.timer || true)"
if [ "$ST" = enabled ]; then
    echo "(timer already enabled)"
else
    /usr/bin/systemctl enable --now apostille-backup.timer \
        || { echo "ABORT: enable backup timer failed"; exit 1; }
fi
ST="$(/usr/bin/systemctl is-enabled apostille-backup.timer)"
[ "$ST" = enabled ] || { echo "ABORT: backup timer state is $ST (expected enabled)"; exit 1; }
echo "apostille-backup.timer: $ST"
/usr/bin/systemctl list-timers --all | grep -E 'backup|NEXT' || true
echo "DONE backup enable (daily $TODAY proven + timer live) — paste this whole output back to the agent"
