#!/usr/bin/env bash
# glimmer-serve.sh — self-hosted Muse Glimmer on plant (P404).
#
# Downloads the operator-named Hugging Face repo into
# /srv/scratch/apostille/models (HF_HOME there too, never ~/.cache) and
# serves it loopback-only through llama.cpp's OpenAI-compatible
# `llama-server`, which the `precheck.glimmer` worker kind calls
# (GLIMMER_URL, default http://127.0.0.1:18480/v1). Runs as the plant
# user, never root; the RTX 3060 has 12 GB, so pick a quantisation
# that fits (the served id is recorded on every verdict).
#
#   GLIMMER_REPO=<org/repo> [GLIMMER_REVISION=<rev>] \
#   GLIMMER_GGUF=<model .gguf in the repo> GLIMMER_MMPROJ=<vision projector .gguf> \
#   ops/open-weights/glimmer-serve.sh download|serve|check
#
# download  fetch the two files (an HF token for a gated repo comes from
#           `hf auth login` / HF_TOKEN in this shell only, never the repo)
# serve     start llama-server on 127.0.0.1:${GLIMMER_PORT:-18480}
# check     GET /v1/models on the running server
set -euo pipefail

ROOT="${GLIMMER_ROOT:-/srv/scratch/apostille/models}"
PORT="${GLIMMER_PORT:-18480}"
export HF_HOME="$ROOT/hf-home"
REPO="${GLIMMER_REPO:-}"
DEST="$ROOT/${REPO//\//__}"
LLAMA_DIR="${LLAMA_DIR:-$ROOT/llama.cpp}"

need() { [ -n "${!1:-}" ] || { echo "ABORT: set $1" >&2; exit 2; }; }

case "${1:-}" in
  download)
    need GLIMMER_REPO; need GLIMMER_GGUF; need GLIMMER_MMPROJ
    [ -w "$ROOT" ] || { echo "ABORT: $ROOT is not writable (operator: mkdir + chown it)" >&2; exit 1; }
    mkdir -p "$DEST" "$HF_HOME"
    command -v hf >/dev/null || { echo "ABORT: install huggingface_hub (pip install -U huggingface_hub) for the hf CLI" >&2; exit 1; }
    hf download "$REPO" "$GLIMMER_GGUF" "$GLIMMER_MMPROJ" \
      ${GLIMMER_REVISION:+--revision "$GLIMMER_REVISION"} --local-dir "$DEST"
    ( cd "$DEST" && sha256sum "$GLIMMER_GGUF" "$GLIMMER_MMPROJ" | tee SHA256SUMS )
    echo "downloaded $REPO into $DEST"
    ;;
  serve)
    need GLIMMER_REPO; need GLIMMER_GGUF; need GLIMMER_MMPROJ
    BIN="$LLAMA_DIR/bin/llama-server"
    [ -x "$BIN" ] || { echo "ABORT: no llama-server at $BIN (unpack the CUDA build into $LLAMA_DIR)" >&2; exit 1; }
    export LD_LIBRARY_PATH="$LLAMA_DIR:$LLAMA_DIR/bin:${LD_LIBRARY_PATH:-}"
    exec "$BIN" -m "$DEST/$GLIMMER_GGUF" --mmproj "$DEST/$GLIMMER_MMPROJ" \
      --alias "${GLIMMER_MODEL:-$REPO}" -ngl 99 --host 127.0.0.1 --port "$PORT"
    ;;
  check)
    curl -fsS "http://127.0.0.1:$PORT/v1/models"
    echo
    ;;
  *)
    sed -n '2,19p' "$0"
    exit 2
    ;;
esac
