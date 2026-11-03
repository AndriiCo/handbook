# Staff enrolment

How staff accounts come to life, log in and leave. Staff sign in with a
passkey (user verification required) from an address in
`admin.allow_ips`; there are no client certificates. Spec:
[customer-auth.md](../specs/customer-auth.md) §5.

## First staffer (setup key)

1. Seal `admin.allow_ips` with your public address (a refused browser
   gets a 403 page naming the address the server saw), then restart.
2. Mint the setup key on the server; it prints once and stops working
   after 72 hours: `cargo xtask setup-key-mint --store <store>
   --kek-file <kek>` (`cargo xtask setup` does this as its first-staff
   step).
3. Open `https://admin.<host>/config`, paste the key, then enrol the
   passkey from the wizard. The first enrolment burns the key.

## Later staff (invite)

```sh
# Prints a single-use URL once; the token is hashed at rest.
cargo xtask admin-invite --role staff --cn jdoe --days 1 --actor <you>
#   url: https://admin.andrii.ai/enrol?invite=<64-hex>
# On production, as hal: role, name, days, origin (empty = default).
printf 'staff\njdoe\n1\n\n' | sudo -n -u andrii-ai /usr/local/sbin/apostille-invite
```

`--days` is 1 to 30 (production uses 1), `--origin` overrides the
admin origin (must be `https://` or `http://localhost`). Deliver the
URL on a separate channel. The staffer opens it, enters a display
name and completes the passkey ceremony, which also logs them in.

Failure strings: `invite malformed` (400, the URL was mangled),
`invite invalid or expired` (410, mint a fresh one), `credential
already registered` (409, use another authenticator), `verification
failed` (422, the invite is burned; mint a fresh one), `rate limited`
(429).

Audit rows: `staff_invite_issue`, `staff_enrolled`,
`staff_enrol_refused` (static reason), never raw tokens or keys.

## Login

`https://admin.<host>/login` with the user ID given at enrolment and the
passkey. A login refreshes the step-up window that destructive actions
(config writes) require. `unknown account or no passkeys` (401) is the
one answer for unknown, locked or credential-less accounts.

## Offboarding

```sh
cargo xtask admin-offboard --user <32-hex-id> --cn jdoe --actor <you>
```

Kills every session, revokes every credential and locks the account,
each step idempotent and audited (`staff_offboarded`). Not reversible: a
new invite for the same name creates a new account.
