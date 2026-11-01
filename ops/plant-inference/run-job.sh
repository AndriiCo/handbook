#!/usr/bin/env bash
# One manual inference job on plant. Usage: ./run-job.sh <job-id> <image.png>
# Deployed by `cargo xtask plant-run setup`; do not edit on plant.
set -euo pipefail
cd ~/models
JOB="${1:?usage: run-job.sh <job-id> <image>}"
IMG="${2:?usage: run-job.sh <job-id> <image>}"
# shellcheck disable=SC1091
source ./MODEL.conf
B=cudart-llama-b12020-bin-ubuntu-cuda-12.8-x64
BIN="$HOME/models/$B/bin/llama-cli"
export LD_LIBRARY_PATH="$HOME/models/$B:${LD_LIBRARY_PATH:-}"
exec 9>~/apostille-pipeline/gpu.lock
flock -n 9 || { echo "GPU busy, try later"; exit 1; }
nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits -l 2 >"$JOB.vram.log" &
SAMPLER=$!
START=$(date +%s)
"$BIN" -m "$MODEL_FILE" --mmproj "$MMPROJ_FILE" -ngl 99 \
  --image "$IMG" -p "$(cat "$PROMPT_FILE")" >"$JOB.verdict.txt" 2>"$JOB.stderr.txt"
SECS=$(($(date +%s) - START))
kill "$SAMPLER" 2>/dev/null
PEAK=$(grep -o '[0-9]*' "$JOB.vram.log" | sort -n | tail -1)
python3 - "$JOB.verdict.txt" "$JOB.metrics.json" "$MODEL_ID" "$SECS" "$PEAK" <<'PYEOF'
import json,sys
vtxt,mout,model,secs,peak=sys.argv[1:6]
verdict=json.load(open(vtxt))
json.dump({"job":vtxt.split('.')[0],"model":model,"verdict":verdict.get("verdict"),"issues":verdict.get("issues",[]),"wall_secs":int(secs),"peak_vram_mib":int(peak),"recorded_at":__import__('datetime').datetime.now(__import__('datetime').timezone.utc).isoformat()},open(mout,'w'),indent=2)
print(open(mout).read())
PYEOF
printf '| %s | %s | %s | %s | %ss | %sMiB |\n' "$JOB" "$MODEL_ID" "$IMG" "$(python3 -c "import json;print(json.load(open('$JOB.verdict.txt'))['verdict'])")" "$SECS" "$PEAK" >>JOBS.md
echo "done: $JOB ${SECS}s peak ${PEAK}MiB"
