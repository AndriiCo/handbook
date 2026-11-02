#!/bin/bash
# release-train.sh — Mac-side release train (Appendix J D4): parallel
# prod-before evidence, deploy via ops-deploy.sh on prod, post-verify,
# and AUTOMATIC rollback on any regression.
#
#   xtask/ops/release-train.sh <version7> <full-sha> [--allow-today-overlap] [--skip-accept]
#
# Assumes ~/apostille/staging/<version7> is already staged on prod
# (ship-release.sh or promote.sh). Never stages and never builds.
# Rollback target is the live version read BEFORE the deploy; the
# rollback runs that release's own staged xtask on prod, then the
# healthz pin is re-checked. A failed rollback ABORTs loudly for the
# operator (it never pretends success).
#
# Dry run (no ssh, no scoreboard; parallel shape + verdicts for real):
#   DRY_RUN=1 xtask/ops/release-train.sh abc1234 <40hex>
#   DRY_RUN_ABORT=verify => post-verify regresses, rollback path runs.
set -u

cd "$(git rev-parse --show-toplevel)"

VERSION="${1:-}"; SHA="${2:-}"
[ $# -ge 2 ] || { echo "ABORT: usage: $0 <version7> <full-sha> [--allow-today-overlap] [--skip-accept]"; exit 2; }
shift 2
DEPLOY_FLAGS=""
for flag in "$@"; do
    case "$flag" in
        --allow-today-overlap|--skip-accept) DEPLOY_FLAGS="$DEPLOY_FLAGS $flag" ;;
        *) echo "ABORT: unknown flag: $flag (only --allow-today-overlap, --skip-accept)"; exit 2 ;;
    esac
done
[[ "$VERSION" =~ ^[0-9a-f]{7}$ ]] || { echo "ABORT: bad version7: $VERSION"; exit 2; }
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || { echo "ABORT: bad full sha: $SHA"; exit 2; }
echo "== train $VERSION ($SHA)"

# Bounded /healthz poll: prints body on success, fails with a clear
# timeout message (D2: no bare sleeps anywhere in this lane).
poll_healthz() {
    local url="$1" tries="${2:-18}" interval="${3:-5}" body i
    for ((i = 1; i <= tries; i++)); do
        body="$(curl -fsS --max-time 15 "$url" 2>/dev/null || true)"
        if [ -n "$body" ]; then printf '%s' "$body"; return 0; fi
        [ "$i" -lt "$tries" ] && sleep "$interval"
    done
    echo "ABORT: $url unreachable after $tries tries (${interval}s apart)" >&2
    return 1
}

live_version_of() {
    printf '%s' "$1" | grep -o '"version":"0.1.0+[0-9a-f][0-9a-f]*"' | sed 's/.*+//;s/"//'
}

echo "== 0. previous live version (rollback target)"
if [ -n "${DRY_RUN:-}" ]; then
    PREV="deadbee"
    echo "DRY-RUN: live $PREV (fixture)"
else
    LIVE_BODY="$(poll_healthz https://andrii.ai/apostille/healthz 12 5)" \
        || { echo "ABORT: cannot read live version (train stops before touching prod)"; exit 1; }
    PREV="$(live_version_of "$LIVE_BODY")"
    [[ "${PREV:-?}" =~ ^[0-9a-f]{7,40}$ ]] || { echo "ABORT: unparseable live version: $LIVE_BODY"; exit 1; }
    echo "live PREV: $PREV"
fi
[ "$PREV" != "$VERSION" ] || { echo "ABORT: $VERSION is already live (nothing to train)"; exit 1; }

echo "== 1. prod-before (parallel: scoreboard + plant stamp + live pin)"
BEFORE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/train-before.XXXXXX")"
trap 'rm -rf "$BEFORE_DIR"' EXIT
if [ -n "${DRY_RUN:-}" ]; then
    (sleep 1; printf '{"passing":9,"total":10,"checks":[{"id":"P0","pass":true},{"id":"S1","pass":false}]}\n' > "$BEFORE_DIR/prod-before.json"; echo "before/scoreboard: 9/10 (only S1 red)") &
    (sleep 1; echo "before/stamp: ci_green $SHA (fixture)") &
    (sleep 1; echo "before/live: $PREV serving") &
    wait
    echo "DRY-RUN: fixture kept in $BEFORE_DIR (web/shots untouched)"
else
    node .agents/scoreboard/scoreboard.mjs --out web/shots/prod-before.json > "$BEFORE_DIR/scoreboard.log" 2>&1 &
    SB_PID=$!
    SHA12="$(git rev-parse --short=12 "$SHA")"
    ssh plant 'grep -l "\"ci_green\": true" ~/apostille-pipeline/acceptance/'"$SHA12"'*.json' > "$BEFORE_DIR/stamp.out" 2>"$BEFORE_DIR/stamp.err" &
    STAMP_PID=$!
    poll_healthz https://andrii.ai/apostille/healthz 6 5 > "$BEFORE_DIR/live.out" 2>"$BEFORE_DIR/live.err" &
    LIVE_PID=$!
    FAIL=""
    wait $SB_PID || FAIL="$FAIL scoreboard"
    wait $STAMP_PID || FAIL="$FAIL plant-stamp"
    wait $LIVE_PID || FAIL="$FAIL live-healthz"
    tail -3 "$BEFORE_DIR/scoreboard.log"
    [ -s "$BEFORE_DIR/stamp.out" ] && echo "stamp OK: $(cat "$BEFORE_DIR/stamp.out")" || { [ -z "$FAIL" ] && FAIL="$FAIL plant-stamp-empty"; }
    [ -z "$FAIL" ] || { echo "ABORT: prod-before failed:$FAIL (deploy blocked)"; exit 1; }
    cp web/shots/prod-before.json "$BEFORE_DIR/prod-before.json"
