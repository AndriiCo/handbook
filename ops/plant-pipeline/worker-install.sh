#!/usr/bin/env bash
# worker-install.sh — install or update the plant work-queue worker (P392).
# Runs on the Mac; the plant side runs over `ssh plant` as the unprivileged
# user. No root, ever: writes ~/apostille-worker, ~/.config/apostille and
# /srv/scratch/apostille only (docs/ops/deploy.md § Plant). The operator
# creates /srv/scratch/apostille once, as root (the script says how).
# Usage: worker-install.sh [--restart] [sha]   (default: HEAD, committed SHAs only)
# The token file (~/.config/apostille/worker.env) is written by hand once;
# see ops/plant-pipeline/README.md § Work-queue worker.
set -euo pipefail

RESTART=0
SHA=""
while [ $# -gt 0 ]; do
  case "$1" in
    --restart) RESTART=1; shift ;;
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    *) SHA="$1"; shift ;;
  esac
done

cd "$(git rev-parse --show-toplevel)"
SHA="${SHA:-$(git rev-parse HEAD)}"
git cat-file -e "$SHA^{commit}" 2>/dev/null || { echo "ABORT: not a commit: $SHA"; exit 2; }
URL="${APOSTILLE_WORKER_URL:-wss://andrii.ai/apostille/api/v1/worker/ws}"
S=/srv/scratch/apostille

ssh plant "set -euo pipefail
  if [ ! -d $S ] || [ ! -w $S ]; then
    echo 'ABORT: $S is missing or not writable by this user. Once, as root on plant:'
    echo '  sudo mkdir -p $S && sudo chown \$(id -un): $S'
    exit 2
  fi
  mkdir -p $S/work $S/meta-cache $S/fetch-cache $S/models $S/target ~/apostille-worker
  mkdir -p ~/.config/apostille && chmod 700 ~/.config/apostille
  if [ -f ~/.config/apostille/worker.env ]; then chmod 600 ~/.config/apostille/worker.env
  else echo 'NOTE: ~/.config/apostille/worker.env is missing; the worker exits until it exists'; fi
  echo WORKER-DIRS-OK"

# Build beside the live tree, then swap: a failed build leaves the old one.
git archive --format=tar.gz "$SHA" | ssh plant "set -euo pipefail
  . \"\$HOME/.cargo/env\" 2>/dev/null || true
  rm -rf ~/apostille-worker/src.new && mkdir -p ~/apostille-worker/src.new
  tar -xzf - -C ~/apostille-worker/src.new
  echo $SHA > ~/apostille-worker/src.new/.sha
  cd ~/apostille-worker/src.new
  CARGO_TARGET_DIR=$S/target cargo build --release -p xtask 2>&1 | tail -n 3
  cd ~ && rm -rf ~/apostille-worker/src.old
  if [ -d ~/apostille-worker/src ]; then mv ~/apostille-worker/src ~/apostille-worker/src.old; fi
  mv ~/apostille-worker/src.new ~/apostille-worker/src
  echo WORKER-BUILD-OK $SHA"

# Cron watchdog (same persistence story as the pipeline worker);
# worker-run.sh rotates worker.log and starts the worker.
ssh plant "( crontab -l 2>/dev/null | grep -v 'apostille-worker'; cat <<CRON
* * * * * sh \$HOME/apostille-worker/src/ops/plant-pipeline/worker-run.sh $URL # apostille-worker
CRON
) | crontab - && echo CRON-WATCHDOG-OK"

if [ "$RESTART" = 1 ]; then
  # Drain, not kill: the running copy finishes and uploads its current
  # job (if any), removes the file and exits; cron then starts the new
  # build within a minute.
  ssh plant "touch $S/work/worker.drain && echo WORKER-DRAINING"
else
  echo "running worker (if any) keeps the old build; rerun with --restart to switch now"
fi
