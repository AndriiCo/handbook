#!/bin/bash
# staging-accept.sh — staging acceptance (Appendix J D5): full
# scoreboard --out staging-accept.json + live demo take, then the
# staging-green verdict. Runs on the Mac against staging on plant
# through the ssh tunnel (docs/ops/deploy.md: public 18318).
#
#   xtask/ops/staging-accept.sh --sha <full-sha> [--base <url>]
#
# Verdict rule (docs/ops/deploy.md step 5): staging-green iff the full
# scoreboard is 9/10 with ONLY operator-owned S1 red, or 10/10 once S1
# goes green at submission, AND the live demo take is 3/3. Anything
# else (an unreadable report included) ABORTs with the red rows named.
# Never edits the tree to make a row pass; never starts the tunnel
# itself (macOS has no setsid — the tunnel lives in a managed
# session). A dead tunnel ABORTs with the exact ssh command.
#
# The verdict reads only this run's report: the old report is removed
# before the scoreboard runs, and the report must name this --base and
# carry an `at` from after the run started (a scoreboard that crashed
# before writing one, or a stale all-green file, ABORTs). Both pipelines
# keep their own exit status (pipefail): a demo that fails ABORTs; the
# scoreboard's own non-zero exit is a red row, which the verdict reads.
#
# Dry run (no tunnel, no browser; verdict logic runs for real):
#   DRY_RUN=1 xtask/ops/staging-accept.sh --sha <40hex>
#   DRY_RUN_ABORT=verdict => fixture with GP1 red, verdict ABORTs.
#   DRY_RUN_GREEN=all     => fixture 10/10 (S1 green), verdict passes.
#   DRY_RUN_DEMO=fail     => the demo take fails, ABORT.
#   DRY_RUN_REPORT=missing => no report written (a crashed scoreboard), ABORT.
#   DRY_RUN_REPORT=stale  => an all-green report from an earlier run, ABORT.
#   DRY_RUN_REPORT=other-base => a report for another --base, ABORT.
set -u
set -o pipefail

cd "$(git rev-parse --show-toplevel)"

SHA=""; BASE="http://127.0.0.1:18318/apostille"
while [ $# -gt 0 ]; do
    case "$1" in
        --sha) SHA="${2:-}"; shift 2 ;;
        --base) BASE="${2:-}"; shift 2 ;;
        *) echo "ABORT: unknown flag: $1 (only --sha <full-sha>, --base <url>)"; exit 2 ;;
    esac
done
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || { echo "ABORT: --sha <40-hex full sha> is required"; exit 2; }
VERSION="$(git rev-parse --short=7 "$SHA" 2>/dev/null || true)"
[[ "$VERSION" =~ ^[0-9a-f]{7}$ ]] || { echo "ABORT: $SHA is not a commit in this repo"; exit 2; }
OUT="web/shots/staging-accept.json"
echo "== staging accept $VERSION ($SHA) via $BASE"

echo "== 1. tunnel + exact-SHA pin"
if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: tunnel assumed alive; staging serves 0.1.0+$VERSION (fixture)"
else
    BODY="$(curl -fsS --max-time 15 "$BASE/healthz" 2>/dev/null || true)"
    if [ -z "$BODY" ]; then
        echo "ABORT: staging tunnel dead ($BASE/healthz unreachable)"
        echo "start it in a managed session (macOS has no setsid):"
        echo "  ssh -N -L 18318:127.0.0.1:18318 -L 18319:127.0.0.1:18319 plant"
        exit 1
    fi
    case "$BODY" in
        *"\"version\":\"0.1.0+$VERSION\""*) echo "staging pin OK: serving $VERSION" ;;
        *) echo "ABORT: staging serves unexpected version: $BODY (want 0.1.0+$VERSION)"; exit 1 ;;
    esac
fi

