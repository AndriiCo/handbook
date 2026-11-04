#!/usr/bin/env bash
# status.sh — poll plant pipeline state (runs anywhere, read-only over ssh).
# Usage: status.sh [run-id]   (no arg: queue summary + recent history)
set -euo pipefail

if [ $# -eq 0 ]; then
  ssh plant '
    R=~/apostille-pipeline
    echo "== queue: $(ls $R/queue/*.json 2>/dev/null | wc -l | tr -d " ") | running: $(ls $R/running/*.json 2>/dev/null | wc -l | tr -d " ") | done: $(ls $R/done/*.json 2>/dev/null | wc -l | tr -d " ") | failed: $(ls $R/failed/*.json 2>/dev/null | wc -l | tr -d " ") =="
    pgrep -f "worker.sh loop" >/dev/null && echo "pipeline worker: alive" || echo "pipeline worker: NOT RUNNING"
    W=/srv/scratch/apostille/work
    pgrep -f "xtask worker --connect" >/dev/null && echo "queue worker: alive" || echo "queue worker: NOT RUNNING"
    [ -f $W/worker.drain ] && echo "queue worker: draining (exits after its current job)"
    tail -n 3 $W/worker.log 2>/dev/null | sed "s/^/  /"
    echo "== recent finals =="
    grep -h "\"event\":\"final\"" $R/history.jsonl 2>/dev/null | tail -n 10 || echo "(no history yet)"
  '
else
  ssh plant "
    R=~/apostille-pipeline; run='$1'
    for d in queue running done failed; do
      [ -f \$R/\$d/\$run.json ] && { echo \"state: \$d\"; cat \$R/\$d/\$run.json; }
    done
    echo '== stages =='
    grep -h \"\\\"run\\\":\\\"\$run\\\"\" \$R/history.jsonl 2>/dev/null || echo '(no events yet)'
    echo '== log tail =='
    tail -n 30 \$R/logs/\$run.log 2>/dev/null || echo '(no log yet)'
  "
fi
