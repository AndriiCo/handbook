#!/bin/bash
# promote.sh — Mac-relayed plant artifact promotion (Appendix J D3).
# Plant never touches production: the Mac fetches the plant pipeline
# artifact, verifies it locally, then relays it to prod staging.
# Never deploys (that stays an explicit train action).
#
#   xtask/ops/promote.sh <plant-run> <version7> <full-sha> --max-glibc <X.Y>
#
# Gates (each ABORTs with a clear message):
#   1. SHA256SUMS: present in the artifact and verifies (shasum -c).
#   2. BUILD_SHA: the staged web binary names BUILD_SHA=<version7>.
#   3. GLIBC: max GLIBC_X.Y the binary requires is <= --max-glibc
#      (parity rule in docs/ops/staging.md; prod glibc is an input,
#      never guessed — the flag is required).
# Plus an artifact-identity check: build-info.json sha == <full-sha>
# (a wrong-SHA artifact, e.g. a pipeline tarball build, stops here).
#
# Dry run (no ssh/scp; checks run for real against a local fixture):
#   DRY_RUN=1 xtask/ops/promote.sh RUN abc1234 <40hex> --max-glibc 2.39
#   DRY_RUN_ABORT=sha256|buildsha|glibc|identity => force that ABORT path.
set -u

cd "$(git rev-parse --show-toplevel)"

RUN="${1:-}"; VERSION="${2:-}"; SHA="${3:-}"
[ $# -ge 3 ] || { echo "ABORT: usage: $0 <plant-run> <version7> <full-sha> --max-glibc <X.Y>"; exit 2; }
shift 3
MAX_GLIBC=""
while [ $# -gt 0 ]; do
    case "$1" in
        --max-glibc) MAX_GLIBC="${2:-}"; shift 2 ;;
        *) echo "ABORT: unknown flag: $1 (only --max-glibc <X.Y>)"; exit 2 ;;
    esac
done
[[ "$VERSION" =~ ^[0-9a-f]{7}$ ]] || { echo "ABORT: bad version7: $VERSION"; exit 2; }
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || { echo "ABORT: bad full sha: $SHA"; exit 2; }
[[ "$MAX_GLIBC" =~ ^[0-9]+\.[0-9]+$ ]] || { echo "ABORT: --max-glibc <X.Y> is required (prod glibc, never guessed)"; exit 2; }
[ -n "$RUN" ] || { echo "ABORT: empty plant run id"; exit 2; }
echo "== promote $VERSION ($SHA) from plant run $RUN (max glibc $MAX_GLIBC)"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/promote-$VERSION.XXXXXX")"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: building local fixture artifact (no scp)"
    mkdir -p "$TMP/artifact/spa"
    {
        printf '\177ELF\002\001\001\000'
        printf 'BUILD_SHA=%s ' "$VERSION"
        printf 'GLIBC_2.17 GLIBC_2.34 '
        printf 'x%.0s' $(seq 1 200)
    } > "$TMP/artifact/web"
    [ "${DRY_RUN_ABORT:-}" = "buildsha" ] && { printf '\177ELF binary without the stamp' > "$TMP/artifact/web"; }
    [ "${DRY_RUN_ABORT:-}" = "glibc" ] && { printf '\177ELF BUILD_SHA=%s GLIBC_99.0' "$VERSION" > "$TMP/artifact/web"; }
    cp "$TMP/artifact/web" "$TMP/artifact/xtask"
    echo "<html></html>" > "$TMP/artifact/spa/index.html"
    printf '{"run":"%s","sha":"%s","glibc":"ldd fixture"}\n' "$RUN" "$SHA" > "$TMP/artifact/build-info.json"
    [ "${DRY_RUN_ABORT:-}" = "identity" ] && printf '{"run":"%s","sha":"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}\n' "$RUN" > "$TMP/artifact/build-info.json"
    printf 'ldd fixture\n' > "$TMP/artifact/glibc.txt"
    (cd "$TMP/artifact" && shasum -a 256 web xtask build-info.json glibc.txt > SHA256SUMS)
    [ "${DRY_RUN_ABORT:-}" = "sha256" ] && printf 'tampered' >> "$TMP/artifact/web"
    ART="$TMP/artifact"
