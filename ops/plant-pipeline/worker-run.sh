#!/bin/sh
# worker-run.sh: the queue worker's cron entry on plant (installed by
# worker-install.sh; runs every minute). Rotates worker.log past 10 MiB,
# but only while no worker holds worker.lock (a running worker keeps
# writing to its open file), then starts the worker. Extra starts exit
# at once on the lock.
# Usage: worker-run.sh <wss url>
set -u
S=/srv/scratch/apostille
LOG=$S/work/worker.log
URL=${1:?usage: worker-run.sh <wss url>}

if [ -f "$LOG" ] && [ "$(wc -c <"$LOG")" -gt 10485760 ] &&
  flock -n "$S/work/worker.lock" true 2>/dev/null; then
  mv -f "$LOG" "$LOG.1"
fi
cd "$HOME/apostille-worker/src" || exit 1
exec "$S/target/release/xtask" worker --connect "$URL" >>"$LOG" 2>&1
