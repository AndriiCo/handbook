# Config values: where every key comes from

Combined operator guide for the sealed `config` rows the app
reads at boot. Sections land with the phase that introduces
their keys; this file grows — it never carries live values.

Conventions: set keys in `/admin/config` (staff/admin session +
client cert on the admin host) or seed headless with
`xtask config-set --key <k> --type <t> --value-from-stdin`
(never on argv). `text` rows render decrypted; `secret` rows
show presence-only and never echo. Every PUT audits key+version
only. A restart applies new rows (read-once boot).

## Google OAuth (`google.*`, Phase 2)

Spec: [customer-auth.md](../specs/customer-auth.md) §9.
Flow: server-side authorization code, no Google JS in the SPA.

### Google Cloud Console click-path

Extends [auth-playbook.md](../auth-playbook.md) §3 (verify exact
clicks in the current UI per the playbook header):

1. Console → the handbook project → **APIs & Services → OAuth
   consent screen**: External user type, app name, support
   email, authorized domain `andrii.ai`. Scopes: `profile` +
   `email` ONLY (keeps verification minimal). While
   External+unverified: **Audience → Test users** → add each
   staging tester's Gmail; unverified apps show the "unverified
   app" interstitial to non-test users. Submit for verification
   only if scope use demands it.
2. **Credentials → Create Credentials → OAuth client ID** →
   Application type **Web application**. No JavaScript origins
   are needed (no GIS button — the flow is a server-built
   redirect). **Authorized redirect URIs** (exact match, one
   per environment, all in the SAME client):
   - prod: `https://andrii.ai/apostille/api/v1/auth/google/callback`
   - staging: `https://<staging-host>/apostille/api/v1/auth/google/callback`
     once the hostname lands (still TBD per staging-plan)
   - dev: `http://localhost:3000/apostille/api/v1/auth/google/callback`
     (dev port default is 3000; `http://localhost` origins are
     accepted only with the `admin.mtls_off` dev seam, which
     refuses release boot)
3. Copy the **Client ID + Client secret** at creation (the
   secret shows once).

RISC / Cross-Account Protection (playbook §3 step 4) is OUT of
this slice but REQUIRED before prod reliance on revocation:
carry as an explicit follow-up gate (spec OPEN-7) — enable the
RISC API, service account, receiver endpoint, register via
`stream:update`. `login.rs` already encodes the event→action
table the receiver will call.

### The two keys

- `google.client_id` — type `text` (public identifier, safe to
  log). Paste in `/admin/config` in the text row.
- `google.client_secret` — type `secret` (write-only). Paste in
  `/admin/config` secret row ("Set (value hidden)"/"Not set" +
  password box only, never echoes), or seed headless:
  `xtask config-set --key google.client_secret --type secret --value-from-stdin`.

Boot-without-keys: missing rows NEVER refuse boot — Google
start/callback fail closed (`503` / `oauth_unavailable`) and
the boot logs one `WARNING` line. A stderr warning + 503s with
working passkey flows is the expected pre-key state.

Rotation: set-new PUT overwrites the ciphertext; restart;
verify a staging login BEFORE revoking the old secret in
Console → Credentials (revoke-after-verify order). KEK
rotation re-wraps rows — it does NOT rotate the Google
secret; do both on suspected exposure.

### Staging acceptance (Google slice)

Preconditions: staging redirect URI registered; test-user
Gmails added while unverified; keys pasted + service
restarted; fixtures synthetic-only.

1. Logged-out login creates account + session: Google button →
   consent → lands logged in; `GET /auth/me` 200 with
   `methods: ["google"]`; `identity_link(method='google')` +
   `account` rows created.
2. Login again resolves the SAME account (no duplicate rows).
3. Verified-email duplicate → `link_required`, accounts stay
   separate (rule 5 — the pre-account-takeover fixture).
4. Link flow from Security (fresh session ≤10 min) → methods
   list shows the google entry; `method_linked` audits the
   sub HASH.
5. Unlink google non-last → 200; sole google method →
   `409 last method cannot be removed`; account NOT locked.
6. Replayed/expired state → `oauth_expired`; stripped binding
   cookie → `oauth_failed`; cancel at Google → `oauth_denied`.
7. Keys wiped on a scratch DB → restart → start 503s,
   passkey flows unaffected; restore + restart → green.
8. Audit rows carry hashes only — no raw sub, email, token,
   or secret bytes anywhere.
