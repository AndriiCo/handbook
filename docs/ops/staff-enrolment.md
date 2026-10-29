# Staff enrolment runbook (Phase 4)

Commitment (decision 56, due TBD): second staffer enrolled
within 30 days of first enrol (clock starts at the first
`staff_enrolled` audit row; fill both dates from prod, close
by deleting this line and logging the enrol date in
`drill.md`).

How staff accounts come to life, log in, and leave. Spec:
[customer-auth.md](../specs/customer-auth.md) §5. Staff method
(decided): **passkey (UV required) + mTLS client cert** — both
factors on every admin request, neither alone grants anything.

This slice adds **no config keys** (certs live as files under
`<store>/admin-certs/`, invites as rows in `staff_invite`,
both keyless by design). First-boot exception: the setup
wizard ([secure-config.md](../specs/secure-config.md) §9) lets
the first staffer enrol with the setup key instead of cert +
invite, then burns the key — everything below covers the
second staffer onward.

Conventions: `--store` defaults to `store` (run from the repo
root or pass `--store /var/lib/apostille` on the server as
hal-or-root per [scripts/README.md](scripts/README.md));
`--actor` defaults to `xtask` (pass `--actor <you>` so the
audit names a human). Timestamps below are the operator's
local time; the server stores epoch seconds.

Open questions for the author (spec §8): OPEN-15 (CN→user
mapping), OPEN-20 (delivery channel), OPEN-21 (key copies),
OPEN-22 (who mints), OPEN-23 (admin quorum size —
break-glass is decided, decision 59), OPEN-24 (CRL/OCSP).
Until answered, the defaults below apply (single admin
mints; split-channel delivery; keys kept 0600 until
confirmed delivery). OPEN-19 (CA custody) is resolved —
see §1.

## 1. Client certificates

The local CA lives at `<store>/admin-certs/` (`ca.pem`,
`ca-key.pem`, per-client `client-<cn>-<fp8>.pem` +
`.key.pem`) and is auto-created on first issue. Fingerprints
are lowercase SHA-256 hex of the DER bytes (see
`xtask/src/admin_cert.rs`).

 Mint order matters: **cert first, invite second** —
 `admin-invite` refuses a CN with no live cert row.

Prod custody (OPEN-19, decided 2026-10-29): the live
`ca-key.pem` stays root-only on the server; one offline
encrypted copy is held by the designated custodian (name
TBD). Server-side auto-created CA keys are staging/dev-only
— gap: no offline-issue/import path exists yet
(`issue()` signs with the local key), so prod issuance
without a standing server key needs a witnessed key
ceremony, not automation.

```sh
# Issue (operator delivers the .pem + .key.pem out-of-band).
cargo xtask admin-cert-issue --cn jdoe --actor andrii --days 365
# Verify a fingerprint is live (CN match + unrevoked + unexpired).
cargo xtask admin-cert-verify --fp <64-hex> --expect-cn jdoe
# Revoke (idempotent; registry-only, no CRL in Phase 4 — OPEN-24).
cargo xtask admin-cert-revoke --cn jdoe --actor andrii
```

Root-free path (prod): hal runs the same operations through the
pinned wrappers (installed by `ops-hal-access.sh`, §5–6) — no
new trust (hal already holds full sqlite + deploy):

```sh
# As hal on enodia. Issue prints paths + fenced PEM delivery blocks
# (capture stdout to 0600 files, deliver out-of-band, then wipe).
printf 'issue\njdoe\n365\n' | sudo -n -u andrii-ai /usr/local/sbin/apostille-cert
printf 'verify\n<64-hex>\njdoe\n' | sudo -n -u andrii-ai /usr/local/sbin/apostille-cert
printf 'revoke\njdoe\n\n' | sudo -n -u andrii-ai /usr/local/sbin/apostille-cert
```

CN rule (enforced): ASCII word chars, dots, dashes, ≤64 chars
(`jdoe`, `ops.bot`, `admin-2` — not emails). One human MAY
hold several CNs (per-device certs: `jdoe-laptop`,
`jdoe-phone`); each enrols separately.

## 2. Invite ceremony

```sh
# 1. Operator mints (prints the single-use URL ONCE — the token
#    is hashed at rest and never retrievable). Prod mandates
#    --days 1 (24h invites, decision 58); the code default
#    stays 7d.
cargo xtask admin-invite --role staff --cn jdoe --days 1 --actor andrii
#   invite for jdoe (staff)
#     url: https://admin.andrii.ai/enrol?invite=<64-hex>
#     expires in 1d; single-use; token is NOT stored
#    Flags: --days 1..30 (default 7), --origin <url> for staging
#    (default https://admin.andrii.ai; must be https:// or
#    http://localhost), --store, --actor.
# Root-free path (prod): 4 stdin lines (role, cn, days, origin-or-empty).
printf 'staff\njdoe\n1\n\n' | sudo -n -u andrii-ai /usr/local/sbin/apostille-invite
```

2. Operator delivers the URL out-of-band (default: a different
   channel than the key pair — OPEN-20).
3. Staffer opens the URL in a browser holding the invited
   client cert, enters a display name, completes the UV
   ceremony. Enrolment logs them in (session on `hb_admin`,
   step-up fresh).
