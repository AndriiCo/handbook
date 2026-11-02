#!/usr/bin/env bash
# worker.sh — async gated build worker (runs on plant as an unprivileged user, no root).
# Usage: worker.sh loop   — poll queue/ forever (systemd user unit / setsid)
#        worker.sh once   — process a single queued job, then exit
set -euo pipefail

ROOT="$HOME/apostille-pipeline"
HIST="$ROOT/history.jsonl"

log_hist() { # $1=run $2=event $3=json-extra (already-escaped fragment or empty)
  local run="$1" event="$2" extra="${3:-}"
  local ts
  ts=$(date -u +%FT%TZ)
  if [ -n "$extra" ]; then
    printf '{"ts":"%s","run":"%s","event":"%s",%s}\n' "$ts" "$run" "$event" "$extra" >>"$HIST"
  else
    printf '{"ts":"%s","run":"%s","event":"%s"}\n' "$ts" "$run" "$event" >>"$HIST"
  fi
}

stage() { # $1=run $2=stage-name $3...=command — run, log, record; returns cmd status
  local run="$1" name="$2"; shift 2
  local slog="$ROOT/logs/$run.$name.log" start end code
  start=$(date +%s)
  log_hist "$run" "stage_start" "\"stage\":\"$name\""
  set +e
  "$@" >"$slog" 2>&1
  code=$?
  set -e
  end=$(date +%s)
  cat "$slog" >>"$ROOT/logs/$run.log"
  log_hist "$run" "stage_end" "\"stage\":\"$name\",\"exit\":$code,\"secs\":$((end - start))"
  return $code
}

run_probe() { # $1=run $2=srcdir
  local run="$1" src="$2"
  stage "$run" versions bash -c "
    echo run=$run sha=$(jq -r .sha "$ROOT/running/$run.json");
    cargo --version; rustc --version; node --version; npm --version;
    python3 --version; git --version;
    which google-chrome || echo no-system-chrome;
    ls ~/.cache/ms-playwright 2>/dev/null || echo no-playwright-browsers;
    nproc; free -h | head -n 2"
}