9. `cargo xtask ci` green; `web-budget` green (Google UI must
   fit the remaining headroom — no bundled Google scripts);
   secrets-scan clean.

### Rollback

Additive-only, no numbered migration: new keys are ROWS in
the generic config KV table; `identity_link(method='google')`
rows are ignored by the old binary (old `list_methods`
renders unknown methods generically, old unlink 400s them).
Rollback = previous binary + previous unit file. Residual: a
rolled-back user with ONLY google linked cannot unlink until
re-upgrade — acceptable, no data loss, no manual cleanup.

## Apple Sign in (`apple.*`, Phase 3)

Spec: [customer-auth.md](../specs/customer-auth.md) §10
(shared chassis decisions D1–D5 + Apple slice). Flow: server-side
authorization code with `response_mode=query` (keeps the GET
callback; wire behavior confirmed at first real login per
spec OPEN-9), scopes `name email`, no Apple JS in the SPA. The
`user` JSON Apple sends on first authorization is ignored (we
store hashes only).

### Apple Developer portal click-path

Extends [auth-playbook.md](../auth-playbook.md) §2 (verify exact
clicks in the current UI per the playbook header):

1. developer.apple.com → Account → **Certificates, Identifiers &
   Profiles → Identifiers** → the primary App ID → enable **Sign
   in with Apple** → Save (prerequisite for the Services ID +
   key below).
2. **Identifiers → Services IDs → +** → Description + Identifier
   (reverse-DNS, e.g. `ai.andrii.signin`; this string IS
   `apple.client_id`, and the `aud` the server checks). Edit →
   enable **Sign in with Apple** → **Configure** → Primary App
   ID → **Website URLs**: `andrii.ai` plus the staging host
   once it lands (domain verification lives here; still TBD
   per staging-plan) → **Return URLs** (exact match, all in
   the SAME Services ID):
   - prod: `https://andrii.ai/apostille/api/v1/auth/apple/callback`
   - staging: `https://<staging-host>/apostille/api/v1/auth/apple/callback`
   - dev: `http://localhost:3000/apostille/api/v1/auth/apple/callback`
     (localhost only with the `admin.mtls_off` dev seam, which
     refuses release boot)
   Save.
3. **Keys → +** → name it → enable **Sign in with Apple** →
   **Configure** (primary App ID) → Continue → Register →
   **Download the `.p8` NOW (once-only)**. Record the **Key
   ID** (shown on the key row) and the **Team ID**
   (Membership page). Multiple active keys are allowed — this
   is what makes revoke-after-verify rotation possible.
4. **Private-relay senders (conditional):** if we ever SEND
   mail to `@privaterelay.appleid.com` addresses (magic-link
   fallback, notifications), register each sender under the
   Services ID Sign-in-with-Apple configuration or Apple
   bounces. Receiving relay addresses at login needs no
   registration. Until a send path exists, skip — but
   re-check before any outbound-mail slice.
5. Notifications endpoint (`.../auth/apple/events`, playbook
   §2 step 4) is OUT of this slice but REQUIRED before prod
   reliance on revocation: carry as an explicit follow-up
   gate (same treatment as Google RISC/OPEN-7, spec OPEN-11).
   `login.rs` already encodes the Apple event→action table
   the receiver will call.

### The four keys

- `apple.client_id` — type `text`. The Services ID from step
  2 (reverse-DNS string; also the aud check).
- `apple.team_id` — type `text` (JWT `iss`).
- `apple.key_id` — type `text` (JWT `kid`).
- `apple.private_key` — type `secret` (write-only). The FULL
  `.p8` PEM including the `-----BEGIN/END PRIVATE KEY-----`
  lines. Paste in `/admin/config` secret row (never echoes),
  or seed headless:
  `xtask config-set --key apple.private_key --type secret --value-from-stdin < AuthKey_XXXX.p8`.
  Multiline survives both paths (see PEM note); never
  `--value` on argv.

PEM note (verified): `secret` validation is length ≤ 8 KiB +
non-empty only — newlines unrestricted; a P-256 `.p8` is
~250 bytes. The admin PUT stores the JSON string's bytes
verbatim; the stdin path trims only the trailing newline.
Admin GET renders secrets presence-only and every PUT audits
key+version only. Operator rule: paste the exact PEM with
header/footer; a stale or truncated paste fails closed at
the first exchange (ES256 sign error → `oauth_failed`),
never at boot.