4. Failure modes (exact server strings → meaning):
   - `invite malformed` (400): the URL was mangled — resend.
   - `invite invalid or expired` (410): unknown, expired, or
     already consumed — mint a fresh invite.
   - `client cert required` (403): no (live) cert presented —
     install the client cert, check expiry/revocation.
   - `certificate does not match invitation` (403): presented
     CN ≠ invited CN — enrol on the device holding the
     invited cert. Does NOT burn the invite.
   - `credential already registered` (409): this security key
     is enrolled elsewhere — use another key.
   - `verification failed` (422): ceremony failed (incl.
     non-UV) — retry; the invite is burned, so mint fresh.
   - `rate limited` (429): slow down.

Audit: `staff_invite_issue` at mint (actor + `role:cn`);
`staff_enrolled` at finish (`role:cn:fingerprint`); refusals
as `staff_enrol_refused` with a static reason. Hashes and
fingerprints only — never raw tokens, challenges, or key
bytes.

## 3. Staff login

URL: `https://admin.andrii.ai/login`. User ID (32-hex,
given at enrol time) + passkey ceremony, with the client
cert present. Login refreshes step-up unconditionally, so a
fresh login satisfies the destructive-action window (§5).

Failure modes: `unknown account or no passkeys` (401 — one
shape for unknown/locked/cred-less, no oracle), `credential
unknown` + `re-register` (404 — staffer needs a fresh
invite, never self-service), `challenge invalid or expired`
(410), `use the customer login` (403 — customer credential
on the admin branch), `client cert required` (403).

## 4. Offboarding

One command, cert first (a login racing the offboard can
never resurrect access — any new session still fails
`AdminAuth`), one transaction per step, every step idempotent
(re-runnable after partial failure):

```sh
cargo xtask admin-offboard --user <32-hex-id> --cn jdoe --actor andrii
# offboarded <id> (jdoe): 1 cert(s), 2 session(s), 1 credential(s) revoked; account locked
```

Steps: (1) revoke every live `admin_cert` row for the CN,
(2) kill all sessions, (3) revoke every credential (sets
`revoked_at` + disables the link rows), (4) lock the
account. Audit: per-action rows + a `staff_offboarded`
summary. `--cn` stays mandatory until OPEN-15 (CN→user
mapping) lands.

Verify after (read-only): `admin-cert-verify --fp …` fails
(revoked); the staffer's  `/me` 401s/403s; a fresh
invite for the same CN enrols a NEW account (offboarding is
not reversible — by design).

Suspend (decision 57) = revoke-all + re-enrol:
`admin-cert-revoke --cn <cn>` for every live CN (or
`admin-offboard --user <id> --cn <cn>` per staffer for the
full suspend: certs + sessions + creds + lock). No
single-command wrapper exists yet — per-CN loop until one
is added.

## 5. Destructive-action step-up (pilot)

`PUT /config/{key}` requires fresh step-up (UV within
10 min, or a session minted within 10 min); otherwise `403
step-up required: log in again first`. Staff login always
steps up, so the common case needs no extra ceremony. The
full action list is OPEN-17 — until then, config writes are
the only gated action.

## 6. Staging acceptance (staff slice)

Preconditions: staging admin origin reachable with TLS +
client certs; staging `--origin` passed to `admin-invite`;
test CNs only (never real staff names); fixtures
synthetic-only.

1. Issue cert → mint invite → enrol green: session on
   `hb_admin`, `GET /me` shows role + fingerprint,
   step-up fresh (config write succeeds).
2. Invite replay (second finish, same token) → `410`.
3. Wrong-CN cert at start and finish → `403 mismatch`; the
   invite is NOT burned (re-enrol with the right cert
   still works).
4. Cert revoked between start and finish → `403`.
5. Staff login green (same device); customer credential on
   admin login → `403 use the customer login`;
   staff credential on apex login → `403 use the admin
   login`.
6. Session-without-cert → `403` on every admin route;
   cert-without-session → `401`.
7. Offboard → sessions dead + cert rejected + creds
   revoked + account locked; re-run is a clean no-op;
   re-enrol needs a fresh invite and mints a new account.
8. Stale session (>10 min, no step-up) on config write →
   `403 step-up required`; fresh login → write succeeds.
9. Cross-origin ceremony (admin host page, apex RP):
   enrol + login both green — the OPEN-1 live-proof sprint
   (decided 2026-10-29), run immediately after enrolment
   unblocks; record the outcome.
10. Audit rows carry hashes/fingerprints only — no raw
    invite tokens, session IDs, challenges, or key bytes.
11. `cargo xtask ci` green; `check`/`check:imports`/
    `build`/`build:admin`/`smoke` green (staff UI ships
    in the admin bundle only); `secrets-scan` clean.

## 7. Rollback

Additive-only, no numbered migration: `staff_invite` is a
new table the old binary never touches; `staff_*` challenge
kinds are unknown strings to the old code (wrong-kind takes
return `None` — verified by test); the new xtask tasks and
admin routes simply don't exist on the old binary.
Rollback = previous binary + previous unit file. Residuals:
invites minted by the new binary sit unused (7-day TTL,
single-use, harmless); staff accounts/credentials created by
the new binary are inert on the old binary (no admin
ceremony routes to use them) and resume working on
re-upgrade. No manual cleanup, no data loss.
