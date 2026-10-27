# TPM-Sealed Encrypted Config Storage (draft for review)

- Status: draft (prompt 010, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child
  session log). Reads: `README.md`; `docs/design.md` §0–§1;
  `docs/SECURITY.md` §Secrets; `docs/web-stack.md` §1.
- Goal: all config values live in SQLite **ENCRYPTED**. An admin
  page manages typed keys: checkbox on/off; text (IDs/names/
  addresses/paths); `SECRET` keys that are **write-only** —
  set-new-only, never retrievable via admin page or API.
  Decryption must fail on any other machine **and** for any other
  OS user on the same server.

## 1. Recommended architecture (one)

**Envelope encryption in SQLite + KEK protected by TPM2 via
systemd credentials + OS-enforced user boundary.**

```text
┌─ provisioning (once, on server, as root) ─────────────┐
│ xtask config-enroll                                    │
│  1. rand KEK-1 (32 B, CSPRNG)                          │
│  2. systemd-creds encrypt --with-key=host+tpm2         │
│       --name=handbook-kek → /etc/credstore.encrypted/  │
│       handbook-kek-v1.cred (0600 root:root)            │
│  3. KEK-1 escrow backup → offline (see §6)             │
└───────────────────────────────────────────────────────┘
┌─ runtime (systemd unit, User=handbook) ───────────────┐
│ LoadCredentialEncrypted=handbook-kek:…v1.cred         │
│  → systemd decrypts via TPM2 (+host key) at start     │
│  → plaintext KEK only in service-private tmpfs        │
│     $CREDENTIALS_DIRECTORY/handbook-kek               │
│ app reads KEK once, mlocks, decrypts config rows      │
│ with XChaCha20-Poly1305; zeroizes on shutdown         │
└───────────────────────────────────────────────────────┘
```

### 1.1 Why systemd-creds, not hand-rolled TPM code

`systemd-creds encrypt` with `--with-key=tpm2` (or `host+tpm2`)
seals the credential so it decrypts only on the same TPM2 chip
(and, with `host+`, only with the same OS installation's
`/var/lib/systemd/credential.secret`). This is exactly our machine
boundary, implemented and maintained by systemd instead of by us.
`LoadCredentialEncrypted=` decrypts at service activation and
exposes plaintext only inside a per-service tmpfs visible solely
to that unit (`$CREDENTIALS_DIRECTORY`). Our Rust code stays
pure-Rust: no C TPM library, no new system build dependency on
either Mac or Ubuntu — the only server-side requirement is systemd
with TPM2 support (verify at provisioning; Ubuntu ships systemd
with TPM2 enabled) plus a TPM2 device. This respects `web-stack.md`
§1 (pure-Rust deps, `rustup + git clone` build) with one sanctioned
runtime exception (OS-level tooling: systemd, certbot).

### 1.2 Precise statement of the two boundaries

- **Machine boundary — enforced by the TPM2 chip.** The `.cred`
  file is ciphertext. Decrypting it requires the TPM2 sealed key,
  which never leaves the chip. Copy the SQLite file, the `.cred`
  file, or the whole disk to another machine → decryption fails.
  (`host+tpm2` additionally binds to the OS install's host key, so
  a disk transplanted into an identical-chassis attacker box with a
  *different* TPM still fails twice over.)
- **User boundary — enforced by the OS kernel, NOT by the TPM.** A
  TPM chip has no notion of OS UIDs: any process that can reach
  `/dev/tpmrm0` **and** possesses the sealed blob can unseal it.
  So the "any other OS user" requirement is enforced by three
  kernel layers: (1) the `.cred` file is `0600 root:root` — only
  root/systemd reads it, at unit start; (2) the decrypted KEK is
  delivered only into the service unit's private credential tmpfs,
  visible exclusively to processes in that unit's mount namespace
  running as `User=handbook`; (3) defense in depth: SQLite file
  and `store/` are `0600/0700 handbook:handbook`. Another
  non-root user gets neither the blob nor the KEK.
- **Explicit non-goal: root is trusted.** Root can read the tmpfs,
  the TPM device, and process memory. No software design on this
  box excludes root; this matches `SECURITY.md`'s existing
  "root-only env file" trust model. Document it; don't pretend
  otherwise.

### 1.3 Envelope encryption

- **Algorithm: XChaCha20-Poly1305** (RustCrypto
  `chacha20poly1305`, pure Rust, NCC-audited family). 24-byte
  random nonce per value → random-nonce reuse is a non-event (~2⁸⁰
  safe messages vs ~2³² for AES-GCM with random nonces); no
  hardware AES needed for good performance; identical AEAD API
  either way.
