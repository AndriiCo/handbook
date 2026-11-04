# Zero-env config migration (config-db)

- Status: draft for review (goal-025 stream 4 of 5, base `f5258e1`)
- Date: 2026-10-27
- Decision: [README §27](README.md) — zero env vars except
  unavoidable bootstrap (store path); everything in DB per
  [secure-config.md](secure-config.md).
- Scope: `crates/config` (typed keys + defaults), `crates/web` +
  `crates/acme` env-read migration, `docs/ops` env tables. Out of
  scope: admin routers (admin-workers stream), voice (voice-telnyx
  stream), ingest, form-framework stream, software-TPM sealing
  (tpm-seal stream), web/ frontend build-time env.

## 1. Startup order

```text
open store (STORE_DIR bootstrap) → run migrations → load KEK
  → read runtime config from sealed DB rows → serve
```

Concretely in `crates/web/src/main.rs`: `STORE_DIR` (bootstrap)
locates the store; `store.init` + `db::migrate` (+ P2/auth/
config migrations) run; `config::load_kek` loads the KEK and the
process **refuses to start** when no KEK is available (new —
runtime config is unreadable without it, and silently serving on
dev defaults would bind localhost in prod); one
`RuntimeConfig::load` reads all 31 keys; listeners serve from the
loaded values. No runtime env fallback exists by design.

Restart semantics: config is read once at boot. Every admin PUT
takes effect on the next restart (a live-reload design is a
follow-up, see OQ-C4).

## 2. Full env-var inventory and disposition

### 2.1 Migrated to DB keys (31 keys)

| Env var | DB key | Type | Default (= old env default) |
|---|---|---|---|
| `STATIC_DIR` | `paths.static_dir` | text | `crates/web/static` |
| `SPA_DIR` | `paths.spa_dir` | text | `web/build` |
| `ADMIN_SPA_DIR` | `paths.admin_spa_dir` | text | `web/build-admin` |
| `ACME_CHALLENGE_DIR` | `paths.acme_challenge_dir` | text | `acme` |
| `BIND` | `net.bind` | text (IP) | `127.0.0.1` |
| `BIND6` | `net.bind6` | text (IP, empty = unset) | empty |
| `PORT` | `net.port` | text (u16) | `3000` |
| `ADMIN_PORT` | `net.admin_port` | text (u16) | `3001` |
| `HTTPS_PORT` | `net.https_port` | text (u16) | `443` |
| `HTTP_PORT` | `net.http_port` | text (u16) | `80` |
| `PUBLIC_HOST` | `net.public_host` | text | `andrii.ai` |
| `TLS_CERT` | `tls.cert_file` | text (path, empty = unset) | empty |
| `TLS_KEY` | `tls.key_file` | text (path, empty = unset) | empty |
| `ADMIN_MTLS=off` | `admin.mtls_off` | bool | false |
| `MEDIA_TERMS_READ=1` | `media.terms_read` | bool | false |
| `ACME_ENABLED=1` | `acme.enabled` | bool | false |
| `ACME_DIRECTORY` | `acme.directory` | text (`prod`/`staging`/URL) | `prod` |
| `ACME_DOMAINS` | `acme.domains` | text (CSV) | `andrii.ai,admin.andrii.ai,www.andrii.ai` |
| `ACME_EMAIL` | `acme.email` | text (empty = none) | empty |
| `ACME_EAB_KID` | `acme.eab_kid` | text (public ID, safe to log) | empty (= missing) |
| `ACME_EAB_HMAC` | `acme.eab_hmac` | **secret** (base64) | required when enabled |
| `USPS_BASE_URL` | `usps.base_url` | text (URL) | `https://apis-tem.usps.com` |
| `USPS_CERTIFIED=1` | `usps.certified` | bool | false |
| *(voice stream)* | `telnyx.base_url` | text (URL) | `https://api.telnyx.com` |
| *(voice stream)* | `telnyx.from_number` | text (E.164, empty = unset) | empty |
| *(voice stream)* | `telnyx.connection_id` | text (empty = unset) | empty |
| *(voice stream)* | `telnyx.webhook_public_key` | text (empty = unset) | empty |
| *(voice stream)* | `telnyx.webrtc_connection_id` | text (empty = unset = real dials 503) | empty |
| *(voice stream)* | `telnyx.api_key` | **secret** | unset (= voice 503) |
| *(quic stream)* | `quic.enabled` | bool | false |

