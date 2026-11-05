# Threat model: Apostille Handbook

- Status: current as of 2026-11-04 (production `0.1.0+83602a0`). It
  replaces the Day-1 slice model and the 2026-10-27 security review
  (both in git history). `docs/SECURITY.md` keeps the claude-red
  baseline the code is held to; this file holds what is built and the
  one list of open findings.
- Rule: a control is listed as built only with a file the author can
  read; symbols over line numbers, which drift.

## 1. Assets

| Asset | Impact if compromised |
|---|---|
| Customer PII: orders, uploads (photos, scans, IDs), addresses | Identity theft, regulatory harm |
| Customer and staff sessions, passkeys, linked identities | Account takeover; staff takeover reaches every customer |
| Handbook integrity and citations | People plan visits and mailings on false facts |
| Audit log | Lost attribution; tampered history hides abuse |
| Sealed config: KEK, Meta, Stripe, FedEx, Telnyx, Gmail, SMTP keys | Spend, data exposure through processors, impersonation |
| TLS keys and ACME EAB secrets | Interception or impersonation of andrii.ai |
| Call recordings and transcripts | Third-party PII; consent obligations |
| Server availability | Orders and the public guides stop |

## 2. Actors

- Anonymous network attacker: probes `/apostille/*`, WS, sign-in.
- Signed-in customer: tries IDOR on orders, uploads and WS topics,
  stored XSS through submitted fields.
- Hostile upload: polyglots, oversized media, EXIF payloads.
- Staff account holder, or a stolen staff session: the highest-value
  target.
- Plant worker or its token: an outbound WebSocket client that leases
  jobs.
- Prompt injection: hostile text inside fetched pages, emails and
  transcripts that reach the Meta models.
- Compromised dependency (crate, npm package, toolchain binary).
- Insider with root on the server, or push access to the repo.
- Thief with a disk image of the store and backups.

Not modelled: attackers inside Google PKI, Meta, Stripe, FedEx or
Telnyx; an already rooted operator Mac (the trusted build signer).

## 3. Trust boundaries

- **Browser ↔ axum, one 443 listener.** Host routing sends
  `andrii.ai` to the public/customer app and `admin.andrii.ai` to the
  staff app (`crates/web/src/host.rs`); HTTP/3 rides the same
  certificate (`crates/web/src/quic.rs`). Port 80 only redirects.
- **axum ↔ SQLite and the blob store.** Bound parameters only; blob
  paths are hex digests (`crates/handbook-store`).
- **Server ↔ providers.** Meta model API (evidence out, drafts back,
  never trusted as fact), Stripe (Checkout plus signed webhooks),
  FedEx (labels), Telnyx (calls plus signed webhooks), Gmail and SMTP,
  Google PKI (ACME).
- **Server ↔ plant worker.** The worker dials out over WebSocket with
  a staff-minted token (`crates/web/src/worker.rs`); results are
  proposals, never published facts.
- **Operator Mac → server.** `cargo xtask setup` builds a stamped SHA
  in a throwaway worktree and ships it over SSH as `hal`; root steps
  run only through `xtask provision` and pinned wrappers
  (`xtask/ops/wrappers/`, `docs/ops/deploy.md`).

## 4. Controls by component (built)

- **Customer sign-in:** passkeys (`crates/web/src/passkey.rs`,
  `crates/auth/src/passkey.rs`), Apple/Google/Meta OAuth with explicit
  linking (`crates/web/src/oauth.rs`, `oauth_apple.rs`,
  `oauth_meta.rs`; RISC signals in `crates/auth/src/risc.rs`), and
  magic links (`crates/web/src/magic.rs`). Opaque 256-bit sessions in
  `__Host-hb_pub`; linking rules in `docs/auth-playbook.md` §6.
- **Staff sign-in:** passkeys only, invite-based enrolment, step-up
  for sensitive actions (`crates/web/src/staff_passkey.rs`, step-up
  window in `crates/web/src/admin.rs`), `__Host-hb_adm` sessions, an
  admin IP allowlist (`crates/web/src/admin_ip.rs`). Client
  certificates are no longer an auth factor.
