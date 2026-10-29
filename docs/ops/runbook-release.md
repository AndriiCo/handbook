# Runbook: rootless releases (`xtask deploy` / `xtask rollback`)

Ships `/srv/apostille` as the unprivileged `hal` user — no root
shell, no setuid ship script. Replaces the `apostille-ship.sh`
flow in `deploy.md` (that script stays as the reviewed fallback
until this runbook's first green prod deploy).

Status 2026-10-28: bootstrap is the one-time root script
`docs/ops/scripts/ops-hal-access.sh` (AGENTS.md §7): group +
dirs + pinned sudoers + validating wrappers. The old one-liner
block and the `apostille-hal` drop-in (restart/status only) are
retired — the script subsumes and removes them. Note: this box's
sudo rejects ALL wildcards in command arguments, so every grant
is a fixed path and variable inputs travel via stdin/pointer
files into the root-owned wrappers.

## 0. One-time bootstrap (needs root — author)

```sh
bash /home/hal/apostille/scripts/ops-hal-access.sh
# Paste the whole output back to the agent. The script is
# idempotent (groupadd -f, install -d, drop-in rewrite) and
# self-verifies with read-only probes as hal.
```

What it installs: `hal` in group `apostille`;
`/srv/apostille/releases` 775 + `/srv/apostille` itself
group-writable (the deploy swap writes `current.tmp` + renames
INSIDE the root); pinned grants — fixed systemctl verbs
(restart/status/is-active/reset-failed/daemon-reload, NEVER
stop/disable/start/reload) plus the install-unit, journal,
config, and sqlite wrappers; the legacy `apostille-hal`
drop-in removed (backup at `/root/apostille-hal.sudoers.bak`).

Verify as hal (the pre-flight for every release day):

```sh
touch /srv/apostille/releases/.w && rm /srv/apostille/releases/.w && echo WRITE-OK
touch /srv/apostille/.w && rm /srv/apostille/.w && echo FLIP-OK
sudo -n systemctl status apostille >/dev/null && echo SUDO-OK
sudo -n systemctl stop apostille  # must FAIL: "not allowed"
echo 'SELECT 1;' | sudo -n -u andrii-ai /usr/local/sbin/apostille-sqlite >/dev/null && echo SQL-OK
printf '\n' | sudo -n /usr/local/sbin/apostille-journal >/dev/null && echo JOURNAL-OK
```

## 1. Build (on the Mac, per `deploy.md`)

```sh
export PATH="/tmp/zig-aarch64-macos-0.17.0:$PATH"   # + cargo-zigbuild: cargo install cargo-zigbuild
cargo zigbuild --release -p web -p xtask --target x86_64-unknown-linux-gnu
# `--target` is NOT optional: without it zigbuild emits a macOS
# binary into target/release/ and the stale x86_64 binary gets
# staged (seen 2026-10-29: prod kept reporting the old SHA).
# Verify before staging:
file target/x86_64-unknown-linux-gnu/release/web  # must say ELF x86-64
grep -a BUILD_SHA target/x86_64-unknown-linux-gnu/release/build/web-*/output  # must name HEAD
(cd web && npm run build && npm run build:admin)
cargo xtask web-budget   # advisory size report (no gate since 2026-10-28)
VERSION=$(git rev-parse --short HEAD)
```

(/tmp gets wiped — if the zig toolchain is gone, re-run the
one-time toolchain block in `deploy.md` first. `cargo install
cargo-zigbuild` without `--root` lands in `~/.cargo/bin` and
survives wipes.)

## 2. Stage (Mac → server, as hal)

```sh
ssh andrii.ai "mkdir -p ~/apostille/staging/$VERSION/spa"
scp target/x86_64-unknown-linux-gnu/release/web \
  andrii.ai:apostille/staging/$VERSION/web
scp -r web/build/. andrii.ai:apostille/staging/$VERSION/spa/
scp -r web/build-admin/. andrii.ai:apostille/staging/$VERSION/spa-admin/
scp -r packs/. andrii.ai:apostille/staging/$VERSION/packs/
# F2 (prompt 083): the instruct worker's pack ships with packs/
# (the unit points at current/packs/general.toml; packs-validate
# and packs-load only read *.json, so the .toml is inert to them).
scp crates/instruct/packs/general.toml \
  andrii.ai:apostille/staging/$VERSION/packs/
scp -r crates/web/static/. andrii.ai:apostille/staging/$VERSION/static/
scp target/x86_64-unknown-linux-gnu/release/xtask \
  andrii.ai:apostille/staging/$VERSION/xtask
```

## 3. Deploy (on the server, as hal)

```sh
cd ~/apostille/staging/$VERSION
./xtask deploy --version $VERSION --src . --expect-build <N>
```

What it does, in order: packs-validate + accept gates (refuse
without `--skip-packs`/`--skip-accept`, which prod never uses);
staged-set validation (`web` + `xtask` ELF magic, `spa/` +
`spa-admin/` + `static/` dirs, optional `packs/`); install to
`/srv/apostille/releases/$VERSION` + `release.toml` manifest
(spec: repo-root `release.toml`); atomic `current` swap
(needs the §0 root-dir group-write);
`sudo systemctl restart apostille`; smoke of
`https://andrii.ai/apostille/healthz` (curl exit 0 AND
`"status":"ok"` in the body, plus the `/healthz` build SHA names
the release). Smoke failure flips `current` back
and restarts the old release automatically. Success prunes to the
10 newest release dirs, oldest-first, never the live one.

Flags that matter: `--migration-notes '<text>'` (record every
migration crossed); `--rollback-unsafe` (mark when a release is
not safe to roll back across, even if its schema delta is soft).

## 4. Rollback (on the server, as hal)

```sh
~/apostille/staging/$VERSION/xtask rollback --to <previous-sha>
```

Repoints `current`, restarts, verifies; verify failure restores
the original link and restarts it. Targets need their
`release.toml` (releases shipped by the old `.sh` predate
manifests — backfill from the spec or use `--force`, once).

Guard: SQLite has no down-migrations, so rollback across a HARD
migration — table rebuild, rename, drop, column type change, or
STRICT added to an existing table — is REFUSED unless the author
explicitly overrides:

```sh
./xtask rollback --to <sha> --force --backup-note '<db backup id/location>'
```

Live hard list (`HARD_MIGRATIONS` in `xtask/src/release.rs`):
005 only — the constraints rebuild (every table via `*_new` +
INSERT..SELECT + DROP + RENAME). 006/007/p2_001 are purely
additive (new tables/columns old code never touches).
`rollback_safe = false` on the live release forces the same
override even across soft-only schemas. The backup note is the
audit trail: nightly `.backup` id per `prod.md`, taken BEFORE
the rollback. Plus a revert commit with reason per repo policy.

## 5. Fake-root drill (no server, no sudo — CI-safe)

```sh
cargo xtask deploy --version abc1234 --src /tmp/staging \
  --root /tmp/fake-srv --sha abc1234 --skip-packs --skip-accept \
  --no-restart --smoke-url http://127.0.0.1:9/apostille/healthz
```

`--no-restart` + `--root` + `--smoke-url` are the test seams;
prod never passes `--no-restart`. The unit suite
(`xtask/src/release.rs`, 12 tests) drives `deploy`/`rollback`
against tempdir fake-roots with one-shot HTTP servers on
127.0.0.1:0: gates, ELF refusal, swap atomicity, revert-on-failed
smoke, retention cap, guard matrix, manifest roundtrip.

## 6. Troubleshooting

- `accept gate refused`: the store's latest build is not
  `--expect-build`, or accepted facts changed since the build.
  Rebuild + re-stamp; never `--skip-accept` on prod.
- `packs-validate refused`: fix the pack, rebuild the staging
  dir, re-ship. Never `--skip-packs` on prod.
- `not an ELF binary`: the staged `web` is a script/archive —
  re-run the zigbuild, re-stage.
- `smoke ... curl exit`: service did not come up; the revert
  already ran — check `journalctl -u apostille`, fix, re-ship.
  `revert: ... SMOKE FAILED` means the OLD release no longer
  smokes either: page the author, do not ship again blindly.
- `no previous release`: first-ever deploy failed its smoke;
  nothing to revert to — fix and re-ship the same version.
- `rollback ... REFUSED`: hard crossing — take a DB backup,
  get the author's call, re-run with `--force --backup-note`.
- `sudo: not allowed`: bootstrap (§0) missing or regressed —
  stop, the box is not release-ready.
