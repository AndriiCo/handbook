# Software-TPM Sealing for Config Secrets (draft for review)

- Status: draft (goal-025 stream 5 of 5, for author review)
- Date: 2026-10-27
- Governing spec: [secure-config.md](secure-config.md) (TPM2-sealed
  envelope, systemd-creds, Mac fallback). This spec does not change
  it — it adds the `Sealer` code abstraction so the same call sites
  run on a software TPM today and a hardware TPM later.
- Reads: `secure-config.md` §1–§2; `crates/config/src/lib.rs`
  (envelope, `Kek`, `set_value`/`get_display`/`rotate`);
  `xtask/src/config.rs` (`config-enroll --seal` via systemd-creds);
  `docs/specs/README.md` decision 27 (software-TPM abstraction,
  hardware-ready).
- Assumptions: no API shape change to the config row paths
  (`set`/`get`/`rotate` keep their signatures); no new system
  build dependencies (pure-Rust rule, `web-stack.md` §1); tests
  run hermetically (no TPM chip, no daemon, port 0).

## 1. Sealer trait design

One object-safe trait in `crates/config` (`seal.rs`), implemented
by exactly two providers today:

```rust
pub trait Sealer: Send + Sync {
    fn name(&self) -> &'static str;
    fn seal(&self, kek: &[u8; 32]) -> Result<Vec<u8>, SealError>;
    fn unseal(&self, blob: &[u8]) -> Result<Kek, SealError>;
}
```

- `seal` wraps one 32-byte KEK into an opaque, provider-tagged
  blob. `unseal` opens a blob back into the existing `Kek` type,
  which feeds `set_value`, `get_display`, `get_secret_for_job`,
  and `rotate` unchanged — the integration point is the `Kek`
  value, not new parameters.
- `SealError` has three variants, all free of key material:
  `Unsupported` (wrong provider for this operation — the stub
  path), `Corrupt` (bad magic/version, failed AEAD open,
  truncation), `Unavailable` (I/O or environment failure).
  It converts into `ConfigError` (`Corrupt` → `Crypto`, the rest
  → `KekUnavailable`), so sealed flows reuse existing handling.
- `Kek::seal_with(&self, sealer: &dyn Sealer)` is the only new
  `Kek` method (additive; the crate's public signatures are
  otherwise untouched).

Providers:

| Provider | Type | seal | unseal |
|---|---|---|---|
| Software (default) | `SoftwareSealer` | real (XChaCha wrap, §2) | real |
| Hardware (stub) | `HardwareTpmSealer` | `Unsupported` + enroll pointer | `Unsupported` + collect pointer |

`Send + Sync` supertraits keep the trait usable from the async
server later; `Box<dyn Sealer>` works today.

## 2. Software provider choice + rationale

Researched candidates for "software TPM" behind the trait:

| Option | Shape | Verdict |
|---|---|---|
| swtpm daemon + `tss-esapi` | C emulator over TCP/socket; Rust via bindgen FFI to C `tss-esapi-sys`/`libtss2` | Rejected |
| `tpm2-tools` subprocess | shell out to `tpm2_create`/`tpm2_unseal`; needs `/dev/tpmrm0` or a running swtpm | Rejected |
| wolfTPM Rust crate | bindgen FFI over C wolfTPM; `open_swtpm()` still needs a daemon | Rejected |
| OS keychain (`keyring`) | macOS Keychain / Secret Service; `secure-config.md` §6 names it as the Mac fallback | Deferred to seam |
| Pure-Rust sealed envelope | XChaCha20-Poly1305 wrap of the KEK under a local wrapping key; zero new deps | **Chosen** |

Why the emulator path loses for this project:

1. **Single binary.** swtpm is a C daemon plus `libtss2-dev`;
   `tss-esapi` links the C TSS via bindgen. That breaks the
   pure-Rust/no-system-libs rule `secure-config.md` §1.1 already
   used to reject in-process `tss-esapi` — the same argument
   covers the emulator transport.
2. **Hermetic tests.** The swtpm flow needs a daemon on a TCP
   port plus `TCTI=swtpm:…` env for every test run. The gate for
   this stream is tests with no hardware and no daemons; an
   emulator backend fails it by construction.
