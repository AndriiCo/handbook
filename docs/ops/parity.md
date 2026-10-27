# Ubuntu 26.04 parity note (Mac dev vs prod box)

Parity rule (from `web-stack.md` §1): pure Rust, no system libs, so
the Mac build and the Ubuntu 26.04.1 x86_64 prod box (`enodia`)
behave identically. Anything below that breaks must be fixed in the
build, not papered over per-host.

## What guarantees parity today

- SQLite is `rusqlite` **bundled** (compiled in) — no system
  `libsqlite3` version skew.
- TLS is rustls in-process (axum-server) — no OpenSSL on either end.
- Cross build via `cargo zigbuild --release -p web --target
  x86_64-unknown-linux-gnu`; verified 2026-10-26: ELF x86-64, max
  GLIBC 2.30, box ships newer — runs fine.
- No Docker; systemd is the only system service beside the app.

## Parity checklist (`ci` green on BOTH hosts)

On the Mac (this repo):

```sh
export RUSTC_WRAPPER=sccache
cargo xtask ci                 # fmt + clippy -D warnings + tests
cargo xtask packs-validate     # 9 jurisdiction packs clean
```

On the box (as `hal`, in a scratch clone — never in `/srv`):

```sh
cargo xtask ci                 # same gate, same toolchain channel
STORE_DIR=$(mktemp -d) cargo xtask packs-load --store "$STORE_DIR"
```

Both runs must be green with zero warnings. Toolchain skew (Mac
stable vs box stable) is resolved by matching `rustc --version`,
not by relaxing lints.

## Known deltas (accepted, not parity bugs)

- TLS issuance/renewal is in-binary ACME (Google Public CA)
  on the box — dev never issues certs.
- Systemd unit + `andrii-ai` user + `/var/lib/apostille` exist on
  the box only; dev runs `STORE_DIR=… cargo run -p web` (ports and
  all other settings come from sealed DB rows, `config-db.md`).
- TPM absent on `enodia` → the KEK is file-backed: root holds
  `/etc/apostille/handbook-kek` (0600), systemd exposes it via
  `LoadCredential=handbook-kek` (see `apostille.service`).
  Hardware-sealed upgrade lands with TPM hardware. No Mac
  behavior change (dev KEK).
- Research traffic (URL watching, fetches) originates ONLY from the
  prod IPs — dev machines must never fetch watched URLs directly.
