# mTLS seam (H6 fix part 2)

Status: implemented (goal-025 stream 6 of 6). Part 1 — refusing
`admin.mtls_off` in release builds — already merged
(`RuntimeConfig::load`).

Before this seam, strict admin auth (`crates/web/src/admin.rs`) read
a `VerifiedClientCert` request extension that nothing inserted, so
strict mode 403'd every admin caller — including the Telnyx webhook
and staff voice/workers APIs that share the admin listener. This
spec wires the seam: rustls client-cert verification on the admin
TLS listener, a fingerprint layer, and fail-closed behavior
everywhere.

## 1. Design

Two layers, split where each failure belongs:

1. **TLS handshake** (`crates/web/src/mtls.rs`: `MtlsAcceptor`,
   `admin_server_config`). When the optional DB key
   `tls.client_ca_file` (PEM CA bundle, §3) is set AND admin TLS is
   active, the admin listener requests a client certificate and
   verifies its chain against that bundle
   (`WebPkiClientVerifier`). A cert from any other CA fails the
   handshake outright — no HTTP exists past that point.
2. **App auth** (`mtls::inject_verified_cert` +
   `admin::AdminAuth`). The verified leaf's SHA-256 fingerprint
   (lowercase hex of DER, via `handbook_store::store::sha256_hex` —
   the exact function `admin-cert-issue` stores and
   `admin_cert_valid` checks) must name a live `admin_cert` row, or
   strict mode answers 403 and audits (`admin_cert_rejected` /
   `admin_cert_missing`).

Deliberate decision: **presence is enforced at the app layer, not
the handshake** (`allow_unauthenticated` at TLS). Three reasons:

- The Telnyx webhook shares the admin listener and authenticates
  via Ed25519 — it holds no client cert. A mandatory handshake
  would kill it.
- A cert-less staff caller gets an auditable HTTP 403 instead of
  an opaque TLS alert.
- Unknown/revoked certs (CA-chained, so past the handshake) must
  403 via the registry check anyway; presence rides the same path.

End to end the requirement holds: no CA-chained + registered cert,
no admin access.

### Request path

```text
TCP → MtlsAcceptor (handshake, capture peer chain)
    → PeerCertService (insert PeerCertificates ext)
    → inject_verified_cert (leaf → sha256_hex → VerifiedClientCert)
    → host guard → AdminAuth (session + registry check) → handler
```

- `MtlsAcceptor` implements `axum_server::accept::Accept`: it runs
  the stock rustls handshake, then wraps the per-connection tower
  service. Connections without a client cert get the inner service
  unchanged — indistinguishable from plain TLS downstream.
- The fingerprint layer is applied ONCE, outside the merged admin
  router in `main` (approval/config + workers + voice staff +
  webhook), so every `AdminAuth` sees the same TLS-supplied
  identity. Without peer certs it no-ops.
- The **public listener is untouched**: same plain TLS config, no
  acceptor, no layer, no client-cert state.

### Config

| Key | Type | Meaning |
|---|---|---|
| `tls.client_ca_file` | text, optional | PEM CA bundle path for admin client-cert verification |

- Missing/empty ≡ unset (silent, like the other optional paths).
- Set but unreadable/invalid (no `CERTIFICATE` blocks, unparsable)
  → **refuse the boot**. mTLS must never silently downgrade.
- Set while admin TLS is off (plain-HTTP dev) → loud `WARNING`,
  inert.
- No CA → strict behaves exactly as before (403 for lack of a
  verified cert).

### Renewal / rotation

- ACME path: the renewal loop reloads the public config via
  `reload_from_pem` and rebuilds + swaps the admin `ServerConfig`
  (`reload_admin_tls`), re-reading the CA file each time — a
  rotated bundle takes effect at the next renewal with no restart.
- Server cert/key rotation (explicit files): restart (unchanged).
- Client cert revocation: `admin-cert-revoke` takes effect on the
  next request (registry check per request, no caching).

### Redaction

- `PeerCertificates` has a manual `Debug` (count only); DER bytes
  never hit logs.
- All mTLS error strings carry labels/paths only, never PEM bytes.
- Audit rows carry the fingerprint (hex), never key material
  (unchanged from `admin.rs`).

## 2. Cert lifecycle

Owner of issuance: `xtask admin-cert-*` (CA at
`<store>/admin-certs/ca.pem`, key 0600; registry in `admin_cert`).

1. **Bootstrap CA** (once per environment): first
   `admin-cert-issue` self-signs the CA. Back up `ca-key.pem`
   offline; the `ca.pem` path is what `tls.client_ca_file` points
   at (staging and prod have SEPARATE CAs — a staging client cert
   must never verify against prod).
2. **Issue**: `cargo xtask admin-cert-issue --cn <staff> --days 90`
   signs a ClientAuth-EKU cert, records
   (fingerprint → cn, expiry) in `admin_cert`, writes the client
   PEM + 0600 key next to the CA. Deliver the pair to the staffer
   out of band; confirm receipt, then delete the server-side key
   copy (or keep it 0600 — decide per environment, see O3).
