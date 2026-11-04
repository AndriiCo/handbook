#!/bin/bash
# ship-release.sh — Mac-side release build + stage for one SHA (docs/ops/deploy.md
# "How to deploy", steps 1-6). Ends by printing the exact ops-deploy.sh
# invocation; it never deploys (step 7 stays an explicit builder action).
#
#   xtask/ops/ship-release.sh [<sha>] [--allow-today-overlap]   # default: HEAD
#   xtask/ops/ship-release.sh <sha> --migration-ack <prod backup path>
#
# --migration-ack is the operator's migration path (docs/ops/deploy.md
# § Rails): a SHA with a schema/migration delta vs live ships only
# when it names a backup of /var/lib/apostille/handbook.sqlite taken on
# production in the last 2 hours (non-empty, readable as hal). No older
# binary opens the migrated store, so that backup IS the rollback.
#
# --allow-today-overlap is an accepted no-op (kept so old orders and
# wrapper scripts keep parsing; D1 removed the day gate from this
# script entirely). Deploys are operator-ordered; xtask deploy and
# ops-deploy.sh keep their own gates.
#
# Zero placeholders: every input is derived or refused with ABORT.
set -u

cd "$(git rev-parse --show-toplevel)"
SHA="${1:-$(git rev-parse HEAD)}"
OVERLAP=""
MIGRATION_ACK=""
case "${2:-}" in
    "") ;;
    --allow-today-overlap) OVERLAP="--allow-today-overlap" ;;
    --migration-ack)
        MIGRATION_ACK="${3:-}"
        [[ "$MIGRATION_ACK" =~ ^/var/lib/apostille/[A-Za-z0-9._/-]+$ ]] \
            || { echo "ABORT: --migration-ack needs the backup path under /var/lib/apostille/"; exit 2; }
        ;;
    *) echo "ABORT: unknown flag: $2 (only --allow-today-overlap, --migration-ack <path>)"; exit 2 ;;
esac
[ -n "$OVERLAP" ] && echo "NOTE: --allow-today-overlap accepted (no-op: no day gate in this script)"
git cat-file -e "$SHA^{commit}" 2>/dev/null || { echo "ABORT: not a commit: $SHA"; exit 2; }
VERSION="$(git rev-parse --short=7 "$SHA")"
[[ "$VERSION" =~ ^[0-9a-f]{7}$ ]] || { echo "ABORT: bad short SHA: $VERSION"; exit 2; }
echo "== ship $VERSION ($SHA)"

# Bounded /healthz poll (D2: no bare sleeps, no single-shot curls on
# load-bearing reads). Prints the body on success; ABORTs with a
# clear timeout message after $2 tries $3 seconds apart.
poll_healthz() {
    local url="$1" tries="${2:-12}" interval="${3:-5}" body i
    for ((i = 1; i <= tries; i++)); do
        body="$(curl -fsS --max-time 15 "$url" 2>/dev/null || true)"
        if [ -n "$body" ]; then
            printf '%s' "$body"
            return 0
        fi
        [ "$i" -lt "$tries" ] && sleep "$interval"
    done
    echo "ABORT: $url unreachable after $tries tries (${interval}s apart)" >&2
    return 1
}

# Plant stamp gate (docs/ops/deploy.md: no stamp with ci_green, no ship).
# Stamp files are acceptance/<sha12>.json on plant.
check_stamp() {
    local sha12 out
    sha12="$(git rev-parse --short=12 "$SHA")"
    out="$(ssh plant 'grep -l "\"ci_green\": true" ~/apostille-pipeline/acceptance/'"$sha12"'*.json')" \
        || { echo "ABORT: no plant stamp with ci_green for $SHA (submit it first)"; exit 1; }
    [ -n "$out" ] || { echo "ABORT: no plant stamp with ci_green for $SHA (submit it first)"; exit 1; }
    echo "stamp OK: $out"
}

