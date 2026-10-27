# TLS certificates: Google Public CA via ACME (decided prompt 013)

- Constraint: the ONLY certificates we may use come from Google
  PKI. No Let's Encrypt, no ZeroSSL, no self-signed in prod.
- Mechanism: certbot on the server speaks ACME to Google Public
  CA with External Account Binding (EAB); our Rust binary
  terminates TLS with the issued files. One EAB secret per ACME
  account; each secret dies 7 days after creation if unused, and
  dies on first use (single registration).
- Endpoints (Google docs, verified 2026-10-26):
  - Production: `https://dv.acme-v02.api.pki.goog/directory`
  - Staging (untrusted, same checks):
    `https://dv.acme-v02.test-api.pki.goog/directory`
- GCP project: `www-andrii-ai`. The EAB secret binds the ACME
  account to this project.

## Author runbook (run on your machine with your gcloud)

```sh
# 1. One-time: enable the Public CA API on the project.
gcloud services enable publicca.googleapis.com \
  --project=www-andrii-ai

# 2. Mint an EAB secret (keyId + b64MacKey) into a LOCAL file.
#    Never commit this file; it is a credential. Use within 7 days.
gcloud publicca external-account-keys create \
  --project=www-andrii-ai \
  --key-output-file="$HOME/.config/andrii/eab-prod.txt"
chmod 600 "$HOME/.config/andrii/eab-prod.txt"
```

Then hand the server ONLY the two values (see `deploy.md` for
where they land: TPM-sealed config, never in the repo, never in
shell history on the server — paste via a file + `shred` after
import, or type into the admin config page).

```sh
# 3. (Optional, for staging-box testing later.) A staging EAB
#    needs the staging API endpoint override (exact commands from
#    the Public CA tutorial's Staging section):
gcloud config set api_endpoint_overrides/publicca \
  https://preprod-publicca.googleapis.com/
gcloud publicca external-account-keys create \
  --project=www-andrii-ai \
  --key-output-file="$HOME/.config/andrii/eab-staging.txt"
chmod 600 "$HOME/.config/andrii/eab-staging.txt"
gcloud config unset api_endpoint_overrides/publicca
```

Server side (no Caddy — prompt 015): our Rust binary terminates
TLS itself (axum-server + rustls, cert/key from
`/var/lib/apostille/tls/`). Issuance + renewal is certbot
(OS-level tooling, Google CA only) in standalone mode: certbot
binds :80 itself while the service is briefly stopped
(~30s downtime per issuance/renewal, every ~60 days —
acceptable for now; the zero-downtime upgrade path below
removes even that).

```sh
# One-time registration (as root; EAB values from step 2):
certbot register --server https://dv.acme-v02.api.pki.goog/directory \
  --eab-kid "$EAB_KID" --eab-hmac-key "$EAB_HMAC" \
  --email <author-email> --no-eff-email

# Issue + install. Run BEFORE the first `systemctl start`
# (nothing listens on :80 yet, so no --pre-hook needed the
# first time). Renewals reuse these hooks automatically.
certbot certonly --server https://dv.acme-v02.api.pki.goog/directory \
  -d andrii.ai \
  --standalone --preferred-challenges http \
  --http-01-address 199.184.91.41 \
  --pre-hook "systemctl stop apostille" \
  --post-hook "systemctl start apostille" \
  --deploy-hook "/usr/local/sbin/apostille-cert-deploy"
```

(`--http-01-address` is load-bearing: sibling services own
:80 on OTHER IPs of this box; a wildcard standalone bind
would fail — or worse, briefly steal their port.)

The deploy-hook (draft: `docs/ops/apostille-cert-deploy.sh`,
root-owned 755, `chattr +i` recommended) installs
`fullchain.pem`/`privkey.pem` to
`/var/lib/apostille/tls/{cert.pem,key.pem}` (key 600,
`andrii-ai:andrii-ai`), restarts the service (the binary
loads certs at boot; SIGHUP hot-reload is a follow-up), and
smokes the public endpoint. Nothing else on the box touches
private keys.

Zero-downtime upgrade path (later, optional): switch the
renewal authenticator to `--manual` http-01 with auth/cleanup
hooks writing `$CERTBOT_TOKEN` files into
`/var/lib/apostille/acme/` (mode 644, root:andrii-ai),
served by the app's `/.well-known/acme-challenge/` route
(already implemented + tested) — then drop the stop/start
hooks and renewals never interrupt the service.

## Rotation / revocation

- EAB secrets are single-use: each fresh ACME account (prod,
  staging, rebuild-after-disk-loss) needs a new `create` call.
- If `eab-prod.txt` leaks: it cannot revoke issued certs, but
  request a new secret for any future account and delete the
  file. Leaked CERT private keys live only on the server disk;
  rotate by deleting `/var/lib/apostille/tls/*` and re-issuing
  plus one service restart (downtime ~seconds).