- **CSRF and Origin:** double-submit token plus a strict Origin
  allowlist on mutations (`crates/auth/src/csrf.rs`); WebSocket
  upgrades check Origin before any session work and authorize every
  topic (`crates/web/src/ws_bus.rs`, `crates/auth/src/ws.rs`); the
  public bus ignores the staff cookie. The channel map is capped at
  4096 with idle reaping.
- **Rate limits:** every sign-in path (passkey, staff passkey, magic
  link, OAuth, setup key, demo login).
- **Headers:** nosniff, Referrer-Policy, X-Frame-Options and HSTS
  (two years, includeSubDomains) on every response
  (`crates/web/src/main.rs`).
- **Uploads:** per-route body caps and a Content-Length pre-check
  (`crates/web/src/uploads.rs`, `admin.rs`); redaction derivatives
  drop EXIF (`crates/media/src/redact.rs`).
- **Payments:** Stripe Checkout (no card data touches us) and
  signature-verified webhooks (`crates/web/src/stripe.rs`).
- **Voice:** Ed25519 webhook verification (`crates/voice/src/webhook.rs`);
  the browser leg uses short-lived Telnyx credentials
  (`crates/web/src/admin_voice.rs`); recordings are consent-gated and
  swept after 365 days by `retention-sweep`.
- **Worker:** per-token auth and a 24 MiB frame cap
  (`crates/web/src/worker.rs`).
- **Model output:** evidence substituted last in prompts, byte-exact
  citation validation, a closed topic enum
  (`crates/contracts/src/topics.rs`), proposed-only storage; a human
  accepts every fact in `/review`.
- **Store:** foreign keys on, CHECK constraints, append-only audit
  triggers, WAL and busy timeout, transactional migrations, atomic
  blob writes, a traversal-guarded and atomic retention sweep
  (`crates/handbook-store/src/db.rs`, `store.rs`).
- **Secrets:** sealed config rows under a KEK delivered as a systemd
  credential (`crates/config/src/seal.rs`), zeroized in memory,
  write-only in the admin UI; `xtask secrets-scan` in CI and the
  pre-commit hook (`xtask/src/secrets.rs`).
- **Service:** runs as `andrii-ai` with only `CAP_NET_BIND_SERVICE`,
  `NoNewPrivileges`, `ProtectSystem=strict`
  (`xtask/ops/systemd/apostille.service`); ACME and certificate hot
  reload are in the binary (`crates/acme`, `docs/ops/acme.md`).
- **Logs:** the request log records the path, never the query.
- **Release binaries:** the `[profile.release]` hardening in
  `Cargo.toml`; `cargo xtask harden-check` refuses a staged binary
  without PIE, NX stack, full RELRO and the stack protector.
- **Deploy:** a plant CI stamp, the migration rail, the root-process
  probe, a `/healthz` smoke naming the SHA and automatic revert
  (`docs/ops/deploy.md`); nightly backups (`apostille-backup.timer`)
  with restore drills.

## 5. Open findings

Ranked. IDs keep their source: H/M/L from the 2026-10-27 review,
F from `docs/SECURITY.md`.

