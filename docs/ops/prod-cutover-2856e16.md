# Prod cutover 2856e16 (goal-024+025 train) — operator runbook

Staged by hal 2026-10-27 (hal can write `releases/` but not flip
`current/` or restart — those need root). Running prod stays on
`c1960da` until step 6.

## 0. Facts (read from the live box, no sudo needed to verify)

- Staged: `/srv/apostille/releases/2856e16/` (ELF web+xtask, spa,
  spa-admin, packs, static, release.toml). `rollback_safe = true`,
  no schema changes (schema 7 / p2 2).
- Prod has **no KEK** (old binary boots degraded; new binary
  refuses KEK-less boot by design — OQ-C1). Enrollment below.
- Prod admin API is **currently dead** (strict-by-default + H6
  unwired seam = 403s). This release fixes it once step 4 lands.
- BIND is explicit IPs (never 0.0.0.0): missing `net.bind` rows
  would localhost-only the new binary = outage. Rows below.

## 1. Root: ownership to convention

```sh
chown -R root:andrii-ai /srv/apostille/releases/2856e16
```

## 2. Root: enroll the prod KEK (new — none exists)

```sh
sudo -u andrii-ai /srv/apostille/releases/2856e16/xtask config-enroll \
  --store /var/lib/apostille --kek-file /etc/apostille/handbook-kek \
  --actor prod-cutover
chmod 600 /etc/apostille/handbook-kek  # enroll already 0600; verify
```

File-backed KEK (no TPM on this box — same tradeoff as staging;
move to TPM-sealed when hardware lands).

## 3. Root: seed prod config rows (as andrii-ai — DB is 0640)

```sh
X=/srv/apostille/releases/2856e16/xtask
D=/var/lib/apostille
K=/etc/apostille/handbook-kek
for kv in \
  "net.bind:199.184.91.41" \
  "net.bind6:2602:f337:ed:4841:4c07:3930:3030:9000" \
  "paths.spa_dir:/srv/apostille/current/spa" \
  "paths.static_dir:/srv/apostille/current/static" \
  "paths.admin_spa_dir:/srv/apostille/current/spa-admin" \
  "paths.acme_challenge_dir:/var/lib/apostille/acme" \
  "tls.cert_file:/var/lib/apostille/tls/cert.pem" \
  "tls.key_file:/var/lib/apostille/tls/key.pem" ; do
  sudo -u andrii-ai $X config-set --store $D --kek-file $K \
    --key "${kv%%:*}" --type text --value "${kv#*:}" --actor prod-cutover
done
```

(Ports 443/444/80, `public_host`, strict mTLS all ride correct
defaults — no rows needed. `paths.*` use `current/` so rows
survive flips and rollbacks.)

## 4. Root: prod admin CA + first client cert

```sh
sudo -u andrii-ai $X admin-cert-issue --store $D --cn <you> --days 90 \
  --actor prod-cutover
sudo -u andrii-ai $X config-set --store $D --kek-file $K \
  --key tls.client_ca_file --type text \
  --value /var/lib/apostille/admin-certs/ca.pem --actor prod-cutover
```

Deliver `<you>` cert+key to your machine; keep the CA key on the
box only. Prod and staging CAs must stay separate (staging CA
already exists under `~/apostille/data/admin-certs/`).

## 5. Root: unit change (KEK credential only — LEAVE old env for rollback)

Add to `[Service]` in `/etc/systemd/system/apostille.service`:

```ini
LoadCredential=handbook-kek:/etc/apostille/handbook-kek
```

Then `systemctl daemon-reload`. Do NOT trim the old
`Environment=` lines until the new release has soaked (rollback
needs them). Do NOT restart yet.

## 6. Root: flip + restart + smoke

```sh
ln -sfn /srv/apostille/releases/2856e16 /srv/apostille/current
systemctl restart apostille
sleep 6
curl -fsS https://andrii.ai/apostille/healthz
journalctl -u apostille --since '-2 min' | grep -iE 'refusing|mTLS|listening'
```

Expect `(admin, mTLS)` and no `refusing`. Then the mTLS matrix
(from your machine, with the step-4 cert):

- no cert → 403; wrong CA → handshake failure;
- cert, no session → 401; cert + staff session → 200
  (staff login is still break-glass python-insert until the
  passkey story lands — same recipe as staging).

## 7. Rollback (if red)

```sh
ln -sfn /srv/apostille/releases/c1960da /srv/apostille/current
systemctl restart apostille
```

Old env is still in the unit, so the old binary boots exactly as
before. Then tell the agent what went red.

## 8. After soak (separate step, not cutover)

- Trim superseded `Environment=` lines to `STORE_DIR` only.
- Add the pinned hal sudoers from `runbook-release.md` so future
  ships + restarts run unprivileged via `xtask deploy`.
- Prune `releases/` past 10 versions, oldest-first, never `current`.