3. **Enroll**: issuance IS enrollment (same table `AdminAuth`
   reads). No second step.
4. **Use**: staffer's browser/`curl` presents the client cert on
   `https://admin.<host>/apostille` (single-443 Host routing since
   goal-026; the old `:444` split is gone). Session login still
   required (mTLS is defense in depth, not a session replacement).
5. **Revoke**: `cargo xtask admin-cert-revoke --cn <staff>` (or
   `--fingerprint <hex>`) — effective on the next request. Revoke
   on offboarding, device loss, or suspected key compromise.
6. **Expire**: 90-day default (`--days 1..825`); expiry is enforced
   at the handshake (webpki) once the chain verifies. Re-issue
   before expiry; there is no auto-renewal for client certs.
7. **Rotate CA**: generate a new CA, re-issue all client certs,
   point `tls.client_ca_file` at the new bundle (or a bundle
   containing old+new during overlap), revoke old rows. Restart,
   or wait for the next ACME renewal to pick up the bundle.

## 3. Staging enrollment steps

Run on the staging host (`STORE_DIR` = staging store):

```sh
# 1. Create the staging CA (first issue self-signs it).
cargo xtask admin-cert-issue --cn break-glass --days 30

# 2. Point the admin listener at the staging CA bundle.
cargo xtask config-set --key tls.client_ca_file \
  --type text --value /var/lib/handbook-staging/store/admin-certs/ca.pem

# 3. Confirm strict mode (no bypass row, no dev var can set it).
cargo xtask config-get --key admin.mtls_off   # expect: missing/false

# 4. Restart the release binary; expect in stderr:
#      listening on https://…:443/apostille (single-443, mTLS on the
#      admin Host branch since goal-026; no :444 anymore)

# 5. Negative probes (all hermetic-equivalent, run from a shell):
#    - no cert     → 403 {"error":"client cert required"}
#    - prod CA cert (wrong CA) → TLS handshake failure, no HTTP
#    - break-glass cert, no session → 401
#    - break-glass cert + staff session → 200, fingerprint echoed

# 6. Issue one cert per staffer (--cn <name>), deliver pairs,
#    then revoke break-glass:
cargo xtask admin-cert-revoke --cn break-glass
```

Promote to prod only after the Day-8 demo gate (strict admin API
200/403 matrix green on staging).

## 4. Acceptance

- [ ] Strict admin API 200 with a known-good (CA-chained +
      enrolled) client cert; `/admin/me` echoes its fingerprint.
- [ ] 403 with a CA-chained but unknown cert; 403 with a revoked
      cert; 403 with no cert (handshake still succeeds).
- [ ] Wrong-CA cert fails the TLS handshake (no HTTP response).
- [ ] Dev header `x-admin-cert-sha256` ignored in strict mode
      (enrolled fingerprint via header, no TLS cert → 403).
- [ ] No `tls.client_ca_file` → strict behaves as before (403s).
- [ ] Public listener serves plain TLS cert-less; public routes
      unaffected.
- [ ] Set-but-bad CA file refuses the boot; CA set without admin
      TLS warns loudly and stays inert.
- [ ] `PeerCertificates` `Debug` and all mTLS errors carry no key
      material.
- [ ] Registry count test bumped (30 keys: 4 bool / 24 text /
      2 secret); overlay test covers the new key.
- [ ] `cargo xtask ci` green (or test/clippy `-D warnings`/fmt on
      `config` + `web` when disk is tight).

All hermetic tests live in `crates/web` (`mtls::tests` for the
layer/parsing/redaction units, `main::tests::mtls_*` for the
port-0 TLS matrix with synthetic rcgen certs) and `crates/config`
(registry + overlay).

## 5. Open questions

- **O1 (SEC): handshake-mandatory vs app-enforced presence?**
  Shipped app-enforced so the Ed25519 webhook shares the
  listener. If SEC wants handshake-mandatory instead, the webhook
  must move listeners first (sibling-stream change).
- **O2 (author): prod CA custody?** Who holds `ca-key.pem`, and
  where (HSM/age-encrypted backup)? Staging CA may live on-host;
  prod should not.
- **O3 (author): server-side client-key copies?** `admin-cert-issue`
  writes the client key next to the CA. Keep 0600 (recovery) or
  delete-after-delivery (forward secrecy)? Recommend delete.
- **O4 (SEC): CRL/OCSP?** Revocation is registry-only (per-request
  DB check). No CRL distribution point or OCSP responder — fine
  while the registry is authoritative, revisit past ~50 certs.
- **O5 (author): `docs/specs/config-db.md` §2.1 table** still says
  29 keys — needs the `tls.client_ca_file` row (sibling-stream
  file, left for its owner).
```

Follow-up: the config-db.md table note.