echo "== 2. full scoreboard --out $OUT"
mkdir -p "$(dirname "$OUT")"
# Only this run's report counts: remove the last one, note the start.
rm -f "$OUT"
START="$(node -e 'console.log(new Date(Date.now() - 1000).toISOString())')"
if [ -n "${DRY_RUN:-}" ]; then
    AT="$(node -e 'console.log(new Date().toISOString())')"
    REPORT_BASE="$BASE"
    [ "${DRY_RUN_REPORT:-}" = "stale" ] && AT="2026-01-01T00:00:00.000Z"
    [ "${DRY_RUN_REPORT:-}" = "other-base" ] && REPORT_BASE="https://andrii.ai/apostille"
    if [ "${DRY_RUN_REPORT:-}" = "missing" ]; then
        echo "DRY-RUN: the scoreboard crashed before writing its report (fixture)"
    elif [ "${DRY_RUN_GREEN:-}" = "all" ] || [ -n "${DRY_RUN_REPORT:-}" ]; then
        printf '{"base":"%s","at":"%s","passing":10,"total":10,"next":null,"checks":[{"id":"P0","pass":true},{"id":"GP2-data","pass":true},{"id":"A1","pass":true},{"id":"GP1","pass":true},{"id":"GP2-page","pass":true},{"id":"GP3","pass":true},{"id":"GP4","pass":true},{"id":"A2","pass":true},{"id":"A5","pass":true},{"id":"S1","pass":true}]}\n' "$REPORT_BASE" "$AT" > "$OUT"
        echo "DRY-RUN: fixture scoreboard 10/10 (S1 green)"
    elif [ "${DRY_RUN_ABORT:-}" = "verdict" ]; then
        printf '{"base":"%s","at":"%s","passing":8,"total":10,"next":"GP1","checks":[{"id":"P0","pass":true},{"id":"GP1","pass":false},{"id":"S1","pass":false}]}\n' "$REPORT_BASE" "$AT" > "$OUT"
        echo "DRY-RUN: fixture scoreboard 8/10 (GP1 + S1 red)"
    else
        printf '{"base":"%s","at":"%s","passing":9,"total":10,"next":"S1","checks":[{"id":"P0","pass":true},{"id":"GP2-data","pass":true},{"id":"A1","pass":true},{"id":"GP1","pass":true},{"id":"GP2-page","pass":true},{"id":"GP3","pass":true},{"id":"GP4","pass":true},{"id":"A2","pass":true},{"id":"A5","pass":true},{"id":"S1","pass":false}]}\n' "$REPORT_BASE" "$AT" > "$OUT"
        echo "DRY-RUN: fixture scoreboard 9/10 (only S1 red)"
    fi
else
    # Exit 1 is a red row (the verdict reads which); anything else, or no
    # fresh report below, is a scoreboard that did not finish.
    node xtask/ops/scoreboard.mjs --base "$BASE" --out "$OUT" 2>&1 | tail -12
    SCORE=$?
    if [ "$SCORE" -ne 0 ] && [ "$SCORE" -ne 1 ]; then
        echo "ABORT: scoreboard run failed outright (exit $SCORE, see above)"
        exit 1
    fi
fi

echo "== 3. live demo take (desktop GP1-GP4, admin review+trace, mobile)"
if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: would run DEMO_BASE_URL=$BASE npm run demo --prefix web"
    if [ "${DRY_RUN_DEMO:-}" = "fail" ]; then
        echo "DRY-RUN: 1 failed, 2 passed (fixture)"
        echo "ABORT: live demo take failed (want 3/3 on $VERSION)"
        exit 1
    fi
    echo "DRY-RUN: 3 passed (fixture)"
else
    DEMO_BASE_URL="$BASE" npm run demo --prefix web 2>&1 | tail -8
    DEMO=$?
    if [ "$DEMO" -ne 0 ]; then
        echo "ABORT: live demo take failed (exit $DEMO; want 3/3 on $VERSION)"
        exit 1
    fi
fi

echo "== 4. verdict"
# "none" when every row is green; nothing at all when the report is
# missing, unreadable, from another base or older than this run (never
# mistaken for green).
REDS="$(node -e '
    const fs = require("fs");
    const [file, base, start] = process.argv.slice(1);
    const r = JSON.parse(fs.readFileSync(file, "utf8"));
    const bare = (u) => String(u ?? "").replace(/\/$/, "");
    if (bare(r.base) !== bare(base)) process.exit(1);
    if (!(Date.parse(r.at) >= Date.parse(start))) process.exit(1);
    if (!Array.isArray(r.checks) || r.checks.length === 0) process.exit(1);
    const reds = r.checks.filter((c) => !c.pass).map((c) => c.id);
    console.log(reds.length ? reds.join(",") : "none");
' "$OUT" "$BASE" "$START" 2>/dev/null)"
if [ -z "$REDS" ]; then
    echo "ABORT: no fresh scoreboard report for $BASE from this run ($OUT missing, unreadable, stale or for another base)"
    exit 1
fi
if [ "$REDS" = "S1" ]; then
    echo "staging-green: 9/10 (only operator-owned S1 red) + demo 3/3 on $VERSION"
    exit 0
fi
if [ "$REDS" = "none" ]; then
    echo "staging-green: all rows green (S1 included) + demo 3/3 on $VERSION"
    exit 0
fi
echo "ABORT: not staging-green — red rows: ${REDS:-unreadable report} (rule: 9/10 with only S1 red, or all green, + demo 3/3)"
exit 1