- **Envelope:** one KEK per generation (v1, v2, …); each config row
  stores `nonce(24) ‖ ciphertext`. No per-row DEKs — unjustified
  complexity at this row count (tens of keys); KEK rotation
  re-encrypts all rows (§5). AEAD AAD = `key_ascii ‖ enc_version`
  (binds ciphertext to its key name and KEK generation; kills
  row-swap/cut-and-paste).
- **In-memory:** KEK held in a `zeroize::Zeroizing<[u8;32]>`, read
  once at startup; `mlock` best-effort; never logged, never in
  error strings, never in `/debug/*` (per `SECURITY.md` §Secrets
  rules, which stay in force).

### 1.4 Schema (new migration)

```sql
CREATE TABLE config(
  key         TEXT PRIMARY KEY,          -- e.g. 'fedex.account_id'
  type        TEXT NOT NULL              -- 'bool' | 'text' | 'secret'
    CHECK(type IN ('bool','text','secret')),
  value_enc   BLOB NOT NULL,             -- nonce(24) || XChaCha20-Poly1305 ct
  enc_version INTEGER NOT NULL,          -- KEK generation
  updated_at  INTEGER NOT NULL,          -- unix seconds
  updated_by  TEXT NOT NULL              -- audit actor
) STRICT;
CREATE TABLE config_kek(
  version    INTEGER PRIMARY KEY,        -- 1, 2, …
  cred_path  TEXT NOT NULL,              -- sealed .cred path for this gen
  created_at INTEGER NOT NULL,
  retired_at INTEGER                     -- NULL = active
) STRICT;
```

Notes: tables live in `handbook.sqlite` (one DB, one backup;
rows are ciphertext); `STRICT` tables (new-code default, cf.
finding F5 in `SECURITY.md`); `bool` plaintext is one byte `0/1`
validated after
decrypt; `text` is UTF-8, length-capped (4 KiB); `secret` max 8
KiB. Every insert/update writes an `audit` row (`entity='config'`,
`before_ref`/`after_ref` = key+version only, **never values**).

### 1.5 Admin page + API semantics (SECRET write-only, enforced in code structure)

Base `/apostille/api/v1`, admin listener + admin origin only
(`admin.andrii.ai`): all routes below require the admin session,
mTLS cert, and staff/admin role.

- `GET /apostille/api/v1/admin/config` returns all keys with: `bool` →
  decrypted `true/false`; `text` → decrypted string; `secret` →
  `{ "set": true|false }` **only**. The admin handler has **no
  code path** that decrypts a `secret` for output — decryption for
  display is a function that takes only `&BoolOrTextRow` types, so
  returning a secret is a compile error, not a policy.
  (Type-level enforcement, reviewable in one file.)
- `PUT /apostille/api/v1/admin/config/{key}` sets a value (any type). For
  `secret`: set-new-only — overwrites ciphertext, never echoes;
  response is `{ "set": true }`. No `DELETE` for secrets (disable
  via a paired `bool` key instead, e.g.
  `mailer.enabled=false`); prevents "delete then read the backup"
  games.
- Runtime reads of secrets (e.g. `META_API_KEY` for the transcribe
  job) happen **only** in server-side job code with direct DB+KEK
  access — never through the admin API layer. This migrates
  `META_API_KEY` out of env (superseding `SECURITY.md` "env-only"
  for app secrets; keep env-only for
  `STORE_PATH/BIND_ADDR/PORT/BASE_PATH` bootstrap vars).
- Auth on these routes: existing admin session + mTLS + role
  checks per `SECURITY.md` §1; secret-set requires step-up
  (re-auth within 5 min) — cheap, high value.

## 2. Rejected alternatives (with reasons)

1. **Raw tpm2-tools sealed objects + hand-rolled PCR policy
   (`tpm2_createprimary → tpm2_pcrread → tpm2_createpolicy →
   tpm2_create -L → tpm2_load → tpm2_unseal -p pcr:…`).** Viable
   and well-documented, but we would own PCR selection,
   policy-session plumbing, re-enrollment on firmware/Secure Boot
   change, and subprocess-or-FFI invocation. systemd-creds does
   exactly this, is maintained upstream, and additionally gives us
   the per-service credential tmpfs (a stronger user boundary than
   file perms alone). **Keep as documented fallback** if the
   provisioned box lacks systemd TPM2 support: `xtask` already
   shells out via `std::process`, so a `tpm2_unseal`-at-startup
   path stays Rust-only at build time.
2. **In-process TPM via Rust `tss-esapi` (Parallax Second, stable
   7.7.0, Apr 2026; 8.0.0-alpha in flight).** Mature project, real
   `Context::unseal()` API — **rejected** because it links the C
   `tpm2-tss` library (`tss-esapi-sys`), which breaks `web-stack.md`
   §1's pure-Rust/no-system-libs parity rule (Mac dev vs Ubuntu
   prod), complicates builds, and has thin docs (~18% documented
   items). It also puts TPM session code in the request-serving
   process instead of at the systemd boundary.
