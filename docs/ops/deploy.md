# Deploy: direct-to-prod (allowed until end of day 10 only)

- Server: `ssh andrii.ai` (alias on the author's Mac ONLY;
  key on the Mac, never in repo). Serves ONLY `199.184.91.41`
  + `2602:f337:ed:4841:4c07:3930:3030:9000` — no other host
  may serve andrii.ai content, and handbook web-research
  traffic (URL watching, fetches) originates ONLY from these
  IPs, never from dev machines.
- Probed 2026-10-26 (prompt 014): host `enodia`, Ubuntu
  26.04.1 x86_64, 12 CPU / 31GB / 732G free, systemd 259,
  glibc 2.43, user `hal`, NO sudo, NO TPM (`/dev/tpm*`
  absent → the TPM-sealed decision's env-only fallback is
  ACTIVE on this box), NO Caddy, Linger=no (user units die
  on logout), port 3000 free, :80/:443 already bound on
  OTHER IPs (.42 etc. — someone else's, do not touch),
  nothing on .41:80/:443 yet.

## Build (cross, on the Mac — no installs)

```sh
# One-time toolchain (all under /tmp; nothing installed):
curl -sL -o /tmp/zig.tar.xz \
  https://ziglang.org/download/0.17.0/zig-aarch64-macos-0.17.0.tar.xz
echo "b607e9b9234790a008116ae5bdb71c6243b84b9fb42a53a9e70fde41c06c536a  /tmp/zig.tar.xz" | shasum -a 256 -c -
tar -C /tmp -xf /tmp/zig.tar.xz
cargo install --root /tmp/czb cargo-zigbuild
rustup target add x86_64-unknown-linux-gnu

# Every release (from the repo root):
export PATH="/tmp/zig-aarch64-macos-0.17.0:/tmp/czb/bin:$PATH"
cargo zigbuild --release -p web --target x86_64-unknown-linux-gnu
(cd web && npm install --no-audit --no-fund && npm run build)
cargo xtask web-budget   # 150KB gate must pass
```

Artifacts: `target/x86_64-unknown-linux-gnu/release/web` +
`web/build/` (static SPA). No other runtime deps. Verified
2026-10-26: ELF 64-bit x86-64, 3.8MB, bundled SQLite compiled
in, max GLIBC 2.30 (Ubuntu 26.04 ships newer — runs fine).
On-target smoke (healthz after first deploy) still pending
SSH access.

## Ship, interim: user-space (no root — used prompt 014)

```sh
VERSION=$(git rev-parse --short HEAD)
ssh andrii.ai "mkdir -p ~/apostille/releases/$VERSION ~/apostille/data"
scp target/x86_64-unknown-linux-gnu/release/web \
  andrii.ai:apostille/releases/$VERSION/web
scp -r web/build andrii.ai:apostille/releases/$VERSION/spa
# NOTE: keep the starter ssh from hanging: exit the session
# explicitly after launch (`exit` below), the setsid child stays.
ssh andrii.ai "ln -sfn ~/apostille/releases/$VERSION ~/apostille/current \
  && cd ~/apostille \
  && PORT=3000 STORE_DIR=\$HOME/apostille/data \
     SPA_DIR=\$HOME/apostille/current/spa \
     setsid ./current/web </dev/null >>apostille.log 2>&1; exit"
# Smoke (runs ON the box; the binary binds 127.0.0.1 only):
ssh andrii.ai "curl -fsS http://127.0.0.1:3000/apostille/healthz"
```

Verified 2026-10-26: healthz JSON, shell 200, demo
login → me all green on the box. The process survives SSH
logout but NOT reboot (Linger=no, no unit file). Stop with
`pkill -f apostille/current/web`. Localhost-only by
construction — nothing public until Caddy lands.

## Ship, final: system paths (needs root — author or H-stream)

```sh
VERSION=$(git rev-parse --short HEAD)
ssh andrii.ai "mkdir -p /srv/apostille/releases/$VERSION"
scp target/x86_64-unknown-linux-gnu/release/web \
  andrii.ai:/srv/apostille/releases/$VERSION/web
scp -r web/build andrii.ai:/srv/apostille/releases/$VERSION/spa
ssh andrii.ai "ln -sfn /srv/apostille/releases/$VERSION \
  /srv/apostille/current && sudo systemctl restart apostille"
curl -fsS https://andrii.ai/apostille/healthz
```

Server-side prerequisites (one-time, needs root): systemd
unit `apostille` (runs `/srv/apostille/current/web` as an
unprivileged user, `STORE_DIR=/var/lib/apostille`,
`SPA_DIR=/srv/apostille/current/spa`, `PORT=3000`,
`Restart=on-failure`); Caddy reverse-proxy
`andrii.ai/apostille/*` → `127.0.0.1:3000` with Google Public
CA ACME (see `certificates.md`); `/var/lib/apostille` owned
by the service user; log dir per `prod.md`.

## Rollback

```sh
PREV=<previous good short-sha>
ssh andrii.ai "ln -sfn /srv/apostille/releases/$PREV \
  /srv/apostille/current && sudo systemctl restart apostille"
```

Keep the last 3 releases; delete older. Rollback is a symlink
flip + restart (<30s), plus a revert commit with reason per
repo policy. Migrations run forward-only at boot: a rollback
across a migration needs the author's explicit call (SQLite
has no down-migrations by design).

## End of the direct path (day 10)

After day 10 the direct Mac→prod path is forbidden: every
release must pass acceptance on staging first (see
`staging-plan.md`). Enforcement: `xtask release` (to be built)
will refuse without a fresh staging acceptance stamp.