Shapes are exactly secure-config.md §1.5 (`bool`/`text`/
`secret`); ports ride as text and parse to u16 at load. Missing
row ≡ default. `acme.eab_kid` is text, not secret: it is a public
identifier (logged at ACME registration, per
`crates/acme/src/config.rs`), while the HMAC stays write-only.
This deviates from the `acme.md` follow-up note ("both secrets");
all rows are TPM-sealed ciphertext either way.

### 2.2 Bootstrap-kept (still env, with reason)

| Env var | Where | Reason it cannot live in DB |
|---|---|---|
| `STORE_DIR` | `crates/web` main | Store path resolution: locates the DB itself (chicken-and-egg). The one sanctioned bootstrap var. |
| `CREDENTIALS_DIRECTORY` | `config::load_kek` | systemd delivery path for the TPM-sealed KEK; needed to decrypt any row. Owned by the tpm-seal stream. |
| `HANDBOOK_DEV_KEYFILE` / `HANDBOOK_DEV_KEK` | `config::load_kek` | Dev-only KEK sources (debug builds, loud banner, refused in release). No DB exists before the KEK. |
| `HANDBOOK_KEK_FILE` | `xtask config-*` | `--kek-file` alternative for offline enroll/set/verify/rotate; the tool that writes sealed rows cannot read its key from them. |
| `RUST_LOG` | tracing `EnvFilter` | Process-level log control, initialized before the store opens. Standard tracing convention. |
| `STATIC_DIR`/`SPA_DIR`/`ACME_CHALLENGE_DIR` in `#[cfg(test)]` | web unit tests | Test-harness fixture overrides, never read in production paths. |
| `env!` (`CARGO_PKG_VERSION`, `CARGO_MANIFEST_DIR`) | various | Compile-time constants, not runtime settings. |
| `std::env::temp_dir()` | forms tests | OS temp dir in test helpers, not configuration. |

### 2.3 Documented but never read (no code change)

- `META_API_KEY`: nothing reads it (model client is Stub-only);
  reserved as future secret `meta.api_key` when the live client
  lands. Not implemented here (no reader exists to migrate).
- `STORE_PATH`/`BIND_ADDR`/`BASE_PATH` (`SECURITY.md`,
  `web-stack.md`): aspirational names; code uses
  `STORE_DIR`/`BIND`/fixed `/apostille`. Doc cleanup is a
  follow-up (OQ-C5).
- Carrier/Stripe keys: sibling streams define their
  `carrier.*` / `stripe.*` namespaces when readers land
  (`telnyx.*` already registered — see §2.1).
- `web/` frontend env: build-time Vite surface, separate from
  process runtime config; out of scope.

## 3. Load semantics (equivalence + deliberate deltas)

No behavior change when the DB holds the old defaults. Per-row
rules in `RuntimeConfig::load`:

- Missing row → default, silent (fresh-install state).
- Present, decrypts, parses → value.
- Present, decrypts, wrong type or unparsable (ports, IPs) →
  default + loud stderr warning (mirrors the old `port_env`
  garbage→default leniency).
- Decrypt failure on any runtime row → **refuse to start**
  (wrong KEK or tamper; matches the fail-closed crypto
  philosophy — never silently default on crypto errors).
- DB error mid-load → refuse to start.
- Empty text ≡ unset for optional values (`net.bind6`,
  `acme.email`, `tls.*`, `acme.eab_*`). For required values
  (`net.public_host`, paths, `usps.base_url`, `acme.domains`)
  empty → default + warning.
- TLS both-or-neither and ACME builder validation are unchanged
  (same refusal paths, now fed from DB values).
- `admin.mtls_off=true` in a release build → **refuse to start**.
  This bakes the H6 fix (`security-review-2026-10-27.md`) into the
  migration: a persistent dev-bypass flag must not survive into a
  release binary. Deliberate delta, no env equivalent.
- KEK unavailable → **refuse to start** (old code booted degraded
  with env config + 503 on the config API). Deliberate delta:
  without the KEK no runtime setting is readable, and degraded
  boot would silently bind localhost in prod. **Prod cutover is
  therefore gated on the tpm-seal stream's release-usable KEK
  source plus enrollment** (the current box has no TPM; see §5).

Micro-deltas at pathological states no sane deploy uses
(empty-string env values previously produced empty hosts/paths or
refused TLS; the DB rule is uniform empty≡unset/default). All are
covered by unit tests in the `config` crate.

## 4. Admin UI coverage (GET/PUT, restart-required)

