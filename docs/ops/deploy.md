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
  && STORE_DIR=\$HOME/apostille/data \
     setsid ./current/web </dev/null >>apostille.log 2>&1; exit"
# Runtime settings (ports, SPA dir, …) live in sealed DB rows now
# (`docs/specs/config-db.md`); only STORE_DIR stays env. Non-default
# values must be `config-set` into the store before first boot.
# Smoke (runs ON the box; the binary binds 127.0.0.1 only):
ssh andrii.ai "curl -fsS http://127.0.0.1:3000/apostille/healthz"
```

Verified 2026-10-26: healthz JSON, shell 200, demo
login → me all green on the box. The process survives SSH
logout but NOT reboot (Linger=no, no unit file). Stop with
`pkill -f "^\./current/web$"` — the anchored pattern matches
only the staging `./current/web` cmdline (the old
`apostille/current/we[b]` pattern never matched it and left a
stale binary serving, found 2026-10-27; the `^` also excludes
prod `/srv/apostille/current/web`). Localhost-only by
construction — nothing public until the unit + certs land.

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

Server-side prerequisites (one-time, needs root):
`docs/ops/apostille.service` installed to
`/etc/systemd/system/` (runs `/srv/apostille/current/web` as
`andrii-ai:andrii-ai` with only `CAP_NET_BIND_SERVICE`,
binds 80/443 on the .41 addresses directly — no Caddy, no
proxy); TLS files + certbot hooks per `certificates.md`;
`hal` sudoers entry for the reviewed ship script:
`hal ALL=(root) NOPASSWD: /usr/local/sbin/apostille-ship *`
(script at `docs/ops/apostille-ship.sh`; install root-owned
755, `chattr +i` recommended). Journald collects stdout
logs per `prod.md`.

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

## Status: LIVE since 2026-10-27 (~05:20 UTC)

Root install completed by the author; public smoke green:
healthz, shell, JSON 404, demo login → me (proves the
`andrii-ai` user writes `/var/lib/apostille`), Google Trust
Services cert valid to 2027-01-25. Interim hal service retired.

## Root install (one-time) — copy-paste for the author

Prereqs: `andrii-ai` user exists; EAB `keyId`/`hmac` minted
(prompt 013 gcloud); the three drafts reviewed
(`apostille.service`, `apostille-ship.sh`,
`apostille-cert-deploy.sh`).

```sh
# ---- Step 0, on the Mac: stage the reviewed files. ----
cd /Users/andrii/Code/handbook
scp docs/ops/apostille.service docs/ops/apostille-ship.sh \
  docs/ops/apostille-cert-deploy.sh andrii.ai:/tmp/

# ---- Step 1, on the server as root: install units/scripts. ----
id andrii-ai   # must exist
install -o root -g root -m 644 \
  /tmp/apostille.service /etc/systemd/system/apostille.service
install -o root -g root -m 755 \
  /tmp/apostille-ship.sh /usr/local/sbin/apostille-ship
install -o root -g root -m 755 \
  /tmp/apostille-cert-deploy.sh /usr/local/sbin/apostille-cert-deploy
chattr +i /usr/local/sbin/apostille-ship \
  /usr/local/sbin/apostille-cert-deploy
echo 'hal ALL=(root) NOPASSWD: /usr/local/sbin/apostille-ship *' \
  > /etc/sudoers.d/apostille-ship
chmod 440 /etc/sudoers.d/apostille-ship
visudo -c   # must print OK — stop here if not
install -d -o root -g andrii-ai -m 755 \
  /srv/apostille /srv/apostille/releases
apt update && apt install -y certbot
systemctl daemon-reload

# ---- Step 2, on the server as root: register + first cert. ----
# Paste the EAB values (shell vars, not committed anywhere):
EAB_KID='paste-keyId'; EAB_HMAC='paste-b64MacKey'
certbot register --server https://dv.acme-v02.api.pki.goog/directory \
  --eab-kid "$EAB_KID" --eab-hmac-key "$EAB_HMAC" \
  --email <author-email> --no-eff-email
# First issuance: nothing listens on :80 yet, so standalone just works.
certbot certonly --server https://dv.acme-v02.api.pki.goog/directory \
  -d andrii.ai \
  --standalone --preferred-challenges http \
  --http-01-address 199.184.91.41 \
  --pre-hook "systemctl stop apostille" \
  --post-hook "systemctl start apostille" \
  --deploy-hook "/usr/local/sbin/apostille-cert-deploy"
unset EAB_KID EAB_HMAC
ls -l /var/lib/apostille/tls/   # cert.pem + key.pem (600) must exist

# ---- Step 3, on the server as hal: first ship + enable. ----
sudo /usr/local/sbin/apostille-ship c1960da   # smokes itself
# Back as root: start on boot (only AFTER the first green smoke).
systemctl enable apostille

# ---- Step 4, on the server as hal: retire the interim service. ----
pkill -f "^\./current/web$"

# ---- Step 5, on the Mac: public verification. ----
curl -fsS https://andrii.ai/apostille/healthz
```

Rollback of the install itself: `systemctl disable --now
apostille`, remove the sudoers drop-in + unit + scripts
(`chattr -i` first), and the interim hal service can be
restarted per the interim section.

## End of the direct path (day 10)

After day 10 the direct Mac→prod path is forbidden: every
release must pass acceptance on staging first (see
`staging-plan.md`). Enforcement: `xtask release` (to be built)
will refuse without a fresh staging acceptance stamp.