else
    scp -r "plant:~/apostille-pipeline/artifacts/$RUN" "$TMP/artifact" \
        || { echo "ABORT: scp artifact $RUN from plant failed (is the run packaged? see plant ~/apostille-pipeline/done/$RUN.json)"; exit 1; }
    ART="$TMP/artifact"
fi

echo "== 1. SHA256SUMS"
[ -f "$ART/SHA256SUMS" ] || { echo "ABORT: $RUN has no SHA256SUMS (worker package stage must write it)"; exit 1; }
(cd "$ART" && shasum -a 256 -c SHA256SUMS) \
    || { echo "ABORT: SHA256SUMS mismatch for $RUN (artifact corrupt or tampered — nothing relayed)"; exit 1; }
echo "sha256 OK"

echo "== 2. BUILD_SHA names $VERSION"
grep -a -q "BUILD_SHA=$VERSION" "$ART/web" \
    || { echo "ABORT: artifact web does not name BUILD_SHA=$VERSION (wrong-SHA build — stage the exact SHA natively instead)"; exit 1; }
echo "build-sha OK"

echo "== 3. GLIBC <= $MAX_GLIBC"
NEED="$(strings "$ART/web" 2>/dev/null | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -t. -k1,1n -k2,2n | tail -n 1 || true)"
[ -n "$NEED" ] || { echo "ABORT: no GLIBC requirements found in artifact web (cannot prove prod parity)"; exit 1; }
echo "max required GLIBC: $NEED (cap $MAX_GLIBC)"
awk -v need="$NEED" -v cap="$MAX_GLIBC" 'BEGIN { split(need, N, "."); split(cap, C, "."); if (N[1] > C[1] || (N[1] == C[1] && N[2] > C[2])) exit 1; exit 0; }' \
    || { echo "ABORT: artifact needs GLIBC $NEED > prod cap $MAX_GLIBC (rebuild against an older toolchain)"; exit 1; }
echo "glibc OK"

echo "== 4. artifact identity"
INFO_SHA="$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).sha)' "$ART/build-info.json" 2>/dev/null || true)"
[ "$INFO_SHA" = "$SHA" ] \
    || { echo "ABORT: artifact build-info sha ${INFO_SHA:-unreadable} != $SHA (not the promoted commit)"; exit 1; }
echo "identity OK: $INFO_SHA"

echo "== 5. relay to prod staging (plant never touches production)"
if [ -n "${DRY_RUN:-}" ]; then
    echo "DRY-RUN: would ssh andrii.ai mkdir -p apostille/staging/$VERSION/{spa,spa-admin,packs,static}"
    echo "DRY-RUN: would scp web spa packs static build-info.json glibc.txt SHA256SUMS -> andrii.ai:apostille/staging/$VERSION/"
    echo "DRY-RUN: would write $SHA to andrii.ai:apostille/staging/$VERSION/SHA"
else
    ssh andrii.ai "mkdir -p apostille/staging/$VERSION/spa apostille/staging/$VERSION/spa-admin apostille/staging/$VERSION/packs apostille/staging/$VERSION/static" || exit 1
    scp "$ART/web" "andrii.ai:apostille/staging/$VERSION/web" || exit 1
    [ -d "$ART/spa" ] && scp -r "$ART/spa/." "andrii.ai:apostille/staging/$VERSION/spa/" || true
    [ -d "$ART/spa-admin" ] && scp -r "$ART/spa-admin/." "andrii.ai:apostille/staging/$VERSION/spa-admin/" || true
    [ -d "$ART/packs" ] && scp -r "$ART/packs/." "andrii.ai:apostille/staging/$VERSION/packs/" || true
    [ -d "$ART/static" ] && scp -r "$ART/static/." "andrii.ai:apostille/staging/$VERSION/static/" || true
    scp "$ART/build-info.json" "$ART/glibc.txt" "$ART/SHA256SUMS" "andrii.ai:apostille/staging/$VERSION/" || exit 1
    printf '%s\n' "$SHA" > "$TMP/SHA" && scp "$TMP/SHA" "andrii.ai:apostille/staging/$VERSION/SHA" || exit 1
fi
echo "RELAYED $VERSION — to deploy: xtask/ops/release-train.sh $VERSION $SHA"