The admin config API (`GET /config`, `PUT
/config/{key}`) is generic over all keys — no per-key code
was needed or added. Each key renders as:

| Keys | Admin control | Notes |
|---|---|---|
| `*.port`, `net.bind`, `net.bind6`, `net.public_host` | text input | u16/IP validated at next boot, not at PUT time |
| `paths.*`, `tls.*`, `acme.directory`, `acme.domains`, `acme.email`, `acme.eab_kid`, `usps.base_url` | text input | paths/URLs validated at next boot |
| `admin.mtls_off`, `media.terms_read`, `acme.enabled`, `usps.certified` | checkbox | `admin.mtls_off` refuses release boot when true |
| `acme.eab_hmac` | set-new secret input, presence-only read | write-only; never echoed |

Every key is restart-required (read-once at boot). The
admin-workers stream owns the rendered UI; it should label each
row "takes effect on restart" and surface the boot warnings
(`journalctl -u apostille`) next to the form.

## 5. Migration / compat plan

No runtime env fallback exists (that would defeat zero-env).
Migration is a one-time, operator-run `config-set` sequence
before the first boot of the new binary:

```sh
KEK=/path/to/kek-file STORE=/var/lib/apostille
set() { cargo xtask config-set --store "$STORE" --kek-file "$KEK" \
  --key "$1" --type "$2" --value "$3"; }
# From the current apostille.service Environment= lines:
set net.bind text <server IPv4>
set net.bind6 text <server IPv6>
set paths.spa_dir text /srv/apostille/current/spa
set net.public_host text andrii.ai
set tls.cert_file text /var/lib/apostille/tls/cert.pem
set tls.key_file text /var/lib/apostille/tls/key.pem
set paths.acme_challenge_dir text /var/lib/apostille/acme
# In-binary ACME deployments only (EAB via stdin, never argv):
cargo xtask config-set --store "$STORE" --kek-file "$KEK" \
  --key acme.eab_hmac --type secret --value-from-stdin < hmac.txt
set acme.eab_kid text '<kid>'
set acme.enabled bool true
cargo xtask config-verify --store "$STORE" --kek-file "$KEK"
```

Then ship the new binary + the trimmed unit file (keeps only
`STORE_DIR`) and restart. Staging/dev: defaults match the old
dev boot (`127.0.0.1:3000/3001`), so no seeding is needed beyond
a KEK. Rollback: previous binary + previous unit file (both in
git); the old binary re-reads env and ignores the new rows.
`STORE_DIR` stays env forever (§2.2).

Gates before prod cutover: (a) tpm-seal KEK source enrolled on
the box (no-TPM box cannot load a release KEK today — without it
the new binary refuses to start, by design); (b) `config-verify`
green; (c) boot smoke on the target box.

## 6. Acceptance criteria

- [ ] `cargo xtask ci` green (fmt, clippy `-D warnings`, tests).
- [ ] Static audit: `std::env::var` in non-test `crates/web`,
  `crates/acme`, `xtask` reads only §2.2 bootstrap vars.
- [ ] Unit: `RuntimeConfig::default` equals every old env default;
  missing rows yield defaults; garbage ports/IPs warn + default;
  wrong-KEK decrypt refuses; every registered key roundtrips
  through `set_value` + typed read.
- [ ] Boot smoke on port 0 (DB `net.port=0`, `net.admin_port=0`)
  with hostile non-bootstrap env set (`PORT`, `BIND`,
  `ADMIN_MTLS=off`, `ACME_ENABLED=1`, …): boots, binds
  127.0.0.1 + ephemeral ports, healthz 200 — proving env reads
  are gone behaviorally, not just by grep.
- [ ] Admin GET lists and PUT sets every §2.1 key (generic API;
  proven by the roundtrip test, no admin.rs changes).
- [ ] `docs/ops` env tables updated (`acme.md`, `deploy.md`,
  `parity.md`, `apostille.service`); no dangling
  references to migrated vars as live inputs.

## 7. Open questions

- OQ-C1 (owner: tpm-seal stream; blocks: prod cutover): what is
  the release-usable KEK source on the no-TPM box, and what is
  the exact enroll + unit-file change? config-db refuses to start
  without a KEK.
- OQ-C2 (owner: author; blocks: none): prod `paths.static_dir`
  was never shipped (ship script copies only `web` + `spa`), so
  the default `crates/web/static` 404s in prod today. Ship the
  dir, or drop the static path? Unchanged by this migration.
