# In-binary ACME (Google Public CA, ARI renewal)

Our Rust binary issues and renews its own TLS certificate — no
certbot on the box, no restarts. One certificate covers `andrii.ai`
+ `admin.andrii.ai`, issued via Google Public CA with External
Account Binding (EAB), validated over http-01, renewed on the CA's
ARI window (RFC 9773) with a lifetime fallback, and hot-swapped
into the live server.

Constraint (unchanged): the ONLY certificates we may use come from
Google PKI. See `certificates.md` for the EAB minting runbook.

## Configuration (DB keys, not env — see `docs/specs/config-db.md`)

| DB key | Type | Meaning | Default |
|---|---|---|---|
| `acme.enabled` | bool | true turns ACME mode on | false |
| `acme.directory` | text | `prod`, `staging`, or a full `https://` directory URL | `prod` |
| `acme.domains` | text | Comma-separated names on the cert | `andrii.ai,admin.andrii.ai` |
| `acme.email` | text | Optional contact mailbox (`user@host`, no `mailto:`) | none |
| `acme.eab_kid` | text | Google EAB key ID (**required**) | — |
| `acme.eab_hmac` | **secret** | Google EAB HMAC, base64 std or URL-safe (**required**, write-only) | — |
| `paths.acme_challenge_dir` | text | http-01 token files (already served) | `acme` |

`STORE_DIR` stays env (bootstrap): certs persist under
`<store>/acme/`. Explicit `tls.cert_file`/`tls.key_file` always
win: when both are set, ACME stays off even with
`acme.enabled=true`. Anything else missing or contradictory
refuses to start rather than downgrade to HTTP. All values are
read once at boot (restart to apply); set them with `cargo xtask
config-set` (EAB HMAC via `--value-from-stdin`, never argv).

Google endpoints: prod
`https://dv.acme-v02.api.pki.goog/directory`, staging
`https://dv.acme-v02.test-api.pki.goog/directory`. Staging boots
with a loud `UNTRUSTED` banner; its certs fail client validation
by design.

## How it works

- Boot (`serve_acme` in `crates/web/src/main.rs`): load
  `<store>/acme/cert.pem` + `key.pem` when the pair parses and has
  over a day of life left; otherwise issue now. No usable pair and
  no issuance means no process — fail closed.
- Issuance (`crates/acme`): restore the ACME account from
  `<store>/acme/account.json`, or register it with the EAB pair on
  first run; one order for all domains; one http-01 challenge per
  name answered from `paths.acme_challenge_dir`; CSR + fresh key via
  rcgen; chain persisted atomically (temp + rename, key 0600).
- Challenges reach us two ways: port 80 serves
  `/.well-known/acme-challenge/` directly (required for first
  issuance, when no cert exists to terminate a redirect), and the
  HTTPS public router keeps serving the same dir for CAs that
  follow the 308. Both serve the same files.
- Renewal loop: after each wake, fetch the CA's `RenewalInfo` for
  the leaf (base64url AKI + serial per RFC 9773 §4.1), sleep until
  a uniform-random point inside the suggested window (one sleep
  capped at 24h, then re-fetch — the CA moves windows on
  revocation events), and renew with `replaces` pointing at the old
  cert. A `Retry-After` hint shortens the sleep.
- Fallback (ARI unreachable, unparsable leaf, or account down):
  renew once remaining lifetime drops to 30 days, re-checking at
  most every 6 hours. Issuance failures back off from 5 minutes,
  doubling to a 6-hour cap, and retry forever.
- Hot swap: each renewal calls
  `RustlsConfig::reload_from_pem` on the live config — existing
  connections keep the old cert, new handshakes get the new one,
  no listener restarts. A refused swap retries the same pair
  (never a fresh issuance) until it lands, so memory cannot lag
  the disk.

## Files on the server

`<store>/acme/` holds `cert.pem` (chain, 0644), `key.pem` (0600),
and `account.json` (0600 — it embeds the ACME account private
key). Back up the dir with the store; its secrecy matters exactly
as much as the TLS key's.

## Manual staging run (live CA, not CI)

CI pins the scheduling math, config parsing, persistence, CSR
shape, and the port-80 challenge route — but no test talks to a
CA. Before trusting a release, run once against staging from a
host that answers the domain's port 80:

```sh
STORE=/tmp/acme-stage/store KEK=/tmp/acme-stage/kek
cargo xtask config-enroll --store "$STORE" --kek-file "$KEK"
set() { cargo xtask config-set --store "$STORE" --kek-file "$KEK" \
  --key "$1" --type "$2" --value "$3"; }
set acme.enabled bool true
set acme.directory text staging
set acme.eab_kid text '<staging-kid>'
set acme.email text 'ops@example.com'
set paths.acme_challenge_dir text /tmp/acme-stage/acme
echo -n '<staging-hmac>' | cargo xtask config-set --store "$STORE" \
  --kek-file "$KEK" --key acme.eab_hmac --type secret --value-from-stdin
# Debug boot reads the same KEK bytes via the dev source (dev: + hex);
# release boots read the systemd credential (see config-db.md §2.2).
STORE_DIR="$STORE" \
  HANDBOOK_DEV_KEK="dev:$(od -An -tx1 "$KEK" | tr -d ' \n')" \
  cargo run -p web
```

Expect: `UNTRUSTED` banner, an issuance log line, HTTPS up on the
staging chain, and `<store>/acme/` holding all three files. Then
`rm` the staging dir — staging credentials never touch prod.

## Troubleshooting

- Boot refuses with an ACME error: the message names the cause
  (config field, CA problem, unparsable pair). Fix it and restart;
  the binary never serves HTTP in place of broken TLS.
- Saved account is dead (moved CAs, revoked account): stop the
  service, delete `<store>/acme/account.json`, mint a FRESH EAB
  secret (each secret dies on first use — never reuse one), set
  the new pair, and start. The pair re-registers on boot.
- Renewal stuck failing: the loop logs every failure and backs
  off; the old cert keeps serving until expiry. Check CA
  reachability, port 80 from the internet, and the challenge dir
  writability — then just wait for the next retry.
- Clock skew breaks ARI math and expiry checks before anything
  else does; NTP first when windows look wrong.

## Follow-ups (not this change)

- Done (config-db stream): EAB pair from TPM-sealed config
  (`acme.eab_kid` text + `acme.eab_hmac` write-only secret) — no
  env fallback remains.
- Metrics: expiry timestamp + last-renewal result exported for
  alerting (currently log lines only).