run_build() { # $1=run $2=srcdir
  local run="$1" src="$2"
  export PW_CHANNEL="${PW_CHANNEL:-}"
  if [ -z "$(ls -A ~/.cache/ms-playwright 2>/dev/null)" ] && [ -z "$PW_CHANNEL" ]; then
    if ! stage "$run" pw_install bash -c "cd '$src' && npx --prefix web playwright install chromium"; then
      echo "pw_install failed, falling back to system chrome" >>"$ROOT/logs/$run.log"
      export PW_CHANNEL=chrome
      log_hist "$run" "pw_fallback" '"channel":"chrome"'
    fi
  fi
  stage "$run" deps bash -c "cd '$src' && cargo fetch --locked && npm ci --prefix web" || return 1
  stage "$run" gate bash -c "cd '$src' && cargo xtask ci" || return 1
  stage "$run" package bash -c "
    set -euo pipefail
    cd '$src'
    cargo build --locked --release -p web -p xtask
    (cd web && npm run build && npm run build:admin)
    OUT='$ROOT/artifacts/$run'
    rm -rf \"\$OUT\"; mkdir -p \"\$OUT/spa\"
    cp target/release/web target/release/xtask \"\$OUT/\"
    cp -r web/build/. \"\$OUT/spa/\"
    cp -r web/build-admin/. \"\$OUT/spa-admin/\"
    cp -r packs/. \"\$OUT/packs/\" 2>/dev/null || mkdir -p \"\$OUT/packs\"
    cp -r crates/web/static/. \"\$OUT/static/\" 2>/dev/null || mkdir -p \"\$OUT/static\"
    GLIBC=\$(ldd --version 2>/dev/null | head -n 1); GLIBC=\${GLIBC:-no-ldd}
    printf '%s\n' \"\$GLIBC\" >\"\$OUT/glibc.txt\"
    jq -n --arg run '$run' --arg sha \"\$(jq -r .sha '$ROOT/running/$run.json')\" \
      --arg cargo \"\$(cargo --version)\" --arg rustc \"\$(rustc --version)\" \
      --arg node \"\$(node --version)\" --arg pw \"\${PW_CHANNEL:-bundled}\" \
      --arg glibc \"\$GLIBC\" \
      '{run:\$run,sha:\$sha,cargo:\$cargo,rustc:\$rustc,node:\$node,playwright:\$pw,glibc:\$glibc}' \
      >\"\$OUT/build-info.json\"
    (cd \"\$OUT\" && sha256sum web xtask build-info.json glibc.txt >SHA256SUMS)
    SHAX=\$(jq -r .sha '$ROOT/running/$run.json')
    STAMP=\$(printf %s \"\$SHAX\" | cut -c1-12)
    jq -n --arg sha \"\$SHAX\" --arg run '$run' --arg out \"\$OUT\" \
      --arg ts \"\$(date -u +%FT%TZ)\" \
      '{sha:\$sha,ci_green:true,run:\$run,finished:\$ts,staged_path:\$out}' \
      >\"$ROOT/acceptance/\$STAMP.json\"
  " || return 1
}

process_one() { # returns 0 if a job was processed, 1 if queue empty
  local job
  job=$(ls "$ROOT/queue/"*.json 2>/dev/null | head -n 1 || true)
  [ -n "$job" ] || return 1
  local run base
  base=$(basename "$job" .json); run="$base"
  mv "$job" "$ROOT/running/$run.json"
  [ -f "$ROOT/queue/$run.tar.gz" ] && mv "$ROOT/queue/$run.tar.gz" "$ROOT/running/$run.tar.gz"
  : >"$ROOT/logs/$run.log"
  local sha type actor attempt
  sha=$(jq -r .sha "$ROOT/running/$run.json")
  type=$(jq -r .type "$ROOT/running/$run.json")
  actor=$(jq -r .actor "$ROOT/running/$run.json")
  attempt=$(jq -r '.attempt // 1' "$ROOT/running/$run.json")
  log_hist "$run" "claimed" "\"sha\":\"$sha\",\"type\":\"$type\",\"actor\":\"$actor\",\"attempt\":$attempt"
  echo "=== run $run sha $sha type $type actor $actor attempt $attempt ===" >>"$ROOT/logs/$run.log"

  local src="$ROOT/runs/$run/src" ok=1
  if [ -f "$ROOT/running/$run.tar.gz" ]; then
    rm -rf "$ROOT/runs/$run"; mkdir -p "$src"
    if stage "$run" setup bash -c "
      tar -xzf '$ROOT/running/$run.tar.gz' -C '$src' &&
      cd '$src' && git init -q -b plant && git add -A &&
      git -c user.email=plant@localhost -c user.name=plant commit -qm 'plant $sha'
    "; then
      if [ "$type" = probe ]; then run_probe "$run" "$src" && ok=0 || ok=1
      else run_build "$run" "$src" && ok=0 || ok=1; fi
    else ok=1; fi
  else
    echo "ABORT: missing tarball for $run (nothing staged, prod untouched)" >>"$ROOT/logs/$run.log"; ok=1
  fi

  local dest result last_fail
  if [ "$ok" -eq 0 ]; then dest=done; result=passed; else
    dest=failed; result=failed
    last_fail=$(grep -h "\"run\":\"$run\"" "$HIST" | grep stage_end | grep -v '"exit":0' | tail -n 1 | jq -r .stage 2>/dev/null || true)
    echo "ABORT: stage ${last_fail:-setup} failed; job stopped before package (nothing staged, prod untouched)" >>"$ROOT/logs/$run.log"
  fi
  jq --arg r "$result" '. + {result:$r}' "$ROOT/running/$run.json" >"$ROOT/$dest/$run.json"
  rm -f "$ROOT/running/$run.json" "$ROOT/running/$run.tar.gz"
  log_hist "$run" "final" "\"result\":\"$result\""
  echo "=== run $run $result ===" >>"$ROOT/logs/$run.log"
  prune_old || true
  return 0
}

prune_old() { # keep only the newest $KEEP_RUNS runs + artifacts + logs
  local keep="${KEEP_RUNS:-3}" old
  for d in runs artifacts; do
    # Names sort chronologically (YYYYMMDD-HHMMSS-sha), but mtime is truth.
    # shellcheck disable=SC2012
    ls -t "$ROOT/$d" 2>/dev/null | tail -n +"$((keep + 1))" | while IFS= read -r old; do
      [ -n "$old" ] && rm -rf "$ROOT/$d/$old"
    done
  done
  # Per-run logs follow the same rule; worker.log + history stay.
  # shellcheck disable=SC2012
  ls -t "$ROOT"/logs/*.log 2>/dev/null \
    | grep -v '/worker\.log$' \
    | tail -n +"$((keep * 5 + 1))" | while IFS= read -r old; do
    [ -n "$old" ] && rm -f "$old"
  done
  # acceptance/*.json stamps, done/ + failed/ verdicts and history.jsonl
  # are bytes, not builds — they stay (deploys check stamps against them).
  return 0
}

requeue_crashed() { # jobs left in running/ by a dead worker get one retry
  local f run attempt
  for f in "$ROOT"/running/*.json; do
    [ -e "$f" ] || continue
    run=$(basename "$f" .json); attempt=$(jq -r '.attempt // 1' "$f")
    if [ "$attempt" -ge 2 ]; then
      jq '. + {result:"failed",note:"worker died twice"}' "$f" >"$ROOT/failed/$run.json"
      rm -f "$f" "$ROOT/running/$run.tar.gz"
      log_hist "$run" "final" '"result":"failed","note":"worker died twice"'
    else
      jq '.attempt = ($attempt + 1)' --argjson attempt "$attempt" "$f" >"$ROOT/queue/$run.json"
      [ -f "$ROOT/running/$run.tar.gz" ] && mv "$ROOT/running/$run.tar.gz" "$ROOT/queue/$run.tar.gz"
      rm -f "$f"
      log_hist "$run" "requeued" "\"attempt\":$((attempt + 1))"
    fi
  done
}

cmd="${1:-once}"
case "$cmd" in
  once) process_one ;;
  loop)
    exec 9>"$ROOT/worker.lock"
    flock -n 9 || { echo "worker already running"; exit 0; }
    requeue_crashed
    echo "$(date -u +%FT%TZ) worker loop started pid $$" >>"$ROOT/worker.log"
    while true; do
      process_one >>"$ROOT/worker.log" 2>&1 || sleep 10
    done
    ;;
  *) echo "usage: worker.sh {loop|once}"; exit 2 ;;
esac