- OQ-C3 (owner: admin-workers stream; blocks: none): admin UI
  should label every config row restart-required and link boot
  warnings; confirm ownership.
- OQ-C4 (owner: parent; blocks: none): live-reload follow-up —
  which keys (if any) should become reloadable without restart?
  Proposed: none for the slice.
- OQ-C5 (owner: parent; blocks: none): refresh `SECURITY.md` /
  `web-stack.md` env-config sections (`STORE_PATH`/`BIND_ADDR`/
  `META_API_KEY`-in-env) to point at this spec. Left untouched
  as non-ops docs outside this stream's scope.

## 8. Admin IP allowlist (replaces client-cert auth)

Client certificates are no longer requested at the handshake and
no longer checked anywhere: admin access is the session cookie +
staff role + a peer IP in `admin.allow_ips` (DB key, Text,
comma-separated CIDRs or bare host IPs, e.g.
`203.0.113.0/24, 198.51.100.7, 2001:db8::/32`). Proxy headers are
never trusted — the admin ports are reached directly, so the
socket peer is the caller. No peer address fails closed.

- Default (key unset): loopback only (`127.0.0.0/8`, `::1`), so
  first boot can still reach setup over localhost.
- Invalid value: warning + loopback only (fail closed, never the
  previous value, never open).
- Restart-required like every key (read-once at boot). Renders as
  a text input in the generic admin config UI.
- Changing it: `cargo xtask config-set --store <store> --kek-file
  <kek> --key admin.allow_ips --type text --value "<cidrs>"`,
  then restart. On production the store is `/var/lib/apostille`
  (see the `ops-gp2-proposals.sh` KEK-staging pattern). Set the
  range BEFORE the deploy lands (old binaries ignore the unknown
  row; the new binary reads it at boot) — deploying with the
  default locks every non-loopback admin out until it is set.
- Denied callers get HTTP 403 naming their detected IP and this
  section: `admin access is limited to the configured IP range
  (admin.allow_ips) — your IP is <ip>; see "Admin IP allowlist"
  in docs/specs/config-db.md to change it`.
- IPv6: list the /64, never one address. Privacy extensions rotate
  the source address per connection, so a single allowlisted IPv6
  address passes one fetch and 403s the next; the denial names the
  /64 to add. IPv4 privacy is per-network, so a bare address (or
  its /24) is stable.
- Dual-stack surprise: the admin host serves A + AAAA (+ HTTP/3),
  so a browser can arrive over IPv6 or QUIC with a different source
  address than the IPv4 you tested. Every denial names the family
  (IPv4/IPv6) and the transport (tcp/quic) — add whichever it shows.
- First boot from anywhere but loopback: set `admin.allow_ips`
  with `xtask config-set` BEFORE opening `/config` — the setup
  surface (page, assets, setup API) is gated by the same list, and
  a refused browser gets a static 403 page (no SPA, no JS) naming
  its IP, family, transport, and these fix steps. API paths keep
  the JSON denial (`code: ip_not_allowed`); `/_app/` assets get a
  bare 403.
- Removed from the registry (P373): `tls.client_ca_file`,
  `admin.require_cert` and `net.admin_https_port`
  (`config::keys::REMOVED`). A stored row is ignored: the boot
  prints one warning, the admin list leaves it out and the admin
  PUT refuses the key. The cert half of `admin.mtls_off` is gone
  too (the flag still gates the loopback-host and test-header dev
  seams). Enrolment ceremonies no longer require a presented
  cert; a presented live cert still binds when present.

## 9. Fake mode (W-3)

Every outside service has one `<area>.mode` row (text): `fake`,
`sandbox` or `live`. A fake is a local stand-in: nothing is sent,
shipped or charged, and its output is marked as fake.

| Row | Integration | `fake` means |
|---|---|---|
| `mailer.mode` | outgoing email (magic links, digests, outreach) | kept in the store's fake outbox (admin **Outbox**), never sent |
| `mail.mode` | incoming email (Gmail poll) | no poll; admin **Compose** injects mail through the poller's capture path |
| `labels.mode` | FedEx and USPS labels | demo PDF stamped "DEMO — NOT POSTAGE", fake-range tracking number |
| `stripe.mode` | Stripe checkout + webhooks | local checkout, DEMO receipt, webhook signed with a local secret |
| `telnyx.mode` | Telnyx calls | a recorded WAV plays into the transcribe step |
| `meta.mode` | Meta Model API | the call cache in `replay-only` mode over the recorded corpus |
| `hf.mode` | Hugging Face weights | no download; Glimmer verdicts from recorded files |