3. **No marginal fidelity.** Our TPM use is seal-one-KEK /
   unseal-one-KEK at process start, not PCR policies or key
   hierarchies in-process. An emulator would exercise TPM wire
   format, which is systemd's job on the real box (§3), not ours.

Why the sealed envelope wins: it reuses the three crypto crates
already in the tree (`chacha20poly1305`, `rand`, `zeroize`),
runs identically on Mac dev, Linux CI, and the server, and keeps
the exact `seal`/`unseal` call shape the hardware provider will
fill in. It is explicitly NOT a hardware security boundary (see
loudness, §5) — it is an API-compatible stand-in with real AEAD
properties (tamper-evident, versioned blobs).

`keyring` stays a valid future provider behind the same trait
(per `secure-config.md` §6) but is wrong as the default: it
needs an interactive keychain, so headless CI and hermetic
tests cannot use it.

### Software blob format (v1)

```text
magic(8) "SWSEAL1:" ‖ nonce(24, random) ‖ XChaCha20-Poly1305 ct(48)
AAD = magic bytes. Plaintext = the 32 KEK bytes. Total = 80 bytes.
```

`unseal` rejects wrong magic, short blobs, and failed opens as
`SealError::Corrupt`. The `1` in the magic is the format version;
a v2 format gets a new magic and a compat arm in `unseal`, never
a silent reinterpretation.

### Wrapping-key custody (software root)

`SoftwareSealer` holds a `Zeroizing<[u8; 32]>` wrapping key
(redacted `Debug`), sourced three ways:

- `generate()` — ephemeral CSPRNG key (tests, never persisted);
- `from_wrapping_key(bytes)` — explicit bytes (callers that own
  custody, e.g. a future `keyring` bridge);
- `load_or_create_keyfile(path)` — file root: reads 32 raw
  bytes, or generates + writes them `0600` (unix) when absent.

The keyfile root is machine-local DAC protection only — it
proves API shape, not a machine boundary. Sealed KEK blobs are
opaque files wherever the caller stores them (dev: under tmp/
seed dirs; the `config-verify` guard in `secure-config.md` §6
still applies).

## 3. Hardware-TPM provider seam

`HardwareTpmSealer` is a compile seam, not an implementation:
both methods return `SealError::Unsupported` with an actionable
message. The messages encode the real hardware path, which
already exists outside this crate:

- **Seal (enroll, privileged, once):** `cargo xtask
  config-enroll --seal` shells out to `systemd-creds encrypt
  --with-key=host+tpm2` (`xtask/src/config.rs`). The stub's
  `seal` error names exactly this command.
- **Unseal (every boot):** systemd opens the `.cred` via the
  TPM2 chip at unit activation and delivers 32 plaintext bytes
  to `$CREDENTIALS_DIRECTORY/handbook-kek`
  (`LoadCredentialEncrypted=`); `config::load_kek()` collects
  them. The stub's `unseal` error names this file.

Replacing the stub with a real in-process provider (e.g. a
`tss-esapi` type behind a `hardware-tpm` cargo feature) is then
a provider-local change: implement the two methods, keep the
trait. No call-site edits. A future provider MUST NOT change
`Sealer`'s shape to fit TPM session plumbing — sessions stay
inside the provider.

## 4. Key hierarchy

```text
hardware root: TPM2 SRK (never leaves chip; systemd owns the handle)
  └─ KEK-vN (32 B CSPRNG, sealed in /etc/credstore.encrypted/*.cred)
       └─ config rows: nonce(24) ‖ ct, AAD = "{key}:{enc_version}"

software root: wrapping key (keyfile 0600 / explicit / ephemeral)
  └─ KEK-vN (32 B CSPRNG, sealed in SWSEAL1 blob, 80 B)
       └─ config rows: same envelope as above, same AAD string
```

Row format and AAD are provider-independent by design: a row
sealed under a software-held KEK decrypts under a
hardware-held KEK with identical bytes, given the same KEK
value and generation. Only the KEK's own custody differs.