fi
echo "prod-before green"

echo "== 2. deploy $VERSION"
# shellcheck disable=SC2086
if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: would ssh andrii.ai bash apostille/scripts/ops-deploy.sh apostille/staging/$VERSION $DEPLOY_FLAGS"
else
    ssh andrii.ai bash apostille/scripts/ops-deploy.sh "apostille/staging/$VERSION" $DEPLOY_FLAGS \
        || { echo "ABORT: deploy failed (ops-deploy auto-reverts smoke failures — check its output above)"; exit 1; }
fi

echo "== 3. post-verify (healthz pin + prod-after scoreboard)"
REGRESS=""
AFTER_BODY=""
if [ -n "${DRY_RUN:-}" ]; then
    if [ "${DRY_RUN_ABORT:-}" = "verify" ]; then
        AFTER_BODY='{"status":"ok","version":"0.1.0+'"$VERSION"'"}'
        printf '{"passing":8,"total":10,"checks":[{"id":"P0","pass":true},{"id":"GP1","pass":false},{"id":"S1","pass":false}]}\n' > "$BEFORE_DIR/prod-after.json"
        echo "DRY-RUN: fixture prod-after 8/10 (GP1 regressed)"
    else
        AFTER_BODY='{"status":"ok","version":"0.1.0+'"$VERSION"'"}'
        printf '{"passing":9,"total":10,"checks":[{"id":"P0","pass":true},{"id":"S1","pass":false}]}\n' > "$BEFORE_DIR/prod-after.json"
        echo "DRY-RUN: fixture prod-after 9/10 (only S1 red)"
    fi
    AFTER_PIN="$VERSION"
else
    AFTER_BODY="$(poll_healthz https://andrii.ai/apostille/healthz 18 5)" \
        || REGRESS="healthz-unreachable-after-deploy"
    if [ -z "$REGRESS" ]; then
        AFTER_PIN="$(live_version_of "$AFTER_BODY")"
        [ "$AFTER_PIN" = "$VERSION" ] || REGRESS="serving $AFTER_PIN, want $VERSION"
    fi
    if [ -z "$REGRESS" ]; then
        node .agents/scoreboard/scoreboard.mjs --out web/shots/prod-after.json 2>&1 | tail -2
        REGRESS="$(node -e '
            const fs = require("fs");
            const before = JSON.parse(fs.readFileSync(process.argv[1], "utf8")).checks;
            const after = JSON.parse(fs.readFileSync(process.argv[2], "utf8")).checks;
            const was = new Map(before.map((c) => [c.id, c.pass]));
            const bad = after.filter((c) => !c.pass && was.get(c.id)).map((c) => c.id);
            console.log(bad.join(","));
        ' "$BEFORE_DIR/prod-before.json" web/shots/prod-after.json)"
    fi
fi
if [ -n "${DRY_RUN:-}" ] && [ "${DRY_RUN_ABORT:-}" = "verify" ]; then REGRESS="GP1"; fi
if [ -n "${DRY_RUN:-}" ] && [ "${DRY_RUN_ABORT:-}" != "verify" ]; then
    REGRESS="$(node -e '
        const fs = require("fs");
        const before = JSON.parse(fs.readFileSync(process.argv[1], "utf8")).checks;
        const after = JSON.parse(fs.readFileSync(process.argv[2], "utf8")).checks;
        const was = new Map(before.map((c) => [c.id, c.pass]));
        console.log(after.filter((c) => !c.pass && was.get(c.id)).map((c) => c.id).join(","));
    ' "$BEFORE_DIR/prod-before.json" "$BEFORE_DIR/prod-after.json")"
fi

if [ -z "$REGRESS" ]; then
    echo "TRAIN GREEN: $VERSION live, no regressions vs prod-before"
    exit 0
fi

echo "REGRESSION: $REGRESS — automatic rollback to $PREV"
if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: would ssh andrii.ai /srv/apostille/releases/$PREV/xtask rollback --to $PREV --store /var/lib/apostille"
    echo "DRY-RUN: would re-poll healthz for pin $PREV"
    echo "DRY-RUN: ROLLED-BACK $VERSION -> $PREV (regression was: $REGRESS)"
    exit 3
fi
ssh andrii.ai "/srv/apostille/releases/$PREV/xtask rollback --to $PREV --store /var/lib/apostille" \
    || { echo "ABORT: ROLLBACK FAILED — prod state unknown, operator needed (tried to restore $PREV after $VERSION regressed: $REGRESS)"; exit 4; }
BACK_BODY="$(poll_healthz https://andrii.ai/apostille/healthz 18 5)" \
    || { echo "ABORT: rollback ran but healthz unreachable — operator needed (wanted $PREV)"; exit 4; }
BACK_PIN="$(live_version_of "$BACK_BODY")"
[ "$BACK_PIN" = "$PREV" ] \
    || { echo "ABORT: rollback ran but serving $BACK_PIN, want $PREV — operator needed"; exit 4; }
echo "ROLLED-BACK $VERSION -> $PREV (regression was: $REGRESS)"
exit 3