3. **TPM authValue/PIN per service user as the user boundary.**
   TPM authorization (password/HMAC/session) is real, but it
   authenticates *to the chip*, and the PIN must live somewhere
   the service can read unattended — i.e. a file with exactly the
   same DAC boundary as the sealed blob. It adds a second secret
   with an identical trust perimeter and zero marginal security
   against OS users, plus PIN-management complexity (systemd's
   `--tpm2-with-pin` exists for interactive unlock, not
   unattended services). **Rejected:** file perms + credential
   tmpfs *are* the user boundary; say so precisely instead of
   adding theater.
4. **AES-256-GCM instead of XChaCha20-Poly1305.** Same RustCrypto
   audit pedigree, but 12-byte nonces demand counter discipline to
   stay safe past ~2³² messages per key, and it leans on AES-NI for
   speed. XChaCha's 24-byte random nonces make every encryption
   independently safe with zero state. Marginal either way at our
   volume; XChaCha is the lower-footgun default.
5. **File-key-only on the server (KEK in a 0600 file, no TPM).**
   Enforces the user boundary but **no machine boundary**: whoever
   copies the disk copies the key. Fails a stated requirement.
   (File keys are acceptable only as the Mac dev fallback, §7,
   where there is no TPM and no real data.)
6. **Per-row DEKs / full-disk-encryption-instead.** Per-row DEKs
   add a key table and rotation matrix for tens of rows —
   complexity without a threat-model win. FDE protects data at
   rest when powered off, but our attacker model includes a
   *running* server's other users and a stolen *copy* of the DB —
   FDE answers neither. (Still enable FDE; it just isn't this
   design.)

## 3. Key rotation

- **Planned rotation:** `xtask config-rotate --actor=…` (run as
  root on server): generate KEK-(n+1) → `systemd-creds encrypt` →
  new `.cred` → `systemctl` delivers both gens (old + new
  credentials simultaneously) → app re-encrypts every row inside
  one SQLite txn (`enc_version` bumped; AAD binds new version) →
  mark old KEK retired → remove old credential → restart. Old
  `.cred` deleted only after a verified boot+decrypt on the new
  gen.
- **Emergency rotation (suspected KEK exposure):** same flow, plus
  rotate every `secret` *value* it protected (KEK rotation
  re-wraps; value rotation replaces — do both, values via admin
  page), plus audit rows and offsite audit ship per `SECURITY.md`
  §1.8.
- **Cadence:** yearly planned rotation; on every operator
  departure with root access; immediately on any suspected leak.

## 4. Backup / DR

- **What is backed up:** (a) SQLite file + `store/` (existing
  backup question, `SECURITY.md` Q8) — config rows are ciphertext,
  safe to store alongside; (b) `.cred` files — ciphertext,
  machine-bound, backed up with the OS image for convenience but
  **useless without that TPM**; (c) **KEK escrow** — the only
  DR-critical secret: each active KEK exported once at
  enroll/rotate, encrypted to *two* admin public keys (age or PGP,
  2-of-2 custodians), stored offline in two locations (e.g.
  encrypted USB in safe + bank box). Escrow access is itself an
  audited, two-person act.
- **Restore on same box (disk failure):** reinstall OS → restore
  DB + `.cred` → TPM chip unchanged → boot, systemd decrypts,
  done.
- **TPM-loss / new machine (re-enrollment):** (1) provision new
  host + TPM; (2) two custodians decrypt escrowed KEK in an
  air-gapped step; (3) `xtask config-enroll --kek-from-stdin`
  seals it to the *new* TPM (`--with-key=host+tpm2`); (4) restore
  DB; (5) verify `xtask config-verify` (decrypt-all + AAD check,
  values never printed); (6) rotate to a fresh KEK at the next
  maintenance window (the escrowed KEK touched human hands —
  hygiene). **RTO:** < 1 h given custodians; **RPO:** last DB
  backup.
- **Never:** escrow in the repo, in the content repo, in cloud
  notes, or in chat. Escrow ciphertext may live next to backups
  only if admin private keys never touch the server.

## 5. Provisioning checklist (server)

1. Confirm TPM2 (`/dev/tpmrm0` present), `systemd-creds --help`
   shows `tpm2` key type; record `systemd --version` and PCR
   policy in the runbook.
2. `install -d -m 0700 /etc/credstore.encrypted`; `xtask
   config-enroll` as root; verify `0600 root:root` on `.cred`.
3. Unit file: `User=handbook`,
   `LoadCredentialEncrypted=handbook-kek:…`,
   `ProtectSystem=strict`, `PrivateTmp=yes`,
   `NoNewPrivileges=yes`; `store/` `0700 handbook:handbook`.