Boot-without-keys: missing/empty/wrong-typed rows → `None`
(lenient overlays, warning on mismatch) — boot ALWAYS
succeeds. Apple readiness = all four halves present and
non-empty; otherwise start 503s (`apple login unavailable`)
and the callback 302s `oauth_unavailable`, with one boot
WARNING line. Passkey/Google flows unaffected. NOTE: key
CORRECTNESS (right .p8 for the key_id/team) cannot be
checked at boot — only a live token call proves it, so the
staging checklist's real login is the proof gate, not the
503 check.

Rotation: create the new key in the portal (step 3) FIRST →
set-new PUT → restart → verify a staging login BEFORE
revoking the old key in Keys (revoke-after-verify order;
Apple permits overlapping keys). KEK rotation re-wraps rows
— it does NOT rotate the .p8; do both on suspected
exposure. `client_id`/`team_id`/`key_id` rotate by
overwrite + restart (no portal revoke exists for
identifiers).

### Staging acceptance (Apple slice)

Preconditions: staging return URL registered on the SAME
Services ID; keys pasted + restarted; test Apple IDs ready
(Apple has NO web sandbox — dedicated test IDs, never
personal; Hide-My-Email ON so staging holds only relay
hashes; revoke grants after the drill; record test IDs
nowhere in the repo); fixtures synthetic-only.

1. Logged-out login creates account + session: Apple button
   → authorize → lands logged in; `GET /auth/me` 200 with
   `methods: ["apple"]`; `identity_link(method='apple')` +
   `account` rows created.
2. Login again resolves the SAME account (no duplicate rows).
3. Real-email login with a matching verified Google email →
   `link_required`, accounts stay separate (rule 5 — the
   pre-account-takeover fixture).
4. Relay login NEVER matches by string, even when the relay
   forwards to a known address (rule 3 — the relay no-match
   fixture); the relay row stores `is_relay=1` and joins an
   existing account only via the explicit link flow.
5. First-login-only payload: revoke the test grant (Apple
   ID Settings → Sign in with Apple → stop using) → next
   login re-requests name/email; second login without
   revoke still resolves by `sub` (no email needed).
6. Link flow from Security (fresh session ≤10 min) →
   methods list shows the apple entry; `method_linked`
   audits the sub HASH.
7. Unlink apple non-last → 200 (fresh-session `"session"`
   proof accepted for Apple-only users, same tradeoff as
   Google); sole apple method → `409 last method cannot be
   removed`; account NOT locked.
8. Replayed/expired state → `oauth_expired`; stripped
   binding cookie → `oauth_failed`; cancel at Apple →
   `oauth_denied`.
9. Keys wiped on a scratch DB → restart → start 503s,
   passkey/Google unaffected; restore + restart → green.
10. Rotation drill: new .p8 set + restart → login green →
    old key revoked in portal → login STILL green (proves
    the new key is live, not the old).
11. Audit rows carry hashes only — no raw sub, email, token,
    relay address, or key bytes anywhere.
12. `cargo xtask ci` green; `web-budget` green (221KB gate;
    no Apple JS); secrets-scan clean.

### Rollback

Additive-only, no numbered migration: new keys are ROWS in
the generic config KV table; `identity_link(method='apple')`
rows are ignored by the old binary (old `list_methods`
renders unknown methods generically, old unlink 400s them).
Rollback = previous binary + previous unit file. Residual: a
rolled-back user with ONLY apple linked cannot unlink until
re-upgrade — acceptable, no data loss, no manual cleanup.

## Meta Login (`meta.*`, Phase 3)

Spec: [customer-auth.md](../specs/customer-auth.md) §11
(shared chassis D1–D5 + Meta slice). Flow: server-side
authorization code (Facebook Login for web), **no `scope`
param** — the default `public_profile` is all the sub-only
flow needs, so there is no review surface at all. No Meta
JS SDK in the SPA (server-built redirect). No email is ever
fetched: `debug_token` returns no email and Meta email never
matches, so the flow stores `provider_sub` = app-scoped id
with a NULL email hash. Graph API version pinned to
`v26.0` (highest valid version probed 2026-10-28; versions
sunset on ~2-year terms — spec OPEN-12 watches expiry).

### Meta developers portal click-path

Extends [auth-playbook.md](../auth-playbook.md) §4 (verify exact
clicks in the current UI):

1. developers.facebook.com → **My Apps → Create App**
   (website/consumer use case) → record the **App ID**
   (dashboard, also Settings → Basic; this IS
   `meta.client_id`).
