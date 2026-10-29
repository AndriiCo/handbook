# Auth Playbook (operator checklist + linking rules)

- Retrieval click-paths (§§2–4) moved into the setup wizard
  ([secure-config.md](specs/secure-config.md) §9,
  `web/src/lib/admin/setup-guides.js`) — first boot pastes
  values there, not from here. The flow, linking, and
  callback rules below stay normative.
- Status: decided direction (prompt 008 Q4); verify each portal's
  exact clicks in its current UI before executing.
- Customer login: **passkey first, Sign in with Apple, Sign in with
  Google, Login with Meta, email click-to-login (magic link) only as
  fallback** for customers who cannot use the first four (Meta added
  prompt 009). Customers may link any number of methods in their
  profile, per the linking rules in §6. Staff/admin login:
  **passkey (UV required) plus mTLS client cert** (decided
  2026-10-28; password+TOTP rejected) — see the
  [staff-enrolment runbook](ops/staff-enrolment.md) and
  [customer-auth.md](specs/customer-auth.md) §5.
- Companion security rules: [SECURITY.md](SECURITY.md) §2
  (Auth/session) and §1 (verdicts 1–8) stay binding.

## 1. Passkeys (primary)

- Relying Party ID: `andrii.ai` (apex — covers `andrii.ai` and
  subdomains per WebAuthn rules; confirm exact RP-ID scoping for the
  admin host at scaffolding, since admin is a separate origin).
- Registration: after choosing passkey, `navigator.credentials.create`
  with resident key (`preferred`), user verification (`preferred`,
  `required` for admin-linking step-up). Login:
  `navigator.credentials.get` (conditional mediation for autofill).
- Server: Rust WebAuthn verification (verify library choice and API
  at scaffolding), challenge single-use + short-lived, credential
  IDs stored per account, sign-count tracked.
- Apple/Google passkey sync is automatic via platform (iCloud
  Keychain / Google Password Manager) — no per-platform config
  beyond the RP ID. User deletion/revocation on our side = delete
  the credential row + kill sessions + audit; platform-side
  deletions surface as "credential not found" at next login, which
  must route to re-registration, never to an error dead-end.

## 2. Sign in with Apple (portal checklist)

1. Apple Developer portal → Certificates, Identifiers & Profiles →
   enable **Sign in with Apple** on the App ID.
2. Register a **Services ID** for web; verify `andrii.ai` domain
   ownership; register return URLs
   (`https://andrii.ai/auth/apple/callback`).
3. Create a Sign-in-with-Apple **private key** (download once,
   store with secrets, never in repo).
4. Register the **notifications endpoint** URL in the portal
   (single endpoint receives all event types as Apple-signed JWTs):
   `https://andrii.ai/auth/apple/events`.
   (Endpoint registration + JWT event shape per WWDC22 "Enhance
   your Sign in with Apple experience"; event types below per
   Apple's server-to-server notification spec and ecosystem
   implementations — confirm exact strings in the current portal.)
5. Server: validate `identityToken` against Apple's JWKS on every
   login (issuer, audience = Services ID / App ID, expiry, nonce).
   Implementation (Phase 3a, spec §10.1): NO nonce — the
   `state`+binding pair covers CSRF and the id_token arrives
   via our own server-side token call, never through the
   browser.
6. Private-relay sender registration (REQUIRED before any
   magic link can reach a `@privaterelay.appleid.com`
   address): Services ID → Sign in with Apple → Configure →
   register each sending address (Phase 5, spec §12.7).
   Receiving relay addresses at login needs no registration —
   sending does.

Event → action table (endpoint verifies Apple's JWT signature first,
rejects anything else with 403 + audit):

| Apple event | Meaning | Our action |
|---|---|---|
| `email-enabled` / `email-disabled` | relay forwarding toggled | update contact flag; never email a disabled relay |
| `consent-revoked` | user stopped using Apple ID with us | disable the Apple identity link, kill its sessions, notify user via remaining channel, audit |
| `account-delete` | user deleted their Apple account | disable the identity link, start data-retention handling per policy, audit |

