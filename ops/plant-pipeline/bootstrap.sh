#!/usr/bin/env bash
# bootstrap.sh — one-time (idempotent) plant setup. Runs from the Mac,
# executes the plant side over ssh as chandra. No root, ever.
# Usage: ./ops/plant-pipeline/bootstrap.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
bash -n ops/plant-pipeline/worker.sh
bash -n ops/plant-pipeline/status.sh

ssh plant 'set -euo pipefail
  R=~/apostille-pipeline
  mkdir -p $R/{bin,queue,running,done,failed,logs,runs,artifacts,acceptance}
  mkdir -p ~/.config/systemd/user
  touch $R/history.jsonl
  cat >~/.config/systemd/user/apostille-pipeline-worker.service <<EOF
[Unit]
Description=Apostille plant pipeline worker (unprivileged)
After=network-online.target

[Service]
Type=simple
ExecStart=$R/bin/worker.sh loop
Restart=on-failure
RestartSec=30

[Install]
WantedBy=default.target
EOF
  # Re-arm hook: any ssh login restarts a dead worker (no-op if alive).
  grep -q "apostille-pipeline-worker" ~/.bashrc 2>/dev/null || cat >>~/.bashrc <<EOF

# apostille-pipeline-worker: re-arm plant build worker after reboot (singleton via flock)
[ -x ~/apostille-pipeline/bin/worker.sh ] && (~/apostille-pipeline/bin/worker.sh loop >>~/apostille-pipeline/worker.log 2>&1 &) 
EOF
  echo BOOTSTRAP-DIRS-OK
'
scp -q ops/plant-pipeline/worker.sh ops/plant-pipeline/status.sh plant:apostille-pipeline/bin/
ssh plant 'set -euo pipefail
  chmod +x ~/apostille-pipeline/bin/*.sh
  bash -n ~/apostille-pipeline/bin/worker.sh && echo WORKER-SYNTAX-OK
  # Playwright browsers (user-level; system libs already present with Chrome).
  # Failure here is non-fatal: the worker falls back to PW_CHANNEL=chrome.
  (npx -y playwright@1.64.1 install chromium 2>&1 | tail -n 3) || echo PW-INSTALL-DEFERRED
  # Persistence: plant has Linger=no, so the systemd user manager
  # stops user units at logout (enabling linger needs root — forbidden).
  # The cron watchdog below is the real persistence: cron jobs live
  # under system.slice and survive user-manager teardown. The systemd
  # unit stays for fast pickup while logged in; flock keeps one worker.
  ( crontab -l 2>/dev/null | grep -v "apostille-pipeline" | grep -v "^PATH=/usr/local/bin:/usr/bin:/bin$"; cat <<EOF
PATH=/usr/local/bin:/usr/bin:/bin
* * * * * $HOME/apostille-pipeline/bin/worker.sh loop >>$HOME/apostille-pipeline/worker.log 2>&1
EOF
  ) | crontab - && echo CRON-WATCHDOG-OK
  (systemctl --user daemon-reload && systemctl --user enable --now apostille-pipeline-worker.service 2>&1 | tail -n 2) || true
  sleep 2
  if pgrep -f "worker.sh loop" >/dev/null; then echo WORKER-ALIVE; else
    echo "WORKER-VIA-CRON-WITHIN-60S (logged out soon, cron revives; check: ssh plant pgrep -af worker.sh)"
  fi
  echo ---; cargo --version; rustc --version; node --version
'