2. **Add a Product → Facebook Login → Settings → Client
   OAuth Settings**: **Client OAuth Login = Yes**; **Use
   Strict Mode for Redirect URIs = Yes**; **Valid OAuth
   Redirect URIs** (exact match, all in the SAME app):
   - prod: `https://andrii.ai/apostille/api/v1/auth/meta/callback`
   - staging: `https://<staging-host>/apostille/api/v1/auth/meta/callback`
   - dev: `http://localhost:3000/apostille/api/v1/auth/meta/callback`
   Enforce HTTPS on. ("Login with JavaScript SDK" is inert
   for this flow — the SPA never loads the SDK.)
3. Permissions: add NOTHING (Data Use Checkup stays trivial).
   While the app is in **Development mode**, only
   Admin/Developer/Tester roles can log in — add each
   staging tester under Roles. Development mode is
   sufficient for staging acceptance. (Going Live may
   require Business verification for some configurations.)
4. **Settings → Basic → App secret**: **Show** (re-auth
   required) → copy (this IS `meta.client_secret`). The
   server sends `appsecret_proof` on the `debug_token` call
   and never trusts a client-supplied user id.
5. Deauthorize + Data-Deletion callback URLs (playbook §4
   step 4, `.../meta/deauthorize`, `.../meta/delete-request`)
   are OUT of this slice but REQUIRED before prod reliance
   on revocation: carry as a follow-up gate like RISC/OPEN-7
   (spec OPEN-13). `signal_action` already encodes both Meta
   actions. NOTE: the deletion-receipt `{url,
   confirmation_code}` tracker has no table yet — OPEN-13
   designs it with the endpoints.

### The two keys

- `meta.client_id` — type `text` (public App ID, safe to log).
- `meta.client_secret` — type `secret` (write-only). Paste in
  `/admin/config` secret row, or seed headless:
  `xtask config-set --key meta.client_secret --type secret --value-from-stdin`.
  Single-line; same presence-only/audit rules as every secret.

Boot-without-keys: identical contract — boot always succeeds;
both halves required for readiness; start 503
(`meta login unavailable`) / callback `oauth_unavailable` +
WARNING line when unset. Passkey/Google/Apple unaffected.

Rotation: Settings → Basic → reset the App secret → set-new
PUT → restart → verify a staging login. WARNING: Meta
secret reset is IMMEDIATE — no overlapping-secret grace
like Apple's multi-key model, so reset-then-verify in one
maintenance window (brief Meta-login outage is inherent;
other methods unaffected). KEK rotation does NOT rotate
the app secret; do both on exposure.

### Staging acceptance (Meta slice)

Preconditions: staging redirect URI registered; testers
added while in Development mode; keys pasted + restarted;
fixtures synthetic-only.

1. Logged-out login creates account + session: Meta button
   → authorize → lands logged in; `methods: ["meta"]`;
   `identity_link(method='meta',
   provider_sub=<app-scoped id>)` + `account` rows created.
   The stored sub is the app-scoped id (never a global id),
   with a NULL email hash.
2. Login again resolves the SAME account.
3. Meta NEVER auto-links and NEVER prompts: a fixture with
   a same-address Apple/Google account stays separate and
   NO `link_required` fires (always-fresh-account — the
   explicit link flow is the only join path).
4. Link flow from Security (fresh session ≤10 min) →
   methods list shows the meta entry; `method_linked`
   audits the id HASH.
5. Unlink meta non-last → 200 (fresh-session proof
   accepted); sole meta method → 409; account NOT locked.
6. Replayed/expired state → `oauth_expired`; stripped
   binding → `oauth_failed`; cancel at Meta →
   `oauth_denied`.
7. `debug_token` app-id mismatch (staging cfg pointed at
   the wrong App ID) → `oauth_failed`, no session.
8. Keys wiped on a scratch DB → start 503s, other methods
   unaffected; restore + restart → green.
9. Rotation drill on staging: reset app secret in portal →
   set-new → restart → login green (documents the inherent
   blip window).
10. Audit rows carry hashes only — no raw app-scoped id,
    token, or secret bytes.
11. `cargo xtask ci` green; `web-budget` green (221KB gate;
    no Meta JS SDK); secrets-scan clean.

### Rollback

Additive-only, no numbered migration (same paragraph as
Apple with `method='meta'`).

## Staff passkeys (Phase 4)

Keyless slice: staff enrolment adds **no config keys**.
Client certs live as files under `<store>/admin-certs/`
(see the CA procedure in
[staff-enrolment.md](staff-enrolment.md) §1), invites as
rows in the `staff_invite` table (token hashes only, 7-day
TTL, single-use). Nothing to paste into `/admin/config`.

