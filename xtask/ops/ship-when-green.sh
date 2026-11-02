#!/bin/bash
# ship-when-green.sh — poll the plant stamp, then run the train
# (Appendix J D5). The Mac waits for plant; it never skips the gate.
#
#   xtask/ops/ship-when-green.sh <full-sha> [--tries N] [--interval S]
#       [--allow-today-overlap] [--skip-accept]
#
# 1. Polls plant ~/apostille-pipeline/acceptance/<sha12>*.json for a
#    row with "ci_green": true (bounded retries, clear timeout
#    message — D2). Submit first with ops/plant-pipeline/submit.sh.
# 2. Runs ship-release.sh (stage) then release-train.sh
#    (prod-before + deploy + verify + auto-rollback).
#
# Dry run (no ssh; downstream scripts echoed, never run):
#   DRY_RUN=1 xtask/ops/ship-when-green.sh <40hex>
#   DRY_RUN_ABORT=stamp => stamp never appears, timeout ABORTs.
set -u

cd "$(git rev-parse --show-toplevel)"

SHA="${1:-}"
[ -n "$SHA" ] || { echo "ABORT: usage: $0 <full-sha> [--tries N] [--interval S] [--allow-today-overlap] [--skip-accept]"; exit 2; }
shift
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || { echo "ABORT: bad full sha: $SHA"; exit 2; }
TRIES=90; INTERVAL=10; SHIP_PASS=""; TRAIN_PASS=""
while [ $# -gt 0 ]; do
    case "$1" in
        --tries) TRIES="${2:-}"; shift 2 ;;
        --interval) INTERVAL="${2:-}"; shift 2 ;;
        # ship-release.sh only accepts --allow-today-overlap;
        # --skip-accept is a train/deploy flag.
        --allow-today-overlap) SHIP_PASS="$SHIP_PASS $1"; TRAIN_PASS="$TRAIN_PASS $1"; shift ;;
        --skip-accept) TRAIN_PASS="$TRAIN_PASS $1"; shift ;;
        *) echo "ABORT: unknown flag: $1"; exit 2 ;;
    esac
done
[[ "$TRIES" =~ ^[0-9]+$ ]] && [ "$TRIES" -ge 1 ] || { echo "ABORT: --tries must be >= 1"; exit 2; }
VERSION="$(git rev-parse --short=7 "$SHA")"
SHA12="$(git rev-parse --short=12 "$SHA")"
echo "== ship-when-green $VERSION ($SHA)"

if [ -n "${DRY_RUN:-}" ]; then TRIES=3; INTERVAL=0; fi

echo "== 1. plant stamp (up to $TRIES tries, ${INTERVAL}s apart)"
FOUND=""
for ((i = 1; i <= TRIES; i++)); do
    if [ -n "${DRY_RUN:-}" ]; then
        [ "${DRY_RUN_ABORT:-}" != "stamp" ] && [ "$i" -ge 2 ] && FOUND="DRY-RUN acceptance/$SHA12-fixture.json"
    else
        FOUND="$(ssh plant 'grep -l "\"ci_green\": true" ~/apostille-pipeline/acceptance/'"$SHA12"'*.json' 2>/dev/null || true)"
    fi
    if [ -n "$FOUND" ]; then
        echo "stamp OK (try $i/$TRIES): $FOUND"
        break
    fi
    [ "$i" -lt "$TRIES" ] && sleep "$INTERVAL"
done
[ -n "$FOUND" ] || {
    echo "ABORT: no plant stamp with ci_green for $SHA after $TRIES tries (${INTERVAL}s apart)"
    echo "submit it first: ops/plant-pipeline/submit.sh $SHA (full SHA — short submits stamp under the wrong name)"
    exit 1
}

echo "== 2. stage"
if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: would run xtask/ops/ship-release.sh $SHA$SHIP_PASS"
else
    # shellcheck disable=SC2086
    xtask/ops/ship-release.sh "$SHA" $SHIP_PASS || exit 1
fi

echo "== 3. train"
if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: would run xtask/ops/release-train.sh $VERSION $SHA$TRAIN_PASS"
    echo "DRY-RUN: ship-when-green complete (stamp + stage + train echoed)"
else
    # shellcheck disable=SC2086
    xtask/ops/release-train.sh "$VERSION" "$SHA" $TRAIN_PASS
fi