# Full root-process probe (docs/ops/deploy.md: the two lines must print
# nothing — checked again right before ops-deploy.sh). The [a] keeps
# pgrep from matching the remote shell running it.
check_roots() {
    local out
    # Trailing `true`: a clean machine makes the last pgrep exit 1
    # (no match), which must not read as an ssh failure — ssh
    # itself still fails closed (unreachable host, bad key), and
    # any output at all still aborts below.
    out="$(ssh andrii.ai 'pgrep -u 0 -c . >/dev/null || echo ROOT-PROCS-HIDDEN; pgrep -af "[a]postille/scripts/"; true')" \
        || { echo "ABORT: root-probe ssh failed ($1)"; exit 1; }
    [ -z "$out" ] || { echo "ABORT: operator script may be running ($1):"; echo "$out"; exit 1; }
}

echo "== 0. plant stamp"
check_stamp

echo "== 1. rails (schema, migrations, unit)"
LIVE_VER="$(poll_healthz https://andrii.ai/apostille/healthz 12 5)" \
    || { echo "ABORT: production /healthz unreachable"; exit 1; }
LIVE_SHA="$(printf '%s' "$LIVE_VER" | grep -o '"version":"0.1.0+[0-9a-f][0-9a-f]*"' | sed 's/.*+//;s/"//')"
[[ "${LIVE_SHA:-?}" =~ ^[0-9a-f]{7,40}$ ]] || { echo "ABORT: unparseable live version: $LIVE_VER"; exit 1; }
git cat-file -e "$LIVE_SHA^{commit}" 2>/dev/null || { echo "ABORT: live SHA $LIVE_SHA not in local repo"; exit 2; }
echo "live SHA: $LIVE_SHA"
HITS="$(git diff "$LIVE_SHA" "$SHA" -- crates xtask \
    | grep -E '^[-+]pub const (P2_)?SCHEMA_VERSION|^\+.*\b(ALTER|DROP) (TABLE|INDEX|COLUMN)|^\+.*\bRENAME (TO|COLUMN)\b' || true)"
MIGRATES=""
[ -z "$HITS" ] || MIGRATES="schema delta vs live $LIVE_SHA:
$HITS"
[ -z "$(git diff --stat "$LIVE_SHA" "$SHA" -- crates/handbook-store/migrations)" ] \
    || MIGRATES="${MIGRATES:-migrations differ from live $LIVE_SHA}"
if [ -n "$MIGRATES" ]; then
    [ -n "$MIGRATION_ACK" ] || { echo "ABORT: $MIGRATES"; echo "(operator migration path: back up handbook.sqlite on production, then --migration-ack <path>)"; exit 1; }
    ssh andrii.ai "test -s '$MIGRATION_ACK' && find '$MIGRATION_ACK' -mmin -120 | grep -q ." \
        || { echo "ABORT: --migration-ack $MIGRATION_ACK is missing, empty or older than 2 h on production"; exit 1; }
    echo "MIGRATION ACK: $MIGRATES"
    echo "MIGRATION ACK: rollback = restore $MIGRATION_ACK (no older binary opens the migrated store)"
fi
git show "$SHA:docs/ops/apostille.service" | ssh andrii.ai 'diff /etc/systemd/system/apostille.service - >/dev/null' \
    || { echo "ABORT: unit file drifted — sync docs/ops/apostille.service first"; exit 1; }
check_roots rails
echo "rails OK"

echo "== 2. prod-before scoreboard"
node xtask/ops/scoreboard.mjs --out web/shots/prod-before.json 2>&1 | tail -2

echo "== 3. worktree build"
# zig 0.17.0 for cargo-zigbuild: the /tmp copy while macOS has not purged
# it (the 11-03 00:00 sweep emptied its lib/), else Homebrew's.
ZIG_TMP=/tmp/zig-aarch64-macos-0.17.0
"$ZIG_TMP/zig" env >/dev/null 2>&1 && export PATH="$ZIG_TMP:$PATH"
[ "$(zig version 2>/dev/null)" = "0.17.0" ] && zig env >/dev/null 2>&1 \
    || { echo "ABORT: no working zig 0.17.0 (brew install zig)"; exit 1; }
WT="../hb-ship-$VERSION"
[ ! -e "$WT" ] || { echo "ABORT: $WT exists (remove it first)"; exit 1; }
git worktree add --detach "$WT" "$SHA" || exit 1
if git diff --quiet "$SHA" HEAD -- web/package-lock.json; then
    ln -s "$PWD/web/node_modules" "$WT/web/node_modules"