The full operator surface — invite ceremony, staff login,
offboarding, step-up pilot, staging checklist, rollback —
is the [staff-enrolment runbook](staff-enrolment.md)
(spec: [customer-auth.md](../specs/customer-auth.md) §5).

## Magic link / mailer (`mailer.*`, Phase 5)

Spec: [customer-auth.md](../specs/customer-auth.md) §12
(fallback-only, token shape, relay rules, no-enumeration).
Flow: server-side request → transactional email → single-use
15-min token (256-bit CSPRNG hex, SHA-256 hash at rest) bound
to the exact email; consume mints a normal customer session
with a 7-day fallback window (1h idle inside it) + a
`magic_login`/`fallback` audit. The emailed link carries the
token in the URL **fragment** (`#t=`), never the query —
fragments never reach server logs. No mailer JS anywhere
(send path is server-side).

**Sender decision (author, OPEN-25 resolved): Gmail SMTP for
andrii.ai initially.** Host/port/credentials/From stay plain
config rows so a later slice swaps in our own sender once IP
reputation is built — no code change for the swap. TLS policy
is fixed by the implementation: STARTTLS required (Gmail
`smtp.gmail.com:587`); no plaintext SMTP, no mode key.

### Gmail setup click-path

Extends [auth-playbook.md](../auth-playbook.md) §5 (verify
exact clicks in the current UI):

1. Sending identity: the andrii.ai mailbox that will send
   (this exact address becomes `mailer.from` AND the
   envelope sender). Sending limits apply per mailbox
   (Workspace daily caps) — fallback-only volume fits
   comfortably; check the Admin console quota if sends
   start failing.
2. Google Account → **Security → 2-Step Verification**
   (must be ON) → **App passwords** → create one named
   e.g. `handbook-magic-link` → copy the 16-character
   password ONCE (this IS `mailer.password`). This is an
   App Password — never a mailbox password, never OAuth.
   Revoking it in the same screen kills the sender
   instantly (rotation drill step 11 below).
3. `mailer.host` = `smtp.gmail.com`, `mailer.port` =
   `587`, `mailer.username` = the full sending address
   (same as `mailer.from`), `mailer.provider` = `smtp`.
4. SPF/DKIM for andrii.ai: Gmail sends from Google infra,
   so delivery authenticates under Google's envelope —
   but the From domain SHOULD still carry SPF/DKIM/DMARC
   records per the playbook §5 checklist (providers score
   the From domain too). Verify with the staging drill:
   request a link, read the raw headers in the test
   inbox — `Authentication-Results` should show dkim=pass
   (google) at minimum.
5. Apple private-relay sender registration (REQUIRED the
   moment magic links can go to `@privaterelay.appleid.com`
   addresses): developer.apple.com → Identifiers → the
   Services ID → Sign in with Apple → Configure → sender
   registration — register the `mailer.from` address, or
   Apple bounces relay-address mail provider-side
   (indistinguishable by design; staging item 5 proves it).

### The seven keys