4. `xtask config-verify` (decrypt-all self-test, no value output)
   green before first traffic; negative test: `sudo -u nobody
   cat` on `.cred` and on `$CREDENTIALS_DIRECTORY` must fail; DB
   copy to a second VM must not decrypt (documents the machine
   boundary).

## 6. Local-dev fallback (Mac, no TPM) — loud by design

- **Mechanism:** `keyring` crate (stable 4.x) storing the dev KEK
  in the macOS Keychain (`service="handbook-dev"`, never the prod
  name). Pure Rust API, platform-native store, no new system dep.
- **Loudness (all mandatory):** (a) every `cargo xtask dev` boot
  prints a banner: `DEV CRYPTO — Keychain-backed dev KEK, NOT
  TPM-sealed, synthetic data only`; (b) dev KEKs carry a `dev:`
  prefix and the server binary **refuses to start** if a
  dev-marked KEK is presented outside `cfg(debug_assertions)`; (c)
  `config-verify` fails the boot if `STORE_PATH` points outside
  tmp/seed dirs while in dev-crypto mode; (d) all seeded secrets
  are synthetic (`dev-seed` only — consistent with `web-stack.md`
  §1).
- **File-key escape hatch:** `HANDBOOK_DEV_KEYFILE` (0600) only if
  Keychain is unavailable (headless CI); same loud banner + same
  prod refusal. CI never holds real secrets.

## 7. Sources

- tpm2-tools seal/unseal + PCR policy flow: [tpm2_unseal(1)](https://manpages.ubuntu.com/manpages/oracular/man1/tpm2_unseal.1.html),
  [TPM tools tutorial](https://joholl.github.io/tpm2-tools/tutorial/2019/10/09/Tools-Tutorial.html),
  [TPM auth forms (password/HMAC/session)](https://man.archlinux.org/man/extra/tpm2-tools/tpm2_create.1.en)
- Rust TPM crate maturity: [tss-esapi 7.7.0 on docs.rs (stable;
  8.0.0-alpha in flight)](https://docs.rs/crate/tss-esapi/latest),
  [parallaxsecond/rust-tss-esapi](https://github.com/parallaxsecond/rust-tss-esapi)
- systemd credentials: [systemd-creds(1) —
  `--with-key=tpm2|host|host+tpm2|auto`](http://man7.org/linux/man-pages/man1/systemd-creds.1.html),
  [Ubuntu manpage](https://manpages.ubuntu.com/manpages/noble/en/man1/systemd-creds.1.html),
  [systemd CREDENTIALS.md (TPM2+host-key binding,
  `LoadCredentialEncrypted`)](https://chromium.googlesource.com/chromiumos/third_party/systemd/+/2f6c52b919dcc4a52e597ada11af9267e3550029/docs/CREDENTIALS.md),
  [per-service `$CREDENTIALS_DIRECTORY`
  tmpfs](https://docs.rs/secretx-systemd/latest/secretx_systemd/),
  [per-user `--user --uid=self` binding (systemd
  256+)](https://dev.to/lyraalishaikh/stop-putting-secrets-in-environment-variables-practical-systemd-creds-on-linux-4gfi),
  [practical TPM2-bound credentials
  guide](https://oneuptime.com/blog/post/2026-03-04-systemd-credentials-secret-injection-rhel-9/view)
- AEAD choice: [XChaCha vs AES-GCM nonce
  limits](https://highassurance.rs/chp14/tactical_trust_1.html),
  [RustCrypto NCC audits](https://brxken128.github.io/dexios/)
- Mac fallback: [keyring crate (4.x,
  mature)](https://lib.rs/crates/keyring), [keyring-rs
  repo](https://github.com/open-source-cooperative/keyring-rs)

## 8. Open questions

1. Does the provisioned Ubuntu 26.04 box have a real TPM2 (2.0,
   not 1.2) and systemd ≥256 with TPM2 enabled? (Verify before
   build; decides systemd-creds vs tpm2-tools fallback.)
2. Which PCR policy should bind the KEK — systemd's default, or an
   explicit `--tpm2-pcrs=` mask? (Tighter = Secure Boot/firmware
   upgrades force re-enrollment; looser = weaker boot-integrity
   binding.)
3. Who are the two KEK-escrow custodians, and where do the two
   offline escrow copies live?
4. Should `META_API_KEY` migrate from env into `config(secret)`
   now (recommended), or stay env-only per current `SECURITY.md`?
5. Step-up auth for secret-set: reuse planned staff passkey/TOTP
   (`SECURITY.md` Q5), or accept plain admin session for v1?
6. Should `bool`/`text` values also be API-readable by staff role,
   or admin-only like secrets management?
7. Yearly KEK rotation acceptable, or align with another cadence
   (e.g. each Ubuntu LTS point release)?
8. `xtask` running as root for enroll/rotate vs a dedicated
   `handbook-admin` group with sudo rules — which fits the deploy
   story?
