#!/bin/bash
# staging-accept.sh — staging acceptance (Appendix J D5): full
# scoreboard --out staging-accept.json + live demo take, then the
# staging-green verdict. Runs on the Mac against staging on plant
# through the ssh tunnel (docs/ops/deploy.md: public 18318).
#
#   xtask/ops/staging-accept.sh --sha <full-sha> [--base <url>]
#
# Verdict rule (docs/ops/deploy.md step 5): staging-green iff the full
# scoreboard is 9/10 with ONLY operator-owned S1 red AND the live
# demo take is 3/3. Anything else ABORTs with the red rows named.
# Never edits the tree to make a row pass; never starts the tunnel
# itself (macOS has no setsid — the tunnel lives in a managed
# session). A dead tunnel ABORTs with the exact ssh command.
#
# Dry run (no tunnel, no browser; verdict logic runs for real):
#   DRY_RUN=1 xtask/ops/staging-accept.sh --sha <40hex>
#   DRY_RUN_ABORT=verdict => fixture with GP1 red, verdict ABORTs.
set -u

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
if [ -n "${DRY_RUN:-}" ]; then
    if [ "${DRY_RUN_ABORT:-}" = "verdict" ]; then
        printf '{"base":"%s","passing":8,"total":10,"next":"GP1","checks":[{"id":"P0","pass":true},{"id":"GP1","pass":false},{"id":"S1","pass":false}]}\n' "$BASE" > "$OUT"
        echo "DRY-RUN: fixture scoreboard 8/10 (GP1 + S1 red)"
    else
        printf '{"base":"%s","passing":9,"total":10,"next":"S1","checks":[{"id":"P0","pass":true},{"id":"GP2-data","pass":true},{"id":"A1","pass":true},{"id":"GP1","pass":true},{"id":"GP2-page","pass":true},{"id":"GP3","pass":true},{"id":"GP4","pass":true},{"id":"A2","pass":true},{"id":"A5","pass":true},{"id":"S1","pass":false}]}\n' "$BASE" > "$OUT"
        echo "DRY-RUN: fixture scoreboard 9/10 (only S1 red)"
    fi
else
    node xtask/ops/scoreboard.mjs --base "$BASE" --out "$OUT" 2>&1 | tail -12 \
        || { echo "ABORT: scoreboard run failed outright (see above)"; exit 1; }
fi

echo "== 3. live demo take (desktop GP1-GP4, admin review+trace, mobile)"
if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: would run DEMO_BASE_URL=$BASE npm run demo --prefix web"
    echo "DRY-RUN: 3 passed (fixture)"
else
    DEMO_BASE_URL="$BASE" npm run demo --prefix web 2>&1 | tail -8 \
        || { echo "ABORT: live demo take failed (want 3/3 on $VERSION)"; exit 1; }
fi

echo "== 4. verdict"
REDS="$(node -e '
    const fs = require("fs");
    const r = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    console.log(r.checks.filter((c) => !c.pass).map((c) => c.id).join(","));
' "$OUT")"
if [ "$REDS" = "S1" ]; then
    echo "staging-green: 9/10 (only operator-owned S1 red) + demo 3/3 on $VERSION"
    exit 0
fi
echo "ABORT: not staging-green — red rows: ${REDS:-unreadable report} (rule: 9/10 with only S1 red + demo 3/3)"
exit 1