- `mailer.provider` — type `text`: `smtp` (prod) or `file`
  (dev — writes one 0600 file per message under `{store}/mail/`
  instead of sending; local dev sets this one row explicitly:
  `xtask config-set --key mailer.provider --type text --value file`).
  Unset/unknown → warn + magic login disabled
  (fail closed, loud 503 — fresh stores never silently file
  real users' links instead of mailing them).
- `mailer.host` — type `text` (SMTP hostname;
  `smtp.gmail.com` today, our relay host after the swap).
- `mailer.port` — type `text` (ports are text rows, same as
  `net.port`; must parse as a nonzero u16, else disabled).
- `mailer.username` — type `text` (SMTP AUTH username; the
  full Gmail address today).
- `mailer.password` — type `secret` (write-only): the Gmail
  App Password today, the relay credential after the swap.
  Paste in `/admin/config` secret row (never echoes), or
  seed headless:
  `xtask config-set --key mailer.password --type secret --value-from-stdin`.
- `mailer.from` — type `text` (header/envelope From; this
  exact address is what gets relay-registered above).
- `mailer.dir` — type `text` (default `mail`, store-root
  relative; file-provider outbox — inert under `smtp`).

Step-up note: `PUT /admin/config/{key}` requires fresh
step-up (staff-enrolment §5), so log in fresh immediately
before pasting keys. Every PUT audits key+version only; a
restart applies new rows (read-once boot).

Boot-without-keys: missing/partial SMTP set → `None`
(lenient overlays + warning, same as every OAuth slice) —
boot ALWAYS succeeds. The request endpoint fails closed with
`503 {"error": "magic login unavailable"}` (a GLOBAL outage
code, identical for known/unknown addresses — no
enumeration), nothing is sent, and boot logs one
`WARNING: config mailer.* … — magic login disabled` stderr
line. Passkey/OAuth flows unaffected. Consume correctness
never depends on mailer liveness (the token is
self-sufficient).

Rotation: set-new PUT overwrites the ciphertext; restart;
verify a staging request→inbox→consume BEFORE revoking the
old credential (revoke-after-verify order). Gmail App
Passwords have NO overlap grace (revoke is immediate, same
warning as Meta's reset) — create-new → set-new → restart →
verify → revoke-old in one window; brief magic-link outage
is inherent, other methods unaffected. KEK rotation re-wraps
rows — it does NOT rotate the App Password; do both on
suspected exposure. `from`/host/port/username rotate by
overwrite + restart. Future own-sender swap: overwrite the
four rows + restart + staging drill — no deploy needed.

### Staging acceptance (magic slice)

Preconditions: staging origin reachable; test inbox
provisioned (synthetic addresses only, never real
staff/customer mailboxes); `mailer.from` sender verified per
the click-path above; keys pasted + service restarted;
fixtures synthetic-only.

1. Happy path: logged-out request for a test address →
   200 → mail arrives → link consumed from a DIFFERENT
   browser (no CSRF cookie on first touch — the
   Origin-only design) → session minted; `GET /auth/me`
   200; `account.fallback_until` set (~7d out);
   `magic_login`/`fallback` audited.
2. Unknown email: request for an unregistered address →
   byte-identical 200, zero mail, zero token rows, zero
   new audits (no enumeration).
3. Token rules: replay → 410 (audit `replay`); link older
   than 15 min → 410 (audit `magic_expired`); malformed
   token → 400; garbage well-formed token → 410 (audit
   `invalid`) with a body byte-identical to replay.
4. Rate limits: 4th request for one address in an hour →
   429 while a different address still 200s (per-target);
   burst past the per-IP caps → 429s on both endpoints.
5. Relay delivery: request for a `@privaterelay.appleid.com`
   test address → delivered (proves sender registration)
   → consume works; the relay row resolves to its own
   owner only (rule 3 — never string-matched across
   accounts).
6. Fallback window: magic login sets `fallback_until`;
   advance the session idle past 1h → 401; before → 200;
   passkey/OAuth login clears `fallback_until`
   (`clear_fallback` — asserted, not assumed); second
   magic login re-arms the 7d window.
7. Verified-email join: magic login for an address with an
   unverified `method='email'` hint row flips it to
   verified — a LATER OAuth login for the same address
   then fires `link_required` (rule-2 chain red→green).
   Magic NEVER creates `identity_link` rows (flip-only):
   the methods list and unlink gate are byte-identical.
8. Ambiguous duplicates: two accounts sharing one address
   → consume 410s (audit `ambiguous`), accounts stay
   separate; locked-mid-flight → 410-shaped (audit
   `account_locked`); staff-account → 410 (audit `role`).
9. Keys wiped on a scratch DB → restart → request 503s
   with the exact `magic login unavailable` string for
   known AND unknown addresses, passkey/OAuth unaffected;
   restore + restart → green.
10. Audit rows carry hashes only — no raw token, email, or
    credential bytes; `magic_token` holds hashes only;
    access logs show no token (fragment design — grep the
    request spans for `#t=` and `token=`).
11. Rotation drill: create-new App Password → set-new →
    restart → request→inbox→consume green → revoke-old.
12. `cargo xtask ci` green; `web-budget` green (223KB gate;
    server-side send — no mailer JS in either bundle);
    secrets-scan clean (no password in repo, logs, or
    fixtures).

### Rollback

Additive-only, no numbered migration: new keys are ROWS in
the generic config KV table; `magic_token` is untouched by
the old binary (it never queries it); no `identity_link`
method values were added (flip-only), so the old unlink
gate and methods list behave identically. Rollback =
previous binary + previous unit file. Residuals: outstanding
tokens sit unconsumable until re-upgrade, bounded by the
15-min TTL (worst case the user re-requests); `fallback_until`
values set by the new binary are ignored by the old binary's
role-only `Session::valid` (sessions stay usable — no lockout,
no manual cleanup).
