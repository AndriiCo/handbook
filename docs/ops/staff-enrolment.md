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
3. Open `https://admin.<host>/config` (every console page sends you
   there while no staff passkey exists), paste the key once, then enrol
   the passkey from the wizard with the same key. The first enrolment
   burns the key and signs you in.

## Later staff (invite)

From the console: **Staff → Invite staff** (admins; needs a passkey
check from the last 10 minutes, confirmed in place). It shows the link
once, built from the admin origin you are on. Pending invites can be
withdrawn there. From the server shell:

```sh
# Prints a single-use URL once; the token is hashed at rest.
cargo xtask admin-invite --store <store> --role staff --cn jdoe --days 1 \
  --actor <you> --origin https://admin.<host>
#   url: https://admin.<host>/enrol?invite=<64-hex>
# On production, as hal: role, name, days, origin (empty = production).
printf 'staff\njdoe\n1\n\n' | sudo -n -u andrii-ai /usr/local/sbin/apostille-invite
```

`--store` is required (no default). `--days` is 1 to 30 (production
uses 1). Without `--origin` the task prints only the path
(`/enrol?invite=…`) to open on the admin host; with it, the full URL
(`https://` or `http://localhost`). Deliver the URL on a separate
channel. The staffer opens it, enters a display
name and completes the passkey ceremony, which also logs them in.

Failure strings: `invite malformed` (400, the URL was mangled),
`invite invalid or expired` (410, mint a fresh one), `credential
already registered` (409, use another authenticator), `verification
failed` (422, the invite is burned; mint a fresh one), `rate limited`
(429).

Audit rows: `staff_invite_issue`, `staff_enrolled`,
`staff_enrol_refused` (static reason), never raw tokens or keys.

## Login

`https://admin.<host>/staff-login`, **Sign in with passkey**: no user
ID. Staff passkeys are discoverable, so the browser offers the ones it
holds for the console and the server finds the account from the
passkey. A passkey enrolled before that (a security key that stored no
discoverable credential) signs in under "Sign in with your user ID".
(`/login` and the other customer pages answer 404 on the admin host.) A login refreshes the step-up window that destructive
actions (config writes) require. `unknown account or no passkeys` (401)
is the one answer for unknown, locked or credential-less accounts.
When a session expires mid-use, the console sends the browser to
`/staff-login?next=<page>` and returns there after sign-in. **Sign out**
sits at the bottom of the console sidebar (`POST /auth/logout` on the
admin host: ends the session, clears the cookie, audits `staff_logout`).

## Passkeys and offboarding

The **Staff** page lists every account with its passkeys (label, added,
last used). Any staffer adds a passkey for another device ("Add a
passkey", up to 5) and removes one of their own; admins remove anyone's.
An account's last passkey stays: revoke the account instead.

**Revoke access** (admins; never your own account, never the last
admin) kills every session, revokes every passkey and locks the
account, each step idempotent, audited `staff_revoked`. From the
server shell:

```sh
cargo xtask admin-offboard --store <store> --user <32-hex-id> --cn jdoe --actor <you>
```

Same steps, audited `staff_offboard`. Not reversible: a new invite for
the same name creates a new account.
