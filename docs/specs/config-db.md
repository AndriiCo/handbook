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
`RuntimeConfig::load` reads all 29 keys; listeners serve from the
loaded values. No runtime env fallback exists by design.

Restart semantics: config is read once at boot. Every admin PUT
takes effect on the next restart (a live-reload design is a
follow-up, see OQ-C4).

## 2. Full env-var inventory and disposition

### 2.1 Migrated to DB keys (29 keys)

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
| `ADMIN_HTTPS_PORT` | `net.admin_https_port` | text (u16) | `444` |
| `HTTP_PORT` | `net.http_port` | text (u16) | `80` |
| `PUBLIC_HOST` | `net.public_host` | text | `andrii.ai` |
| `TLS_CERT` | `tls.cert_file` | text (path, empty = unset) | empty |
| `TLS_KEY` | `tls.key_file` | text (path, empty = unset) | empty |
| `ADMIN_MTLS=off` | `admin.mtls_off` | bool | false |
| `MEDIA_TERMS_READ=1` | `media.terms_read` | bool | false |
| `ACME_ENABLED=1` | `acme.enabled` | bool | false |
| `ACME_DIRECTORY` | `acme.directory` | text (`prod`/`staging`/URL) | `prod` |
| `ACME_DOMAINS` | `acme.domains` | text (CSV) | `andrii.ai,admin.andrii.ai` |
| `ACME_EMAIL` | `acme.email` | text (empty = none) | empty |
| `ACME_EAB_KID` | `acme.eab_kid` | text (public ID, safe to log) | empty (= missing) |
| `ACME_EAB_HMAC` | `acme.eab_hmac` | **secret** (base64) | required when enabled |
| `USPS_BASE_URL` | `usps.base_url` | text (URL) | `https://apis-tem.usps.com` |
| `USPS_CERTIFIED=1` | `usps.certified` | bool | false |
| *(voice stream)* | `telnyx.base_url` | text (URL) | `https://api.telnyx.com` |
| *(voice stream)* | `telnyx.from_number` | text (E.164, empty = unset) | empty |
| *(voice stream)* | `telnyx.connection_id` | text (empty = unset) | empty |
| *(voice stream)* | `telnyx.webhook_public_key` | text (empty = unset) | empty |
| *(voice stream)* | `telnyx.api_key` | **secret** | unset (= voice 503) |

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

The admin config API (`GET /admin/config`, `PUT
/admin/config/{key}`) is generic over all keys — no per-key code
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
set net.bind text 199.184.91.41
set net.bind6 text 2602:f337:ed:4841:4c07:3930:3030:9000
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
  `parity.md`, `drill.md`, `apostille.service`); no dangling
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