## 3. Sign in with Google (portal checklist)

1. Google Cloud Console → project for the handbook → OAuth consent
   screen (External, app name, support email, authorized domain
   `andrii.ai`); add test users until verified, then submit for
   verification if required by scope (profile+email only keeps this
   minimal).
2. Credentials → **OAuth client ID (Web)**: authorized JavaScript
   origins `https://andrii.ai`; authorized redirect URIs for the
   code flow (`https://andrii.ai/auth/google/callback`).
3. SPA uses Google Identity Services button (FedCM-aware); server
   validates the ID token against Google's certs (issuer, audience
   = client ID, expiry, nonce).
4. Cross-Account Protection (RISC) — same Cloud project: enable the
   **RISC API**, create a **service account**, build the event
   receiver endpoint
   (`https://andrii.ai/auth/google/risc`), and
   register it via `POST https://risc.googleapis.com/v1beta/stream:update`
   with push delivery to that URL (per
   https://developers.google.com/identity/protocols/risc).

Event → action table (receiver validates Google's SET signature
first):

| Google RISC event | Our action |
|---|---|
| `…/sessions-revoked` | kill the linked sessions, require re-auth, audit |
| `…/account-credential-change-required` | force step-up on next login, notify user, audit |
| `…/account-disabled` | disable the Google identity link + sessions, audit |
| `…/account-enabled` | clear the disabled flag (link stays disabled until user re-auths), audit |
| `…/verification` | handshake only, no user action |

## 4. Login with Meta (portal checklist)

"Login with Meta" for third-party websites is **Facebook Login**
(per https://developers.facebook.com/docs/facebook-login/web/,
inspected 2026-10-26).

1. Meta App Dashboard → create App ID (website) → **Add a Product
   → Facebook Login → Settings → Client OAuth Settings** → enter
   redirect URIs, including
   `https://andrii.ai/auth/meta/callback`.
2. Set **Login with JavaScript SDK** to yes; add `andrii.ai` to
   **Allowed Domains for JavaScript SDK** (tokens only return to
   authorized domains; HTTPS only). Implementation (Phase 3b,
   spec §11): the SPA never loads the Meta JS SDK — the flow
   is a server-built redirect, so this toggle is inert; leave
   it off.
3. Permissions: `public_profile` + `email` need no app review and
   are auto-granted — request nothing more. Implementation
   (Phase 3b, spec §11): NO email scope is requested and no
   email is ever fetched — the flow is sub-only by design
   (Meta email never matches), so there is no review surface
   at all.
4. Register the **Deauthorize Callback URL** and the **Data
   Deletion Request Callback URL** (both HTTPS, both in
   Facebook Login > Settings — exact field locations per
   current dashboard, verify in portal):
   `https://andrii.ai/auth/meta/deauthorize`
   and `https://andrii.ai/auth/meta/delete-request`
   (live: verifies `signed_request`, purges, returns
   `{url, confirmation_code}` per spec §11.9).
   Also register the **Privacy Policy URL**
   `https://andrii.ai/privacy` (App Settings; same URL
   serves the Google/Apple consoles).
5. Server: exchange the code server-side, keep the App Secret in
   secrets storage only, and validate tokens/calls per the
   current Graph API docs at implementation (debug_token /
   appsecret_proof pattern — verify exact current form; never
   trust a client-supplied user id).

Event → action table (verify Meta's `signed_request` signature
with the App Secret first; reject anything else 403 + audit):

| Meta signal | Meaning | Our action |
|---|---|---|
| Deauthorize callback | user removed the app / revoked access | disable the Meta identity link, kill its sessions, notify via remaining channel, audit |
| Data deletion request | user asked Meta to delete their data with us | delete Meta-sourced data, respond with `{url, confirmation_code}` status tracker, audit |

## 5. Email click-to-login (fallback only)

- Offered only when passkey/Apple/Google/Meta all fail for the customer.
  Token: 256-bit CSPRNG, single-use, 15-minute expiry, bound to the
  exact email address; consuming from a different browser is fine
  (that's the point) but the token never acts as a persistent
  credential — it mints a normal short session.
- Sending: dedicated transactional subdomain (e.g.
  `mail.andrii.ai`) with SPF/DKIM/DMARC passing. Implementation
  (Phase 5, spec §12.8, OPEN-25 resolved): Gmail SMTP for
  andrii.ai initially (App Password in a sealed row); the
  host/port/credentials/From rows stay configurable so a later
  slice swaps in our own sender. Rate-limit sends per email +
  per IP; identical responses for known/unknown addresses (no
  enumeration).
- Fallback logins get a `auth_fallback=true` audit mark and
  tighter idle timeout until the customer registers a passkey.
  Implementation (Phase 5, spec §12.6, OPEN-2 resolved): 7-day
  window + 1h idle, enforced via `session_usable`; re-armed by
  each magic login, cleared by passkey/OAuth login.

## 6. Account-linking rules (the anti-takeover core)

Multiple methods must never let an attacker claim someone else's
account. Rules (enforced server-side, tested with fixtures):

1. Identities table: one account ↔ many verified identities
   (passkey credential, Apple `sub`, Google `sub`, Meta app-scoped
   user id, email address). The profile page lists linked methods
   and offers link/unlink for each.
2. Link on **verified-email equality only**: Apple/Google emails
   arrive verified; magic-link email becomes verified by
   construction. Unverified, empty, or changed emails never link.
3. Apple **private-relay** addresses are distinct identities — they
   link to an account only through the explicit link flow (rule 4),
   never by string match (relay ≠ real address). Meta-supplied
   email is likewise **never trusted for auto-linking** — Meta
   identities join an account only via the explicit link flow.
4. No silent auto-merge: adding a method to an existing account
   requires an **explicit link flow** — active session on the old
   method + successful auth on the new method + step-up
   (user verification) — then audit.
5. First-login creates a fresh account; it never attaches to an
   email-matched account created by another method (kills
   pre-account takeover). Duplicate-email state surfaces as "an
   account with this email exists — sign in with X to link", never
   as a merge.
6. Unlinking requires a remaining verified method + re-auth; the
   last method cannot be removed. Revoked/deleted provider
   identities (signals in §2–§4) auto-disable that link; if it was
   the last link, the account locks pending manual recovery.
7. Every link/unlink/signal action writes an `audit` row with
   actor + method + provider `sub`/relay hash (never raw PII).

## 7. Telnyx (purpose TBD — confirm before configuring)

The author listed Telnyx among platforms but its role in this
project is not yet fixed. Best guess: voice/SMS numbers for the
P1 call console (placing validation calls) — **confirm**. Do not
configure anything until the purpose decision lands; when it does,
this section gains the number-provisioning, recording-consent, and
credential-rotation checklist.

## 8. Verification checklist (staging, before prod)

- [ ] Passkey register → logout → autofill login (macOS Safari,
  Chrome, iOS) green; revoked-credential path routes to
  re-registration.
- [ ] Apple login green incl. Hide-My-Email; relay address never
  merges by string match (fixture test).
- [ ] Google login green; RISC receiver verified via Google's
  `verification` event; each event in §3 table tested with a
  synthetic SET.
- [ ] Apple events endpoint tested with Apple-signed sample JWTs
  (each row in §2 table); forged JWT rejected 403 + audited.
- [ ] Magic link: single-use, 15-min expiry, no enumeration
  (identical timing/response for unknown addresses).
- [ ] Meta login green; Meta email never auto-links (fixture:
  same-address Apple account stays separate until explicit link
  flow); deauthorize + data-deletion callbacks tested with
  Meta-signed samples (link disabled, data deleted, tracker URL +
  confirmation code returned); forged `signed_request` rejected
  403 + audited.
- [ ] Link/unlink matrix tested: every pair of methods, last-method
  removal refused, pre-account-takeover fixture red→green.
- [ ] Secrets (`META_API_KEY`, Apple private key, Google client
  secret if any, session secret, CA key) present only in the
  root-only env file; `xtask ci` greps clean for secret patterns.