| ID | Finding | Where | Fix |
|---|---|---|---|
| H2 | SSRF on the watch/fetch path: no scheme allowlist, no private/loopback/link-local rejection, no DNS pinning, ureq's default redirects; robots fails open and is read uncapped. Staff URLs reach it from `/workers`. | `crates/ingest/src/fetch.rs` (`get_raw`, `check_robots`) | Reuse the WARC path's guard, pinning and hop cap (`crates/ingest/src/warc.rs`); cap robots. |
| H4 | The demo login is still mounted on the public router in production (hashed tokens, `demo-` refs only, 10/min/IP; cookie lacks `__Host-`). | `crates/web/src/main.rs` (`/api/v1/demo/login`) | Gate it off in production or remove it. |
| H5 | The public guest login signs anyone into one shared customer account with no credential until 2026-11-21 08:00 UTC. It reaches no other customer and no staff route, and the account cannot add sign-in methods. Guests can see each other's entries and can file orders staff will see. | `crates/auth/src/guest.rs`, `crates/web/src/guest.rs` | Ends by itself at the deadline; locking the account stops new sign-ins sooner (open sessions run out within 12h). Remove the code afterwards. |
| H3 | Upload type comes from the client's Content-Type; no magic-byte sniff. Originals keep EXIF, including GPS. | `crates/media/src/upload.rs` (`classify`) | Sniff and store a canonical MIME; strip or quarantine EXIF at intake. |
| M2 | No Content-Security-Policy; HSTS not preloaded. | `crates/web/src/main.rs` | Nonce or hash CSP for both SPAs; preload once stable. |
| M1 | No rate limit on WS upgrades, uploads or bus publish. | `ws_bus.rs`, `uploads.rs`, `admin.rs` | Per-IP and per-account buckets. |
| M3 | IPs, user agents and emails are hashed with plain SHA-256, which a dictionary reverses. | `crates/auth/src/lib.rs` (`hash_str`) | HMAC with a sealed pepper. |
| M4 | `pii-sweep` misses `audit.reason`, `task.detail`, `source.uri` and transcripts. | `xtask/src/pii_sweep.rs` | Extend the sweep to those surfaces. |
| M6 | `config-set --value` accepts secrets on the command line (shell history, `ps`). | `xtask/src/config.rs` | Refuse `--value` for `--type secret`; use stdin. |
| L2 | The served `robots.txt` is `Allow: /` with no `Disallow: /apostille/api/` (only `web/static/robots.txt` has it). | `crates/web/static/robots.txt` | Serve one file with the Disallow. |
| L3 | Secrets-scan has no KEK-shaped rule, no `.cred`/`eab-*` filename rule, and skips untracked files. | `xtask/src/secrets.rs` | Add the rules; scan untracked files in CI. |
| L4 | TLS versions and suites are rustls defaults, not pinned. | `crates/web/src/mtls.rs` | Pin TLS 1.2+ with modern suites. |
| SC | No `deny.toml`; `cargo xtask ci` runs no cargo-deny, cargo-audit or npm audit. | `xtask/src/main.rs` (`run_ci`) | Add `deny.toml` and the three checks. |
| F6 | `flag()` takes the first of duplicate flags silently; `--actor` defaults to `xtask`. | `xtask/src/main.rs` | Reject duplicates; require or derive the actor. |
| H6r | Dead mTLS remains: optional client-CA layer, the `x-admin-cert-sha256` dev header behind `admin.mtls_off` (refused on non-loopback release binds), and the unused `admin_cert_valid`. | `crates/web/src/mtls.rs`, `admin.rs`, `crates/auth/src/lib.rs` | Remove the cert path, or keep it only for invite enrolment and say so. |

Closed since the review (evidence in the files above): H1 (WS Origin),
H5 (expired certs; certs no longer authenticate), M5 (query in logs),
L1 (staff cookie on the public bus), the KEK zeroize half of M6, the
body-limit half of H3, F1–F5 and F7.

## 6. Accepted and residual risks

- RESIDUAL: one server. Backups are nightly and restore-drilled, but
  the offsite copy is still open (`docs/ops/deploy.md` § Watch).
- RESIDUAL: audit triggers stop code bugs, not `DROP TRIGGER` by
  someone with SQL access; true tamper evidence needs the offsite copy.
- RESIDUAL: no hardware TPM on the server; the KEK is a software-sealed
  systemd credential (`docs/specs/tpm-seal.md`).
- RESIDUAL: model output is validated for shape, not truth; a human
  accepting each fact is the truth gate.
- ACCEPTED: the plant worker runs on a machine we control but do not
  harden like production; its token can lease jobs and post proposals,
  nothing more, and re-minting `worker.token` in `/config` cuts it off.

## 7. Out of scope

- DDoS absorption beyond rate limits and the firewall.
- Physical theft beyond disk encryption.
- Breaches inside the providers listed in §3; we limit what we send.
- The operator Mac and SSH key hygiene (a key compromise means
  redeploying from a clean checkout and rotating keys).