Rules (`config::Modes`, `config::fakes_allowed`):

- Unset ≡ today's behaviour, exactly. `sandbox` and `live` never
  override the switches that already choose between them
  (`labels.live`, `labels.customer_live`, `usps.live`, the Stripe
  key's `sk_test_`/`sk_live_` prefix, `mailer.provider`): the mode
  row can add a fake, never raise anything to live. A value other
  than the three warns and counts as unset.
- **Guard.** Any `fake` row refuses the boot unless every bind IP is
  loopback (the dev-seam rule, in every build profile) or the boot
  is a staging target: `APOSTILLE_ENV_LABEL=staging` (set only by
  the staging boot) with a `net.public_host` other than `andrii.ai`.
  Production can never run a fake.
- The admin config PUT and the setup-wizard PUT refuse to save a
  `fake` row while this boot's guard is closed (422), so a
  production console cannot save a row its next boot would refuse.
- The boot prints `FAKE MODE — <integrations>`; `GET
  /admin/version` lists them as `fakes`, and the console shows a
  FAKE ribbon naming each one on every screen.

Each fake in detail: email §9.1; labels `docs/specs/carrier-apis.md`
§6; Stripe `docs/specs/payments-billing.md` §13; Telnyx
`docs/specs/voice-telnyx.md` (Fake calls); Meta and Hugging Face §9.2.
`cargo xtask up` and `cargo xtask judge` seal every row to `fake`
while it is unset (and `orders.enabled` on), except `meta.mode` on a
`--live` or `--record` boot, which calls the model.

### 9.1 Fake email (W-3b)

- **Outbox** (`mailer.mode = fake`): `magic::mailer_from_config`
  returns `Mailer::Outbox` whatever `mailer.provider` says. Every
  send — magic links, follower digests, first-source outreach —
  becomes a `fake_outbox` row (`CREATE TABLE IF NOT EXISTS`, metadata
  DB) with its closed header set (RFC 8058 pair, `Message-ID`,
  `Reply-To`). Nothing is dialled, so no real address is reached and
  the overall send budget is untouched. Staff read it at **Outbox**
  (`GET /apostille/api/v1/admin/fake/outbox`, newest 200); the rows
  hold live sign-in links, which is the point on a loopback or staging
  boot with synthetic data.
- **Inbox** (`mail.mode = fake`): `xtask mail-poll` exits without
  calling Gmail. **Compose** (`POST /apostille/api/v1/admin/fake/inbox`,
  409 unless the mode is fake) takes From, subject, body, an optional
  `In-Reply-To`, `dmarc: pass|fail` and optional attachments, builds
  the RFC 2822 bytes Gmail would hand the poll — including the
  `Authentication-Results: mx.google.com; …` stamp with the chosen
  DMARC/DKIM/SPF verdict — and runs them through
  `mail::poll::capture_message`, the poll's own capture step. Trust is
  therefore decided exactly as for polled mail (P435): `official` only
  when the stamp authenticates an official From domain; a DMARC fail
  caps at `community`; unknown domains land in `mail-inbox`. A reply
  naming an outbox `Message-ID` links to its outreach email. The
  capture is watermarked (`X-Handbook-Fake` header, `fake-inbox:<id>`
  origin, `received_via: fake-inbox`, `fake: true`) and the next
  `xtask instruct-sweep` drafts from it into `/review` unchanged.

### 9.2 Fake Meta Model API and Hugging Face (W-3e)

- `meta.mode = fake` **is** the existing call cache in `replay-only`
  mode (`config::call_cache_rows` reports it whatever
  `meta.cache_mode` says): answers come from the recorded corpus under
  `paths.meta_cache_dir`, a request with no recorded answer is an
  error, and no model is called. As for every sealed cache row,
  `$META_CACHE_MODE` and `--cache-mode` still override it, and the
  mode covers the URL fetch cache too (`docs/ops/call-cache.md`).
- `hf.mode = fake`: no Glimmer weights are downloaded (nothing runs
  `ops/open-weights/glimmer-serve.sh download`) and verdicts come from
  the recorded files in `evals/captures/demo_precheck/`: the public
  demo precheck serves them from the store (`up` installs them), and
  the plant `precheck.glimmer` worker answers from them when its
  `worker.env` carries `HF_MODE=fake` — the worker reads no server
  config, so `up` writes that line while `hf.mode` is fake.