else
    (cd "$WT/web" && npm ci --no-audit --no-fund) || exit 1
fi
export CARGO_TARGET_DIR="$PWD/target"
(cd "$WT" && cargo zigbuild --release -p web -p xtask --target x86_64-unknown-linux-gnu) || exit 1
R="$PWD/target/x86_64-unknown-linux-gnu/release"
file "$R/web" | grep -q "ELF 64-bit" || { echo "ABORT: staged web is not an ELF binary"; exit 1; }
grep -a -q "BUILD_SHA=$VERSION" "$R"/build/web-*/output || { echo "ABORT: BUILD_SHA does not name $VERSION"; exit 1; }
(cd "$WT/web" && npm run build >/dev/null && npm run build:admin >/dev/null) || exit 1
[ "$(git -C "$WT" status --porcelain)" = "" ] || { echo "ABORT: worktree dirty (node_modules link?)"; exit 1; }
# Licences page (andrii.ai/licenses.html): regenerate in the worktree
# when 7 days old or its inputs moved, so the staged copy is never
# stale. After the dirty check on purpose; `ci` commits it on main.
cargo xtask licenses --root "$WT" --no-commit || exit 1
echo "build OK"

echo "== 4. stage (worktree only)"
ssh andrii.ai "mkdir -p ~/apostille/staging/$VERSION/spa ~/apostille/staging/$VERSION/spa-admin ~/apostille/staging/$VERSION/packs ~/apostille/staging/$VERSION/static ~/apostille/staging/$VERSION/evals"
scp "$R/web" "andrii.ai:apostille/staging/$VERSION/web" || exit 1
scp -r "$WT/web/build/." "andrii.ai:apostille/staging/$VERSION/spa/" || exit 1
scp -r "$WT/web/build-admin/." "andrii.ai:apostille/staging/$VERSION/spa-admin/" || exit 1
scp -r "$WT/packs/." "andrii.ai:apostille/staging/$VERSION/packs/" || exit 1
scp "$WT/crates/instruct/packs/general.toml" "andrii.ai:apostille/staging/$VERSION/packs/" || exit 1
scp -r "$WT/crates/web/static/." "andrii.ai:apostille/staging/$VERSION/static/" || exit 1
scp "$R/xtask" "andrii.ai:apostille/staging/$VERSION/xtask" || exit 1
scp -r "$WT/evals/." "andrii.ai:apostille/staging/$VERSION/evals/" || exit 1
git -C "$WT" rev-parse HEAD > /tmp/SHA-ship && scp /tmp/SHA-ship "andrii.ai:apostille/staging/$VERSION/SHA" || exit 1
scp "$WT/docs/ops/apostille.service" "andrii.ai:apostille/staging/$VERSION/" || exit 1
scp "$WT/xtask/ops/ops-deploy.sh" andrii.ai:apostille/scripts/ || exit 1
for script in ops-gp2-proposals.sh ops-a2-record.sh ops-a5-eval.sh; do
    scp "$WT/xtask/ops/$script" andrii.ai:apostille/scripts/ || exit 1
done
echo "stage OK"

echo "== 5-6. repeat rails + pre-flight"
check_roots pre-flight
ssh andrii.ai 'touch /srv/apostille/releases/.w && rm /srv/apostille/releases/.w && echo WRITE-OK'
ssh andrii.ai 'touch /srv/apostille/.w && rm /srv/apostille/.w && echo FLIP-OK'
ssh andrii.ai 'sudo -n systemctl status apostille >/dev/null && echo SUDO-OK'
ssh andrii.ai 'echo "SELECT 1;" | sudo -n -u andrii-ai /usr/local/sbin/apostille-sqlite >/dev/null && echo SQL-OK'
ssh andrii.ai 'printf "\n" | sudo -n /usr/local/sbin/apostille-journal >/dev/null && echo JOURNAL-OK'
echo "STAGED $VERSION — to deploy: bash ~/apostille/scripts/ops-deploy.sh ~/apostille/staging/$VERSION [--skip-accept]"
