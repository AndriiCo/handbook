# TLS certificates: Google Public CA via ACME (decided prompt 013)

- Constraint: the ONLY certificates we may use come from Google
  PKI. No Let's Encrypt, no ZeroSSL, no self-signed in prod.
- Mechanism: Caddy on the server speaks ACME to Google Public CA
  with External Account Binding (EAB). One EAB secret per ACME
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

Caddy side (server): the site block sets `acme_ca` to the
production directory URL above plus the EAB `key_id`/`hmac_key`
from step 2. Caddy handles issuance + renewal; nothing else on
the box touches private keys.

## Rotation / revocation

- EAB secrets are single-use: each fresh ACME account (prod,
  staging, rebuild-after-disk-loss) needs a new `create` call.
- If `eab-prod.txt` leaks: it cannot revoke issued certs, but
  request a new secret for any future account and delete the
  file. Leaked CERT private keys live only on the server disk;
  rotate by deleting Caddy's storage for the domain and
  re-issuing (downtime: one ACME round-trip).
