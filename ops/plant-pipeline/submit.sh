#!/usr/bin/env bash
# submit.sh — enqueue an async gated build on plant (runs on the Mac).
# Returns in seconds; the worker picks the job up within ~10 s.
# Usage: submit.sh [--type build|probe] [sha]   (default: build @ HEAD)
set -euo pipefail

TYPE=build
SHA=""
while [ $# -gt 0 ]; do
  case "$1" in
    --type) TYPE="$2"; shift 2 ;;
    -h|--help) sed -n '2,5p' "$0"; exit 0 ;;
    *) SHA="$1"; shift ;;
  esac
done
[ "$TYPE" = build ] || [ "$TYPE" = probe ] || { echo "ABORT: --type must be build|probe"; exit 2; }

cd "$(git rev-parse --show-toplevel)"
GIVEN_SHA="$SHA"
SHA="${SHA:-$(git rev-parse HEAD)}"
git cat-file -e "$SHA^{commit}" 2>/dev/null || { echo "ABORT: not a commit: $SHA (commit first; plant builds committed SHAs only)"; exit 2; }
# Explicit SHAs archive from git, so working-tree edits cannot leak in:
# the tree is checked only for the HEAD default. The session-log hook
# rewrites docs/prompts/sessions after every turn; it never ships, so it
# is always excluded.
if [ -z "$GIVEN_SHA" ] && [ -n "$(git status --porcelain --untracked-files=no -- . ':!docs/prompts/sessions')" ]; then
  echo "ABORT: tracked edits uncommitted — commit or stash first (plant builds $SHA exactly, your edits would not ship)" >&2
  git status --porcelain --untracked-files=no -- . ':!docs/prompts/sessions' >&2
  exit 2
fi

SHORT=$(git rev-parse --short "$SHA")
RUN="$(date -u +%Y%m%d-%H%M%S)-$SHORT"
ACTOR="$(whoami)@$(hostname -s)"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

git archive --format=tar.gz "$SHA" >"$TMP/$RUN.tar.gz"
jq -n --arg run "$RUN" --arg sha "$SHA" --arg type "$TYPE" --arg actor "$ACTOR" \
  --arg ts "$(date -u +%FT%TZ)" '{run:$run,sha:$sha,type:$type,actor:$actor,created_at:$ts,attempt:1}' \
  >"$TMP/$RUN.json"

# The worker triggers on queue/*.json, so the tarball must land first:
# uploading both in one scp let the worker extract a half-written archive
# (gzip EOF at stage setup). Two steps, tarball before manifest.
scp -q "$TMP/$RUN.tar.gz" "plant:apostille-pipeline/queue/" \
  || { echo "ABORT: scp tarball to plant failed (is ~/apostille-pipeline bootstrapped? run bootstrap.sh)"; exit 1; }
scp -q "$TMP/$RUN.json" "plant:apostille-pipeline/queue/" \
  || { echo "ABORT: scp manifest to plant failed (orphan tarball $RUN.tar.gz in queue/)"; exit 1; }
echo "queued $RUN ($TYPE @ $SHORT)"
echo "poll: ./ops/plant-pipeline/status.sh $RUN"
