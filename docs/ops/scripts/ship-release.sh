#!/bin/bash
# ship-release.sh — Mac-side release build + stage for one SHA (AGENTS.md §7
# "How to deploy", steps 1-6). Ends by printing the exact ops-deploy.sh
# invocation; it never deploys (step 7 stays an explicit builder action).
#
#   docs/ops/scripts/ship-release.sh [<sha>]   # default: HEAD
#
# Zero placeholders: every input is derived or refused with ABORT.
set -u

cd "$(git rev-parse --show-toplevel)"
SHA="${1:-$(git rev-parse HEAD)}"
git cat-file -e "$SHA^{commit}" 2>/dev/null || { echo "ABORT: not a commit: $SHA"; exit 2; }
VERSION="$(git rev-parse --short=7 "$SHA")"
[[ "$VERSION" =~ ^[0-9a-f]{7}$ ]] || { echo "ABORT: bad short SHA: $VERSION"; exit 2; }
echo "== ship $VERSION ($SHA)"

echo "== 1. rails (schema, migrations, unit)"
HITS="$(git diff 0c8ea63 "$SHA" -- crates xtask \
    | grep -E '^[-+]pub const (P2_)?SCHEMA_VERSION|^\+.*\b(ALTER|DROP) (TABLE|INDEX|COLUMN)|^\+.*\bRENAME (TO|COLUMN)\b' || true)"
[ -z "$HITS" ] || { echo "ABORT: schema delta vs 0c8ea63 — take the migration path with the author:"; echo "$HITS"; exit 1; }
[ -z "$(git diff --stat 0c8ea63 "$SHA" -- crates/handbook-store/migrations)" ] \
    || { echo "ABORT: migrations differ from 0c8ea63 — take the migration path with the author"; exit 1; }
git show "$SHA:docs/ops/apostille.service" | ssh andrii.ai 'diff /etc/systemd/system/apostille.service - >/dev/null' \
    || { echo "ABORT: unit file drifted — sync docs/ops/apostille.service first"; exit 1; }
CLEAN="$(TZ=America/Los_Angeles date +%F)"
[ -z "$(ssh andrii.ai "ls /srv/apostille/releases | grep -x '$CLEAN'")" ] \
    || { echo "ABORT: a release dir dated today already exists (one deploy a day)"; exit 1; }
ssh andrii.ai 'pgrep -af "[a]postille/scripts/"' \
    && { echo "ABORT: root-owned apostille process running"; exit 1; } || true
echo "rails OK"

echo "== 2. prod-before scoreboard"
node .agents/scoreboard/scoreboard.mjs --out web/shots/prod-before.json 2>&1 | tail -2

echo "== 3. worktree build"
WT="../hb-ship-$VERSION"
[ ! -e "$WT" ] || { echo "ABORT: $WT exists (remove it first)"; exit 1; }
git worktree add --detach "$WT" "$SHA" || exit 1
if git diff --quiet "$SHA" HEAD -- web/package-lock.json; then
    ln -s "$PWD/web/node_modules" "$WT/web/node_modules"
else
    (cd "$WT/web" && npm ci --no-audit --no-fund) || exit 1
fi
export CARGO_TARGET_DIR="$PWD/target"
export PATH="/tmp/zig-aarch64-macos-0.17.0:$PATH"
(cd "$WT" && cargo zigbuild --release -p web -p xtask --target x86_64-unknown-linux-gnu) || exit 1
R="$PWD/target/x86_64-unknown-linux-gnu/release"
file "$R/web" | grep -q "ELF 64-bit" || { echo "ABORT: staged web is not an ELF binary"; exit 1; }
grep -a -q "BUILD_SHA=$VERSION" "$R"/build/web-*/output || { echo "ABORT: BUILD_SHA does not name $VERSION"; exit 1; }
(cd "$WT/web" && npm run build >/dev/null && npm run build:admin >/dev/null) || exit 1
[ "$(git -C "$WT" status --porcelain)" = "" ] || { echo "ABORT: worktree dirty (node_modules link?)"; exit 1; }
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
scp "$WT/docs/ops/scripts/ops-deploy.sh" andrii.ai:apostille/scripts/ || exit 1
echo "stage OK"

echo "== 5-6. repeat rails + pre-flight"
[ -z "$(ssh andrii.ai "ls /srv/apostille/releases | grep -x '$CLEAN'")" ] \
    || { echo "ABORT: today got spent while staging (one deploy a day)"; exit 1; }
ssh andrii.ai 'pgrep -af "[a]postille/scripts/"' \
    && { echo "ABORT: root-owned apostille process running"; exit 1; } || true
ssh andrii.ai 'touch /srv/apostille/releases/.w && rm /srv/apostille/releases/.w && echo WRITE-OK'
ssh andrii.ai 'touch /srv/apostille/.w && rm /srv/apostille/.w && echo FLIP-OK'
ssh andrii.ai 'sudo -n systemctl status apostille >/dev/null && echo SUDO-OK'
ssh andrii.ai 'echo "SELECT 1;" | sudo -n -u andrii-ai /usr/local/sbin/apostille-sqlite >/dev/null && echo SQL-OK'
ssh andrii.ai 'printf "\n" | sudo -n /usr/local/sbin/apostille-journal >/dev/null && echo JOURNAL-OK'
echo "STAGED $VERSION — to deploy: bash ~/apostille/scripts/ops-deploy.sh ~/apostille/staging/$VERSION [--skip-accept]"