| Key | Custody | Leaves machine? | Rotation trigger |
|---|---|---|---|
| TPM SRK | chip | never | TPM loss → re-enroll (§4.1) |
| Wrapping key | keyfile/env/ephemeral | only in dev backups | suspected dev leak; delete + re-seal |
| KEK-vN | sealed blob; `Zeroizing` + `mlock` in RAM | escrow only (§4.1) | yearly / operator exit / suspected leak |

### 4.1 Escrow and DR (unchanged from governing spec)

Escrow, two-custodian restore, and RTO/RPO are exactly
`secure-config.md` §4: each active KEK is exported once at
enroll/rotate, encrypted to two admin public keys, stored
offline in two places. Software-sealed dev KEKs are synthetic
(`dev-seed` only) and are never escrowed — there is nothing to
recover.

## 5. What runs where

| Platform | Root | Seal path | Unseal path | Loudness |
|---|---|---|---|---|
| Linux prod (TPM2) | TPM SRK | `xtask config-enroll --seal` (systemd-creds) | systemd → tmpfs → `load_kek` | none (real boundary) |
| Linux, no TPM2 | file KEK (existing fallback) | enroll without `--seal` | `HANDBOOK_KEK_FILE` (xtask) | DEV-CRYPTO banner (existing) |
| Mac dev | software wrapping key | `SoftwareSealer::seal` | `SoftwareSealer::unseal` | dev flows keep the existing banner + `dev:` refusal |
| CI / tests | ephemeral | `SoftwareSealer::generate` | same instance or keyfile | none (synthetic fixtures only) |

Mac dev keeps every `secure-config.md` §6 rule: loud banner,
`dev:`-marked keys refused outside debug builds, synthetic
data only. The software provider adds no new prod-accepted
key source — `load_kek()`'s accepted sources are unchanged.

## 6. Acceptance criteria

- [ ] `Sealer` is object-safe (`&dyn Sealer` / `Box<dyn Sealer>`
  usable) and `Send + Sync`.
- [ ] `SoftwareSealer` round-trips a KEK; wrong wrapping key,
  bit flips, truncation, and bad magic all fail closed.
- [ ] `HardwareTpmSealer` compiles as the seam and both methods
  fail with messages naming the real enroll/collect path.
- [ ] Sealed-then-unsealed KEKs drive `set_value`,
  `get_display`, `get_secret_for_job`, `rotate`, and
  `verify_all` with no signature changes.
- [ ] Zero new dependencies; no daemon, chip, env, or network
  in tests; `Debug` impls never emit key bytes.
- [ ] `cargo test -p config`, `cargo clippy -p config
  --all-targets -- -D warnings`, and `cargo fmt --check`
  are green.

## 7. Open questions

1. `keyring`-backed provider: adopt as the Mac-dev default
   later (per `secure-config.md` §6), or keep the keyfile root?
   (Owner: author; blocks nothing — the seam fits either.)
2. Should `xtask config-enroll` gain a `--software-seal` mode
   that writes SWSEAL1 blobs for dev parity? (Owner: config
   stream; needs a decision before any xtask touch.)
3. In-process `tss-esapi` provider behind a feature: ever, or
   is systemd-creds the permanent hardware boundary? (Owner:
   author; `secure-config.md` Q1/Q2 feed this.)
4. Wrapping-key rotation procedure for long-lived dev boxes:
   re-seal blobs in place, or re-enroll dev from scratch?
   (Owner: config stream; dev-only, low stakes.)

## 8. Sources

- Governing: `secure-config.md` §§1–2, 4, 6–7 (systemd-creds,
  tss-esapi rejection, escrow, Mac fallback, manpages).
- swtpm shape (C daemon, TCP `TCTI=swtpm:…`, tests require it
  running; `libtss2-dev` + `swtpm` install):
  [hyde-core setup notes](https://lib.rs/crates/hyde-core).
- `tss-esapi` FFI (bindgen over C TSS libs, `tss-esapi-sys`):
  [tss-esapi-sys on docs.rs](https://docs.rs/crate/tss-esapi-sys/latest).
- wolfTPM Rust crate (bindgen over C wolfTPM, `open_swtpm`
  still daemon-backed):
  [wolfSSL announcement](https://www.wolfssl.com/announcing-wolftpm-rust-support-with-the-wolftpm-1-0-0-crate/).
