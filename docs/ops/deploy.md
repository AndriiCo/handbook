# Deploy: direct-to-prod (allowed until end of day 10 only)

- Server: `andrii-ai` over SSH (key on author's Mac, never in
  repo). Serves ONLY `199.184.91.41` +
  `2602:f337:ed:4841:4c07:3930:3030:9000` — no other host may
  serve andrii.ai content, and handbook web-research traffic
  (URL watching, fetches) originates ONLY from these IPs, never
  from dev machines.
- OS: Ubuntu 26.04+, x86_64. One box, no scaling (per design).
- Status 2026-10-26: NO `andrii-ai` entry exists in
  `~/.ssh/config` and neither server IP appears there — deploy
  is BLOCKED until the author adds the Host entry (HostName,
  user, port, IdentityFile) or corrects the alias.

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

## Ship (once SSH exists)

```sh
VERSION=$(git rev-parse --short HEAD)
ssh andrii-ai "mkdir -p /srv/apostille/releases/$VERSION"
scp target/x86_64-unknown-linux-gnu/release/web \
  andrii-ai:/srv/apostille/releases/$VERSION/web
scp -r web/build andrii-ai:/srv/apostille/releases/$VERSION/spa
ssh andrii-ai "ln -sfn /srv/apostille/releases/$VERSION \
  /srv/apostille/current && sudo systemctl restart apostille"
curl -fsS https://andrii.ai/apostille/healthz
```

Server-side prerequisites (one-time, author or H-stream):
systemd unit `apostille` (runs `/srv/apostille/current/web`
as an unprivileged user, `STORE_DIR=/var/lib/apostille`,
`SPA_DIR=/srv/apostille/current/spa`, `PORT=3000`,
`Restart=on-failure`); Caddy reverse-proxy
`andrii.ai/apostille/*` → `127.0.0.1:3000` with Google Public
CA ACME (see `certificates.md`); `/var/lib/apostille` owned
by the service user; log dir per `prod.md`.

## Rollback

```sh
PREV=<previous good short-sha>
ssh andrii-ai "ln -sfn /srv/apostille/releases/$PREV \
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
