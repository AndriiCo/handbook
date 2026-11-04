# Customer Auth: passkeys + linked methods (draft for review)

- Status: draft (for author review; Phase-1 implementation alongside)
- Date: 2026-10-28
- Grounding: [auth-playbook.md](../auth-playbook.md) (§1 passkeys, §6 linking rules), [spa-separation.md](spa-separation.md) §1.4 (sessions/cookies/roles), [SECURITY.md](../SECURITY.md) §1–§2, `crates/auth/src/` (`lib.rs` sessions+roles, `login.rs` policy, `csrf.rs`, `ws.rs`), `crates/web` session/cookie handling, and webauthn-rs 0.5.5 API (builder + ceremony shapes + error variants, verified against vendored sources).
- Out of scope: the magic-link sender (playbook §2–§5 portal checklists stand; this spec defines the tables and link/unlink seams they plug into). Google OAuth is §9 (Phase 2); Apple + Meta are §§10–11 (Phase 3). Staff enrolment endpoints are Phase 4 (spec §6 here, tables land with the code).

## 0. Decisions embodied

1. Library: `webauthn-rs` 0.5.5. Features: default (`attestation`) + `danger-allow-state-serialisation` + `conditional-ui` (discoverable login, §2.5). The `danger` feature is our documented safe case: ceremony state persists server-side in SQLite (single-use + 5-min expiry), never in cookies — exactly what the crate docs bless. No `ring` from this stack (verified: crypto backend is openssl; the only `ring` in the graph is the pre-existing `rcgen` test-cert path).
2. OpenSSL is vendored (`openssl-sys` `vendored` feature, unified from the web crate): hermetic local builds (Mac system OpenSSL is v4, which openssl-sys may reject) and working `cargo zigbuild` cross builds (no target system libs). Precedent: the workspace already builds C deps (rusqlite bundled, aws-lc-sys).
3. RP ID `andrii.ai` serves **both** `https://andrii.ai` and `https://admin.andrii.ai`: `WebauthnBuilder::new(rp_id, rp_origin)` requires only that rp_id be an effective domain of rp_origin, and `.allow_subdomains(true)` accepts subdomain origins (verified in vendored builder docs + `allow_subdomains` semantics). Belt-and-braces: also `.append_allowed_origin(https://admin.andrii.ai)`. Browser-side RP-suffix rule still needs a live ceremony on each origin (OPEN-1).
4. Passkey ceremonies live under `/apostille/api/v1/auth/…` on **both** branches (public router for customers now; admin router for staff enrolment/login in Phase 4). Sessions, cookies, roles and CSRF are **reused unchanged** from `crates/auth`; the admin branch adds the IP allowlist (§5).
5. Counters follow the crate, not a bespoke column: `AuthenticationResult` → `Passkey::update_credential` → re-persist `passkey_json` when `needs_update()`. No `counter`/`backup_*`/`uv_performed` columns (YAGNI — additive later if staff enrolment needs UV proof persisted).
6. Rate limits are an in-memory mutex bucket (single server; no new table): login/register finish 10/min/account + 60/min/IP, start endpoints 30/min/IP, atomic check-and-decrement.

## 1. Database tables

All in the existing metadata DB (same file as `session`/`identity_link`; single-writer rule: the web binary runs `auth::migrate_auth_tables` at startup — extend that function, do not fork DDL).

### 1.1 Existing tables reused (no changes)

From `auth::migrate_auth_tables` (`crates/auth/src/lib.rs`):

- `session(id, user_id, role, customer_ref, created_at, last_seen_at, expires_at, ip_hash, ua_hash)` + `idx_session_user`.
- `identity_link(user_id, method, provider_sub, email_hash, verified_email, is_relay, disabled_at)`, PK `(user_id, method, provider_sub)`. Playbook §6 rule 1 lives here: one account ↔ many verified identities.
- `admin_cert(fingerprint, cn, issued_at, expires_at, revoked_at, cert_path)`.
- `audit(at, actor, action, entity, entity_id, …)` (existing; every §6 rule-7 write goes here).

### 1.2 `account` (NEW)

`user_id` is currently an unanchored string. This table anchors it.

```sql
CREATE TABLE IF NOT EXISTS account(
  user_id TEXT PRIMARY KEY,          -- opaque: uuid v4 hex, also the WebAuthn user handle
  role TEXT NOT NULL DEFAULT 'customer', -- customer | staff | admin (auth::Role wire form)
  customer_ref TEXT UNIQUE,          -- nullable until first customer action needs one
  created_at INTEGER NOT NULL,
  locked_at INTEGER,                 -- set when rule 6 last-link revocation locks the account
  step_up_at INTEGER,                -- last UV/step-up success (unix); rule 4 + re-auth proofs read this
  fallback_until INTEGER             -- magic-link logins set auth_fallback window (playbook §5); NULL otherwise
) STRICT;
```

### 1.3 `webauthn_credential` (NEW)

One row per registered passkey. The `Passkey` struct from `finish_passkey_registration` serialises to JSON for storage (see decision 0.5).

```sql
CREATE TABLE IF NOT EXISTS webauthn_credential(
  credential_id TEXT PRIMARY KEY,    -- base64url-no-pad of CredentialID; also identity_link.provider_sub for method='passkey'
  user_id TEXT NOT NULL REFERENCES account(user_id),
  passkey_json TEXT NOT NULL,        -- serialised Passkey (public key material for login/start)
  label TEXT NOT NULL DEFAULT '',    -- user-given name ("MacBook Pro"), ≤64 chars
  created_at INTEGER NOT NULL,
  last_used_at INTEGER,
  revoked_at INTEGER                 -- soft delete; revoked creds are excluded from start_* lists
) STRICT;
CREATE INDEX IF NOT EXISTS idx_cred_user ON webauthn_credential(user_id);
```

Companion row: every credential insert also inserts `identity_link(user_id, 'passkey', credential_id, NULL, 0, 0, NULL)` in the same transaction (rule 1: passkey is one identity among many; rule 6 counts links, not credentials).

### 1.4 `webauthn_challenge` (NEW)

Server-side ceremony state (crate docs: MUST persist server-side or replay attacks weaken the system).

```sql
CREATE TABLE IF NOT EXISTS webauthn_challenge(
  id TEXT PRIMARY KEY,               -- opaque handle returned to the client (NOT the raw challenge)
  user_id TEXT,                      -- register/login account; NULL for discoverable until identify_* resolves it
  kind TEXT NOT NULL,                -- register | login | discoverable
  state_json TEXT NOT NULL,          -- serialised PasskeyRegistration / PasskeyAuthentication / DiscoverableAuthentication
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,       -- created_at + 300 (5 min); single-use
  consumed_at INTEGER                -- set on first finish attempt; reuse → 410
) STRICT;
CREATE INDEX IF NOT EXISTS idx_challenge_exp ON webauthn_challenge(expires_at);
```

Sweep: `DELETE FROM webauthn_challenge WHERE expires_at <= ?1` on every start call.

### 1.5 `staff_invite` (Phase 4 with the enrolment code)

```sql
CREATE TABLE IF NOT EXISTS staff_invite(
  token_hash TEXT PRIMARY KEY,       -- SHA-256 hex of the 256-bit invite token (cf. login::token_hash)
  role TEXT NOT NULL,                -- staff | admin
  cn TEXT NOT NULL,                  -- expected client-cert CN the invite binds to
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,       -- created_at + 7d
  consumed_at INTEGER,
  consumed_by TEXT
) STRICT;
```

## 2. HTTP endpoints

Base paths: public branch `/apostille/api/v1/auth/…`; admin branch `/apostille/api/v1/auth/…` (staff passkey enrolment/login only — Phase 4). All mutating endpoints require Origin check + double-submit CSRF. All DTOs use `#[serde(deny_unknown_fields)]` (SECURITY.md input-validation rule).

Wire note: challenge responses (`CreationChallengeResponse`, `RequestChallengeResponse`) and credential payloads (`RegisterPublicKeyCredential`, `PublicKeyCredential`) serialise per the crate's serde impls and pass through verbatim inside our envelopes.

### 2.1 `POST /auth/passkey/register/start`

Two modes: (a) logged-out first passkey (creates the account at finish), (b) logged-in add-passkey (explicit link flow, rule 4).

Request: `{"display_name": "Andrii", "label": "MacBook Pro"}` — `display_name` required in (a), ignored in (b); `label` optional, ≤64 chars. Mode (b) requires session cookie.

Device cap (decision 54): 5 live passkeys per account (counts live `webauthn_credential` rows only — never `identity_link`); self-service remove frees a slot. Staff has no add-key path, so the cap binds on the customer branch only (staff device changes stay admin-assisted per decision 50).

Success `200`: `{"challenge_id": "<opaque>", "options": { "<CreationChallengeResponse>" }}`. Server: `start_passkey_registration(user_uuid, name, display, exclude)` where `exclude` = existing non-revoked credential IDs (mode b) or `None` (mode a); persist state (`kind='register'`).

Errors: `400` bad display_name/label; `401` mode-(b) without session; `403` CSRF/Origin fail; `409` device cap reached in mode (b) — remove a passkey first; `429` rate-limited; `500` store unavailable.

### 2.2 `POST /auth/passkey/register/finish`

Request: `{"challenge_id": "<opaque>", "credential": { "<RegisterPublicKeyCredential>" }, "email?": "…", "label?": "…"}` (client echoes the optional email + label from start; mode (a) stores the email as an unverified `identity_link` row for login resolution + Phase-3 recovery).

Server: load + consume challenge (missing/expired/consumed → `410`); `finish_passkey_registration(&cred, &state)`; assert credential ID globally unregistered (crate docs MUST — conflict → `409`); insert `account` (mode a) + `webauthn_credential` + `identity_link` in one transaction (mode-(b) inserts run capped — the live-count check sits inside the same transaction, so racing finishes cannot both pass); mint session (mode a) or audit (mode b); audit row (`passkey_registered`, no PII).

Success `200` (mode a): `{"user_id": "…", "methods": ["passkey"]}` + `Set-Cookie`. Mode (b): `{"linked": true, "credential_id": "…", "methods": […]}`.

Errors: `410` challenge invalid/consumed; `409` credential already registered; `409` device cap reached in mode (b) (`passkey_cap_refused` audit); `422` verification failed (log variant, return generic `{"error": "verification failed"}` — never leak crate internals).

### 2.3 `POST /auth/passkey/login/start`

Non-discoverable login. Request: `{"user_id": "…"}` (unknown user / zero creds → `401` generic, no enumeration).

Success `200`: `{"challenge_id": "…", "options": { "<RequestChallengeResponse>" }}` via `start_passkey_authentication(&[Passkey…])` (deserialised from `passkey_json`); state (`kind='login'`).

### 2.4 `POST /auth/passkey/login/finish`

Request: `{"challenge_id": "…", "credential": { "<PublicKeyCredential>" }}`.

Server: consume challenge; `finish_passkey_authentication(&cred, &state)` → `AuthenticationResult`; if `needs_update()`, `update_credential(&res)` + re-persist `passkey_json`; update `last_used_at`; clear `fallback_until`; `create_session` + cookie; rotate CSRF token (SECURITY.md: rotate on login); audit (`passkey_login`).

"Credential not found" (platform-side deletion, playbook §1): unknown credential ID → `404 {"error": "credential unknown", "action": "re-register"}` — the SPA routes to register, never an error dead-end.

Other errors: `401` verification failed (generic); `403` account locked (`locked_at` set — manual recovery); `410` challenge bad; `429` + backoff.

### 2.5 Discoverable login (conditional mediation / autofill)

Built (P431b) with webauthn-rs 0.5.5's `conditional-ui` feature (empty feature, no new crates): `From<Passkey>/From<&Passkey> for DiscoverableKey` plus `start/identify/finish_discoverable_authentication`. No new routes: §2.3/§2.4 serve both shapes.

- Start: `login/start` with an empty `account` (`{"account": ""}`, the login page's autofill probe) returns `start_discoverable_authentication()` options (empty `allowCredentials`, `mediation: "conditional"`). State kind `'discoverable'`, `user_id` NULL.
- Finish: `login/finish` takes the challenge as `'login'` or else `'discoverable'`. Discoverable: `identify_discoverable_authentication` yields the user handle and credential id; the credential's owner (live `webauthn_credential` row) must equal the user handle, else `401`; then `finish_discoverable_authentication` against that one passkey, and the §2.4 tail (counter, fallback clear, session, audit `passkey_login`). Unknown credential id → the same `404 re-register` shape.
- Registration asks for a discoverable credential (`residentKey: "preferred"`, customers keep the email path for authenticators that cannot store one). Identifier-first login (§2.3) stays for older non-discoverable keys.

### 2.6 Profile: list / link / unlink

- `GET /auth/profile/methods` (session required) → `{"methods": [{"method": "passkey", "id": "cred…", "label": "…", "disabled": false}, …]}`. Reads `identity_link` + credential labels.
- Passkey linking reuses §2.1/§2.2 mode (b) (session + fresh step-up: `step_up_at` within 10 min or a session minted within 10 min); no separate link routes in Phase 1. OAuth-phase link routes (`POST /auth/profile/link/start|finish`) follow the same rule-4 shape when non-passkey methods land. Every link audits `method_linked` with provider `sub` hash (rule 7 — `hash_str`, never raw PII).
- `POST /auth/profile/unlink` — `{"method": "passkey", "id": "…", "reauth_proof": "<challenge_id of a just-completed ceremony>"}`. Server: `unlink_permitted(remaining, reauthed)` (`login.rs`); `remaining` counts non-disabled links excluding the target. Last method → `409 {"error": "last method cannot be removed"}`. Success: set `identity_link.disabled_at` (+ `webauthn_credential.revoked_at` for passkeys), audit `method_unlinked`. Revoked provider identities auto-disable via `signal_action` handlers (later slices); if the disabled link was the last verified one, set `account.locked_at` (rule 6 tail) + audit `account_locked`.

### 2.7 Session endpoints

- `POST /auth/logout` — `delete_session` + clear cookie + audit. Idempotent `200`.
- `GET /auth/me` — reads `PUB_COOKIE`, `get_session`, `Session::valid(now)`, `touch_session`, returns `{"user_id", "role", "methods": […]}`. `401` on missing/expired. (The DEMO `/api/v1/me` stays until Phase 4 removes the dev seam; no page routes to it after Phase 1.)

## 3. Playbook §6 rules → enforcement points

| Rule | Enforced in |
|---|---|
| 1 — one account ↔ many identities | schema + register/link handlers (`identity_link` PK; companion row in-txn) |
| 2 — verified-email equality only | `login::email_usable_for_matching` (OAuth/magic handlers, later slices) |
| 3 — relay distinct; Meta email never links | `login::email_usable_for_matching` + `is_relay` flag + explicit flow only |
| 4 — explicit link flow | `login::explicit_link_permitted(step_up_ok)` + link handlers (old session + new ceremony + `step_up_at` fresh) |
| 5 — fresh account, no auto-merge | `login::decide_unknown_sub` (passkey-first registration always creates — no email involved) |
| 6 — unlink needs remaining + re-auth; last-link revocation locks | `login::unlink_permitted` + `signal_action` + `account.locked_at` |
| 7 — audit every link/unlink/signal | handlers + `audit` rows with hashes, never raw PII |

## 4. Session / cookie reuse (no new mechanisms)

- Cookies: `auth::PUB_COOKIE` (`__Host-hb_pub`) public, `auth::ADM_COOKIE` (`__Host-hb_adm`) admin; `auth::session_cookie` / `auth::cookie_value`; CSRF `hb_csrf` + `x-csrf` via `csrf::mint_token` / `csrf::tokens_match`, rotated on login and role change.
- Lifecycle fns reused verbatim: `mint_session_id`, `create_session`, `get_session`, `touch_session`, `delete_session`, `delete_expired_sessions`, `revoke_user_sessions`.
- Timeouts: customer idle 24h + absolute 12h; staff/admin idle 30m + absolute 8h (SECURITY.md reconciled). Enforced by `Session::valid` in the extractor.
- Origin: `csrf::origin_allowed` (exact origins, no ports; localhost only with the dev seam). Every mutation: Origin + double-submit.
- WS unchanged (session cookie + Origin before subscribe; `ws::authorize`).

## 5. Staff passkeys

Staff method: **passkey (UV required) from an address in `admin.allow_ips`**. The client certificate that first paired with the passkey (decided 2026-10-28) was replaced by the IP allowlist: there is no certificate to issue, present or revoke, and no `/admin/…` page path (the admin host is the console). Every admin request needs the admin host, a staff session and an allowed peer IP (§5.4).

All staff ceremony routes live on the **admin router only**, behind the admin host guard, the IP gate and Origin checks (the CSRF layer passes `/auth/*` through, so start handlers check Origin and finish handlers Origin + the CSRF echo). They reuse the customer machinery: `build_webauthn`, `RateBuckets`, `webauthn_challenge` state, `create_session`, CSRF rotation on login. Deltas from the customer flow: invite or setup-key gating, UV asserted at finish, `ADM_COOKIE`, role-aware accounts, separate challenge kinds (`staff_register`, `staff_login`, `staff_discoverable`, `staff_add`) so customer and staff states never cross.

### 5.1 Invitation lifecycle

1. **Mint.** From the console (**Staff → Invite**, admin role + fresh step-up, §5.5) or the CLI `xtask admin-invite --store <store> --role <staff|admin> --cn <name> [--days 7] [--actor …] [--origin <admin origin>]`. The CLI has no default store or origin: without `--origin` it prints the path (`/enrol?invite=…`) to open on the admin host. The production wrapper (`apostille-invite`) passes the store and the production origin. Prod invites are 24h (`--days 1`); the default is 7d.
   - `role` ∈ {`staff`, `admin`}. `cn` names the person (ASCII word characters, dots, dashes, ≤ 64); it labels the invite and the account's audit rows.
   - Token: 256-bit CSPRNG hex (`login::mint_magic_token`). The DB stores `token_hash` only. The console shows the URL once (built from the request's admin origin); the CLI prints it once. Audit `staff_invite_issue`.
2. **Present.** The invitee opens `/enrol?invite=<token>`. Missing, consumed and expired invites answer identically (`410`). The token stays in the address bar until enrolment succeeds (a reload or a cancelled passkey prompt can retry) and is stripped then.
3. **Consume (single-use, atomic).** Enrol-finish consumes with `consumed_at IS NULL` as the guard (zero rows → `410`) before persisting the credential. Enrol-start stashes the invite hash in its challenge state; finish compares.
4. **Withdraw.** A pending invite can be withdrawn from the Staff page (admin + step-up): it expires at once.

First staffer: no invite. The CLI-minted setup key (secure-config.md §9) replaces it while the setup window is open; `/config` holds the pasted key and runs the enrolment itself, and the first verified enrolment burns the key.

### 5.2 Enrolment endpoints

`POST /apostille/api/v1/auth/passkey/enrol/start` — request (`deny_unknown_fields`): `{"invite": "<64-hex>"}` or `{"setup_key": "setup_…"}` (exactly one), `"display_name"`, optional `"label"`. Server: validate the gate (`410` if invalid); `mint_user_id`; `start_passkey_registration` with `residentKey: "required"` (staff sign in without a user ID, §5.3); state kind `staff_register`. Success `200`: `{"challenge_id", "options", "role", "cn"}`. Errors: `400` shape; `403` Origin; `410` gate invalid; `429`; `503`.

`POST /apostille/api/v1/auth/passkey/enrol/finish` — request: `{"challenge_id", "credential", "invite" | "setup_key", "label?"}`. Server, in order: Origin + CSRF echo; `take_challenge(id, 'staff_register')`; gate re-check (invite hash matches the stashed one and is still live, or the setup key still verifies); invite consumed (`410` on race loss); `finish_passkey_registration` (`422` on failure); credential id globally new (`409`); the setup key burns on the first verified enrolment; `ensure_staff_account(user_id, role)` + `store_credential` (label = the given label, else the display name); `set_step_up`; session on `ADM_COOKIE`; CSRF rotated; audit `staff_enrolled`. Success `200`: `{"user_id", "role", "methods": ["passkey"]}`.

`POST /apostille/api/v1/staff/passkeys/add/start` and `…/add/finish` — a signed-in staffer adds another passkey (a second device): session + fresh step-up, the staffer's own passkeys excluded, state kind `staff_add` bound to the session's user, at most 5 live passkeys (`409` at the cap). Audit `staff_passkey_added`.

### 5.3 Staff/admin login endpoints

`POST /apostille/api/v1/auth/passkey/login/start` — `{"account": ""}` (or `{}`) starts a **discoverable** ceremony: the authenticator offers the staff passkeys it holds for this RP, no user ID typed (state kind `staff_discoverable`). `{"account": "<user_id hex>"}` keeps the identifier-first shape for keys enrolled before discoverable registration, and for the config step-up, which names the session's own user. Unknown/locked/credential-less accounts → `401 {"error": "unknown account or no passkeys"}`. Rate key `sl:{ip}` 30/min.

`POST /apostille/api/v1/auth/passkey/login/finish` — `take_challenge` as `staff_login`, else `staff_discoverable`. Discoverable: the credential's owner must equal its user handle. Unknown credential id → `404 {"error": "credential unknown", "action": "re-register"}` (re-register = a fresh invite from an admin). Then: explicit `user_verified()` assert (`401` + audit); account unlocked with role `staff|admin`, else `403 {"error": "use the customer login"}`; counter flow; `set_step_up`; session on `ADM_COOKIE` (staff timeouts: idle 30m, absolute 8h); audit `staff_login`.

`POST /apostille/api/v1/auth/logout` (admin host) — Origin-checked, idempotent: deletes the session behind `ADM_COOKIE`, clears it and the CSRF cookie, audits `staff_logout`. The console's **Sign out** sits in the sidebar footer.

`__Host-` cookies always carry `Secure` (browsers drop them otherwise). Browsers accept `Secure` cookies from `http://localhost`, so the plain-HTTP loopback boot keeps its sessions.

### 5.4 What admits a request

Every non-ceremony admin route extracts `AdminAuth`: a live session on `ADM_COOKIE`, `role.meets(Staff)`, and a peer IP inside `admin.allow_ips` (no proxy headers trusted; unset means loopback only). A stolen cookie used from another network fails the IP check (`403` naming the detected IP); an allowed network without a session fails at the cookie (`401`, and the console sends the browser to `/staff-login?next=<page>`). Browser navigations from a refused address get a static 403 page that names the address but not the config key. The ceremony handlers do not take `AdminAuth` (enrolment precedes any session); they sit behind the same host and IP layers.

### 5.5 Step-up for destructive actions

Destructive admin actions (config writes, restarts, staff invites and revocations, adding a passkey) require `step_up_at` within 600s (`STEP_UP_WINDOW_SECS`) or a session minted within 600s, else `403 {"error": "step-up required: log in again first", "step_up": "required"}`. The console confirms in place: one passkey ceremony for the session's own user (identifier-first §5.3 shape), then the same request again.

### 5.6 Staff management and offboarding

The console's **Staff** page (`/staff`) lists every staff/admin account with its passkeys (label, added, last used) and the pending invites. Admins can invite, withdraw an invite, revoke one passkey of any account and revoke a whole account; any staffer can add a passkey to their own account and revoke one of their own passkeys. Guards: a staffer cannot revoke their own account or their last live passkey, and the last unlocked admin cannot be revoked (`409` each).

Revoking an account (`POST /apostille/api/v1/staff/{user_id}/revoke`) runs the offboarding order, each step idempotent: kill every session, revoke every passkey (`revoked_at` + link `disabled_at`), lock the account. Audit `staff_revoked`. The CLI `xtask admin-offboard --store <store> --user <id> --cn <name>` does the same from the server shell (it also revokes any legacy `admin_cert` rows for the name).

### 5.7 Rate limits + PII/audit rules (per §6)

- Enrol start `30/min/IP` (`es:{ip}`); enrol finish `60/min/IP` (`ef:{ip}`) + `10/min/invite` (`ef:{token_hash-prefix-16}`); login start `30/min/IP` (`sl:{ip}`); login finish `60/min/IP` + `10/min/account` (`sf:…`).
- Logs/audit: actor + action + credential-ID **hashes** + invite-hash prefixes only. Never: raw invite tokens, setup keys, session IDs, challenges, cookies or key material.

### 5.8 Acceptance

Fresh boot: every console page lands on `/config`; one setup-key paste enrols the first passkey there; sign out; sign back in with no user ID. Invite → enrol green; invite replay → `410`; non-UV ceremony → `422`; customer credential on the admin login → `403`; staff credential on the apex login → `403`; refused IP → `403` page; expired session → `/staff-login?next=…` and back; revoke → sessions dead + passkeys revoked + account locked; `cargo xtask ci` green.

## 6. Rate limits, logging, PII

- Login/register finish 10/min/account + 60/min/IP with backoff; start endpoints 30/min/IP; in-memory mutex bucket, atomic check-and-decrement.
- Logs/audit: actor + action + entity + credential-ID/email **hashes** only; never session IDs, challenges, cookies, raw emails, or key material.

## 7. Verification (staging, before prod)

Extends playbook §8: register → logout → autofill login on Safari/Chrome/iOS; revoked-credential → re-register (never dead-end); cross-origin check — passkey registered on apex logs in on `admin.andrii.ai` and vice versa (OPEN-1 closes here); relay/Meta no-match fixtures; pre-account-takeover fixture red→green; last-method unlink refused; `cargo xtask ci` green.

## 8. OPEN items

| ID | Question | Owner | Blocks |
|---|---|---|---|
| OPEN-1 | ~~Live-ceremony proof that RP ID `andrii.ai` + `allow_subdomains(true)` + `append_allowed_origin(admin)` serves both origins end-to-end~~ RESOLVED: live-proof sprint runs immediately after enrolment unblocks; both-origin ceremony green required before admin-login sign-off (author, 2026-10-29) | CODE (staging) | §5 admin login, Day-6 gate |
| OPEN-2 | ~~Magic-link `fallback_until` window length + tighter idle value~~ RESOLVED by §12.6: 7d window + 1h idle (author-confirmed 2026-10-28) | Author | Phase 5 |
| OPEN-3 | ~~Staff enrolment if passkey stalls: fallback scope~~ RESOLVED: staff recovery stays admin-assisted only (fresh invite, never self-service); TOTP/backup codes rejected; break-glass via the sealed envelope only (decision 59); customer magic-link fallback is permanent, apex-only per §12 (author, 2026-10-29) | Author | §5 enrolment |
| OPEN-4 | Admin-branch Google OAuth: needed at all, or staff stays passkey + IP allowlist (§5)? Phase 2 ships apex-only either way | Author | admin login scope |
| OPEN-5 | tokeninfo→JWKS migration trigger (quota/latency at scale); ID-token local validation design | CODE | post-Phase-2 scale |
| OPEN-6 | ~~Playbook §3 touch-up~~ RESOLVED: playbook §3 rewritten (server-side code flow + tokeninfo, per-env clients) | CODE | portal setup |
| OPEN-7 | ~~Google RISC receiver phase + synthetic-SET fixtures (§3 table; `signal_action` rows already exist)~~ RESOLVED: built now — `POST /auth/google/risc` + `auth::risc` (§9.9), synthetic-RSA fixtures in-tree (author outcome keyed "OPEN-24", re-keyed 2026-10-29) | SEC | revocation handling |
| OPEN-8 | ~~Profile display for Google links (method-only vs masked email; raw email never stored)~~ RESOLVED: minimal profile now — method-only list, no masked email in v1; raw email never stored (author, 2026-10-29) | Author | profile UI |
| OPEN-9 | Apple `response_mode=query` + `email_verified`/`is_private_email` wire types confirmed at first real login | CODE (staging) | §10.1/§10.3 |
| OPEN-10 | Apple JWKS TTL-cache trigger (latency/rate at scale); cache design (in-memory, never a table) | CODE | post-Phase-3a scale |
| OPEN-11 | Apple `/auth/apple/events` receiver phase + signed-JWT fixtures (§10 scope cut; `signal_action` rows ready) | SEC | revocation handling |
| OPEN-12 | ~~Meta Graph API version pin (v26.0)~~ RECORDED in §11.1 + code; sunset watch (~2yr terms) is standing ops hygiene, no further spec action | CODE | §11.1, ongoing |
| OPEN-13 | Meta deauthorize + data-deletion phase; deletion-receipt `{url, confirmation_code}` tracker table design | SEC | §11.8 gap |
| OPEN-14 | Profile display for `apple`/`meta` links (extends OPEN-8; raw email never stored) | Author | profile UI |
| OPEN-15 | ~~CN→user_id mapping for offboarding~~ — obsolete: client certs were replaced by the IP allowlist; the Staff page revokes by `user_id` (§5.6) | Author | §5.6 automation |
| OPEN-16 | ~~Schedule conditional-ui build~~ — built (P431b, §2.5) | Author | autofill scope |
| OPEN-17 | Destructive-action list for §5.5 step-up (config writes? cert revoke? fact accept/reject?) | SEC | §5.5 enforcement |
| OPEN-18 | ~~Playbook §1 + header touch-up~~ RESOLVED: header + §1 point at §5; live proof stays OPEN-1 | CODE | doc consistency |
| OPEN-19 | ~~Prod CA custody: who holds `ca-key.pem`, where (HSM / offline backup)?~~ RESOLVED: offline encrypted copy held by the designated custodian (name TBD); live key stays root-only on the server; HSM deferred (author, 2026-10-29) | Author | prod enrolment |
| OPEN-20 | Delivery channel for invite URLs + client key pairs (same channel or split)? | Author | enrol ceremony |
| OPEN-21 | Server-side client-key copies: keep 0600 or delete-after-confirmed-delivery? | Author | runbook wording |
| OPEN-22 | Who may mint invites (any admin via xtask, or narrower set)? | Author | invite authz |
| OPEN-23 | ~~Admin-role enrolment quorum: single admin or two-person/break-glass?~~ PARTLY RESOLVED: break-glass is the sealed envelope + ordered Drill 4 (decision 59); quorum size for `--role admin` stays open (author outcome keyed "OPEN-22", re-keyed 2026-10-29) | Author | `--role admin` path |
| OPEN-24 | ~~CRL/OCSP for staff client certs~~ — obsolete: no client certs (§5) | SEC | cert lifecycle |
| OPEN-25 | ~~Mail provider choice~~ RESOLVED: Gmail SMTP for andrii.ai initially; host/port/creds/From stay configurable for a future own-sender swap (author, 2026-10-28, §12.8) | Author | Phase 5 send path |
| OPEN-26 | Bounce/complaint handling phase + webhook receiver design (out of v1 scope per §12.8) | SEC | post-Phase-5 deliverability |
| OPEN-27 | Link-prefetch confirmation step: build only if staging shows prefetcher burns (§12.4); design owned then | SEC | fallback UX, conditioned |
| OPEN-28 | Contact-email change / email-row removal: `method='email'` rows count as live methods but are not unlinkable in v1 — define the change flow | Author | profile UI |

## 9. Google OAuth (Phase 2)

Scope: customer login + linking on the **apex branch only** (`https://andrii.ai`). Plain OAuth2 authorization-code flow with **server-side exchange**; no Google Identity Services JS (deviation from playbook §3 step 3, decided here: tokens never touch the browser, no FedCM/SDK dependency, verification stays one server round-trip). The admin branch gets **no** Google routes in Phase 2 (OPEN-4). Paths stay flat with passkey/Apple/Meta (`/auth/google/…`, matching the playbook §3 sketch — no `/oauth/` segment). Requested scopes: `openid email profile` only.

### 9.1 Endpoints (public router, alongside §2)

- `POST /auth/google/start` → `200 {"auth_url": "https://accounts.google.com/o/oauth2/v2/auth?…&state=<handle>"}`. DTO `{"mode"?: "login"|"link"}` with `deny_unknown_fields` (§2 header rule); absent mode means `login`. Modes mirror §2.1: (a) logged-out register-or-login; (b) logged-in link. Mode (b) requires a **customer** session cookie **and** freshness — `step_up_at` within 600s **or** session minted within 600s — else `403 {"error": "step-up required: log in again first"}` (same rule as `register_start` mode b; non-customer sessions get `403 {"error": "google linking is for customer accounts"}`). Guards: Origin + double-submit CSRF (start mints the `hb_csrf` cookie like every §2 start); unknown `mode` → 400. Unconfigured keys (§9.5) → `503 {"error": "google login unavailable"}`.
- `GET /auth/google/callback?code=…&state=…` finishes the flow and **302s**, never JSON (top-level browser navigation; Google appends its own params, so no `deny_unknown_fields` — only `code`/`state`/`error` are read). Success → `302 /apostille/login` (login; the page shows the logged-in state) or `302 /apostille/security` (link; the fresh page load lists the new method — no success param). Failures → `302 /apostille/login?error=<code>` with codes `oauth_denied` (user cancelled at Google), `oauth_expired` (410 analog: missing/consumed/expired state), `oauth_failed` (binding mismatch, exchange/verify failure, role refusal — one generic code, no PII, no provider internals), `oauth_unavailable` (503 analog), `oauth_rate_limited` (429 analog), `link_required` (rule-5 hint: "an account with this email exists — log in with your passkey first, then link Google from Security"), `already_linked` (sub owned by another account in link mode), `account_locked`. The login page renders the `?error=` copy **outside** the logged-in gate (link-mode failures land there too); the SPA renders each code as copy, never a dead end.

### 9.2 State: reuse `webauthn_challenge` with `kind='oauth'`

No new table. The DDL puts no `CHECK` on `kind`, and `take_challenge`/`sweep_challenges`/`proof_fresh` are kind-generic, so the OAuth state rides the existing machinery: `id` = 256-bit hex handle (the `state` param), `user_id` = link-mode session user or NULL, `state_json` = `{"mode": "login"|"link", "binding": "<256-bit hex>"}`, 5-min expiry, single-use consume-on-first-callback (consume **before** the exchange call, mirroring §2.2/§2.4), sweep on every start. CSRF binding for the GET callback (no Origin enforcement on navigations): start also sets an `hb_oauth=<binding>` cookie (`HttpOnly; SameSite=Lax; Path=/; Max-Age=300; Secure` in prod); callback requires cookie binding == row binding, then clears the cookie on **all** exits. Replays die on single-use; cross-site forgery dies on the binding.

### 9.3 Exchange + verification (no new crypto deps)

On `spawn_blocking` (ureq is blocking; precedent: voice/labels call sites), with the workspace-standard ureq 3 `rustls-no-provider + rustls-webpki-roots` feature set plus the `aws-lc-rs` rustls feature (ingest precedent) — already in `Cargo.lock`, so the web crate gains dependency entries but **zero new crypto code**:

1. `POST https://oauth2.googleapis.com/token` with `{code, client_id, client_secret, redirect_uri, grant_type=authorization_code}` (exact `redirect_uri` match, as registered). Take the `id_token`; the access token is unused (`access_type=online`, no refresh — our sessions are the credential; Google tokens are never stored — no table holds them, by design).
2. `GET https://oauth2.googleapis.com/tokeninfo?id_token=…`, which returns `aud, sub, iss, exp, email, email_verified` (Google backend-auth docs; `email_verified` is the string `"true"`).
3. Verify: `aud == google.client_id`; `iss` is `https://accounts.google.com` or `accounts.google.com`; `exp` (unix) > now; `sub` non-empty; `email` well-formed. `email_verified == "true"` sets `verified_email=1`, otherwise `0` — and only `1` rows ever match (§6 rule 2, enforced by the existing `usable_email_owner` query). Any failure → `oauth_failed`. No JWT/JWK stack, no JWKS fetch — tokeninfo does the signature work server-side. Scale-out trigger for a JWKS migration is OPEN-5.

### 9.4 Account resolution (playbook §6 rules 1–5)

`LoginMethod::Google` (`'google'`) + `identity_link(user_id, 'google', sub, email_hash, verified_email, 0, NULL)` — the table expresses this today (`method` is free text).

- Known live `(google, sub)` → login: locked account → `account_locked`; non-customer role → `oauth_failed` + audit (staff log in with a passkey on the admin host per §5, never Google); else `create_session` + rotate CSRF + `clear_fallback` + audit `oauth_login`; stamp the consumed state row's `user_id` (one `UPDATE`, so `proof_fresh` doubles as this login's unlink re-auth proof).
- Unknown sub + link mode (row `user_id` + matching valid customer session): sub owned by another account → `already_linked`, never a merge; sub already linked to this account → idempotent success; else insert link in-txn (`ensure_account` is idempotent), audit `method_linked` with `hash_str(sub)` (rule 7, hashes only), `302 /apostille/security`.
- Unknown sub + logged-out → `decide_unknown_sub(usable_email_owner(email).is_some())`: match → `link_required` (rule 5, no auto-merge, no account disclosure in the URL); no match → `ensure_account` + link + session + audit `oauth_login` (reason `new_account`), `302 /apostille/login`.
- Login by sub is allowed regardless of `email_verified`; unverified rows just never match (rules 2–3 fall out of the existing query).

Unlink reuses `POST /auth/profile/unlink` with `{"method": "google", "id": "<sub>", "reauth_proof": "<consumed challenge id | "session">"}`: the method gate accepts `passkey` and `google`; google disables the link row only (no credential row exists). Re-auth proof is a fresh consumed challenge (passkey ceremony or stamped oauth row via `proof_fresh`) **or**, for Google-only users who cannot run a passkey ceremony, the literal `"session"` when the current session was minted within 600s (accepted rule-6 tradeoff: recent auth + remaining-method lock; last-method removal stays refused). Keep `unlink_permitted` (`409 {"error": "last method cannot be removed"}`) and the rule-6 tail (`locked_at` + `account_locked`) verbatim; audit `method_unlinked` with the method as reason. `GET /auth/profile/methods` already lists `google` rows (label `""` — acceptable; OPEN-8 covers richer display).

Rule-6 note: a Google-authenticated session counts as *fresh auth* for the mode-(b) session-minted window but does **not** set `step_up_at` (no UV signal) — same asymmetry as passkey login without UV.

### 9.5 Config (auth.* overlay pattern)

Two registry keys: `google.client_id` (`text`, `overlay_optional_text` → `Option<String>`) and `google.client_secret` (`secret`, `overlay_secret` → `Option<SecretText>`, redacting, never logged). `RuntimeConfig` defaults both to `None`; **boot always succeeds** without them (missing/empty/wrong-typed → silent `None`, mirroring the Telnyx-secret leniency); endpoints fail closed per §9.1. `AppState` carries both (secret as `SecretText`, same as `voice_api_key`). Registry `ALL` grows 34→36. Admin display: client_id readable text, secret presence-only. Redirect URI is derived (`https://{auth.origin}/auth/google/callback`, trailing slash trimmed), not configured.

### 9.6 Errors, rate limits, PII

- Start errors mirror §2.x JSON shapes: 400 bad mode, 401 link-without-session, 403 CSRF/Origin/step-up/non-customer, 429 `{"error": "rate limited"}`, 503 unconfigured, 500 store unavailable. Never log `auth_url` (state is bearer-ish), codes, tokens, emails, or subs — hashes only.
- Callback errors are the §9.1 `?error=` codes (302, no PII in URL or logs). All 302 targets are fixed same-origin paths, never reflected input (no open redirect).
- Rate limits mirror §6: start `30/min/IP` (`gs:{ip}`); callback `60/min/IP` (`gc:{ip}`) pre-identify + `10/min/account` (`gc:{user_id}`) once resolved; unlink's existing `10/min/account` covers Google unlinks. Atomic check-and-decrement buckets, same struct.
- Audit actions: `method_linked` / `method_unlinked` / `oauth_login` + `account_locked` tail, actor + sub/email hashes only; anonymous failures audit `google_denied` / `google_expired` / `google_failed` (reason carries the sub-cause: exchange, tokeninfo, binding — never bodies, which can echo secrets).

### 9.7 Staging acceptance (extends §7)

Google login green (new account → session + cookie rotation); known-sub login green; verified-email duplicate → `link_required`, accounts stay separate until explicit link flow (pre-account-takeover fixture red→green); link-mode without fresh step-up refused; state replay → `oauth_expired`; binding-cookie stripped → `oauth_failed`; user-cancel at Google → `oauth_denied`; keys unset → start 503 + callback `oauth_unavailable`, boot clean; last-method Google unlink refused; non-customer Google login refused; `cargo xtask ci` green.

### 9.8 Table-expressiveness note

Everything above fits the current schema — no DDL: `kind`/`method` are unconstrained text, `usable_email_owner`/`live_method_count`/`links_for_user` are method-generic, `proof_fresh` covers oauth rows once `user_id` is stamped. Deliberately unexpressed: Google access/refresh tokens (never stored — `access_type=online`, our sessions are the credential) and raw emails (hashes only, per §6 logging rules).

### 9.9 RISC receiver (OPEN-7, decided 2026-10-29)

`POST /auth/google/risc` (public router, flat path): Google pushes
Security Event Tokens as the raw POST body
(`application/secevent+jwt`). No session, no CSRF/Origin — the SET
signature is the auth (Meta `delete_request` precedent, §11.9).
Boot stays green when Google is unconfigured; the handler 503s.

Verification (`auth::risc::verify_set`, mirrors §10.3): `kid` →
GET `risc_certs_url` (Google default; test-override only) → JWK →
RS256-only decode → manual claims with injected `now` (`iss`
slash or bare; `aud` string or single-entry array; `|now-iat| ≤
300s`; `exp` when present). `events` takes the first key (unknown
URIs verify fine — forward-compat); `subject` `iss_sub` is
actionable, `email` and unknown formats are audit-only (acting on
an unverified email match is a takeover vector).

Event→action reuses `signal_action` verbatim: sessions-revoked
kills sessions + clears step-up; credential-change clears step-up
only; account-disabled disables the link + kills sessions + the
rule-6 tail (last link → lock); account-enabled audits only;
verification audits the hashed `state` echo. Unknown event /
unknown subject → `202 {"status":"accepted"}` + audit (Google
backs off on repeated errors — never non-2xx a verified SET).
Malformed → `400`; bad signature/claims → `403`; certs
unreachable → `503`. Rate limit `gr:{ip}` 30/min. No DDL: every
action is idempotent, so no replay table — a replayed SET
re-applies and still 202s.

Re-auth revives: a fresh verified Google login whose sub has only
a disabled link revives it (`google_link_revived` audit) instead
of forking a second account; link-mode re-add of one's own
disabled sub revives, of another user's 302s `already_linked`.

## Shared chassis decisions (Apple + Meta)

Both providers reuse the Phase-2 Google chassis (§9, decision 32): flat paths, `POST start` returning `auth_url`, `GET callback` doing exchange+verify+resolve on `spawn_blocking` through the stubbable `HttpTransport`, single-use state with a split binding cookie, and 302s with a fixed code vocabulary. Five cross-provider decisions are fixed here so §§10–11 stay parallel:

**D1 — Flat paths.** `POST /auth/apple/start`, `GET /auth/apple/callback`, `POST /auth/meta/start`, `GET /auth/meta/callback` on the public router only (apex-only, like §9; OPEN-4 still governs the admin branch). The playbook already names both callback URLs (§2 step 2, §4 step 1).

**D2 — Separate state kinds (`kind='apple'`, `kind='meta'`).** `take_challenge` enforces `expected_kind` before parsing anything, so a handle minted for one provider can never be consumed by another provider's callback even if an attacker swaps callback URLs — the kind column is the confused-deputy boundary. A shared kind with a provider field inside `state_json` would move that check to after consume+parse, strictly weaker. Cost is two consts plus thin wrappers; the `OauthState {mode, binding}` JSON shape, 5-min TTL, consume-before-exchange order, and sweep are reused verbatim.

**D3 — Per-provider binding cookies (`hb_apple`, `hb_meta`).** A shared cookie breaks concurrent flows: start Google, then start Apple before finishing, and the second `Set-Cookie` clobbers the first binding. Per-provider names let flows coexist; each callback clears only its own cookie on all exits. Attributes identical to §9.2 (`HttpOnly; SameSite=Lax; Path=/; Max-Age=300; Secure` in prod).

**D4 — One shared 302 code vocabulary, no provider-prefixed codes.** The §9.1 codes are reused verbatim for all three providers: the code in the URL buys the user nothing provider-specific — the page they land on already knows which button they pressed — and every provider-specific failure collapses to `oauth_failed`/`oauth_unavailable` by the §9.1 no-PII/no-internals rule anyway. The SPA keeps one 8-entry provider-agnostic map. Providers ARE distinguished server-side in audit actions (`apple_denied`, `meta_failed`, …).

**D5 — Crypto: `jsonwebtoken 11` on the workspace provider (verified ring-free).** Apple needs ES256 (client_secret signing + id_token verification). Hand-rolling JWS/JWK verification (~150 lines of kid/alg/curve handling) is exactly the code that breeds auth vulnerabilities, so this phase takes the audited `jsonwebtoken` crate with the `aws_lc_rs` backend: manifest verified to contain zero `ring` mentions, exactly-one-backend auto-selects aws-lc (no `install_default`, no clash), new lock crates are `jsonwebtoken` + `simple_asn1` only (non-crypto; every transitive dep already locked). Meta's `appsecret_proof` (HMAC-SHA256) uses `aws-lc-rs` `hmac` directly (already locked). No new crypto implementations anywhere — the §9.3 rule holds in spirit: the policy bans `ring`, not vetted crates on the approved provider.

## 10. Apple — Sign in with Apple (Phase 3a)

Scope: customer login + linking, apex branch only. Server-side authorization-code flow; no Apple JS (tokens never touch the browser). Requested scope: `name email`. The `/auth/apple/events` endpoint (playbook §2 step 4) is OUT of this slice — same treatment as RISC/OPEN-7; `signal_action` rows already exist (OPEN-11 tracks it).

### 10.1 Endpoints

- `POST /auth/apple/start` → `200 {"auth_url": "https://appleid.apple.com/auth/authorize?…"}`. DTO, guards, link-mode rule-4 freshness (customer session + `step_up_at`/session-minted within 600s), and error shapes identical to §9.1 with `s/google/apple/`. Authorize params: `response_type=code`, `client_id` = Services ID, `redirect_uri` (derived, exact match), `scope=name email`, `state`, **`response_mode=query`** (explicit: keeps the GET-callback chassis; OPEN-9 confirms live behavior at the first real login). The `user` JSON Apple sends on first authorization is ignored (we store no display names; link rows carry hashes only).
- `GET /auth/apple/callback?code=…&state=…` → 302s per D4. User-cancelled at Apple → `oauth_denied` (`error` param present → burn state best-effort, audit `apple_denied`). No `deny_unknown_fields` (provider appends params).
- No nonce: the `state`+binding pair covers CSRF and the id_token arrives via our own server-side token call, never through the browser. The `real_user` id_token field is an informational anti-fraud signal, not authentication — ignored for auth decisions (documented here so nobody gates on it later).

### 10.2 State: `webauthn_challenge` with `kind='apple'`

D2/D3: `id` = state handle, `user_id` = link-mode user or NULL, `state_json` = `{"mode","binding"}` (same struct), 5-min TTL, consume-before-exchange, sweep on every start, `hb_apple` binding cookie required at callback, cleared on all exits.

### 10.3 Exchange + verification

On `spawn_blocking` through the existing `HttpTransport` + `UreqTransport` (plus a `get_query` transport method for Meta's GET calls — no other transport changes):

1. Mint `client_secret` (ES256 JWT, per exchange — no cache): header `{"alg":"ES256","kid": key_id}`, claims `iss=team_id`, `sub=client_id` (Services ID), `aud=https://appleid.apple.com`, `iat=now`, `exp=now+300`, via `jsonwebtoken::encode` with `EncodingKey::from_ec_pem` (Apple `.p8` files are PKCS#8 — matches verbatim). Per-exchange minting is deliberate: zero cache invalidation, zero clock-skew risk, one ECDSA sign per login is negligible.
2. `POST https://appleid.apple.com/auth/token` with `{grant_type=authorization_code, code, client_id, client_secret, redirect_uri}` (same POST shape as Google — the shared exchange fn). Take `id_token`; refresh tokens are neither requested nor stored (our sessions are the credential — same rule as §9.3).
3. Verify `id_token` via Apple JWKS (no tokeninfo exists): `GET https://appleid.apple.com/auth/keys`, select JWK by the JWT header `kid` (reject unknown `kid` → `oauth_failed`, never retry-with-refresh in v1), build `DecodingKey::from_jwk`, decode with an ES256-only `Validation` (the crate enforces the algorithm — no alg-confusion class), then check claims manually with injected `now` (mirroring `tokeninfo_verify` testability): `iss == https://appleid.apple.com`, `aud == apple.client_id`, `exp > now`, `sub` non-empty. Round-trip count equals Google's (token + keys vs token + tokeninfo). No JWKS cache in v1 (correctness first; OPEN-10 tracks a TTL cache if latency demands it).
4. Email claims: `email` (may be a `@privaterelay.appleid.com` relay, or absent for returning users), `email_verified` (accept boolean `true` OR string `"true"`, fail closed otherwise — lenient parse, exact wire type confirmed at staging per OPEN-9), `is_private_email` boolean → `is_relay`.

Any failure → `oauth_failed` with an audit reason (`exchange`, `keys`, `sig`, `claims:aud`, … — tags only, never bodies/URLs/tokens).

### 10.4 Account resolution (rules 1–5)

`LoginMethod::Apple` (`'apple'`) + `identity_link(user_id, 'apple', sub, email_hash, verified_email, is_relay, NULL)`:

- Known live `(apple, sub)` → login: locked → `account_locked`; non-customer → `oauth_failed` + audit; else session + CSRF rotation + `clear_fallback` + audit `oauth_login`; stamp the consumed `kind='apple'` row's `user_id` (`stamp_state_user` is kind-generic SQL — works verbatim) so `proof_fresh` doubles as this login's unlink proof.
- Unknown sub + link mode: owned-by-other → `already_linked`; already-own → idempotent success + `302 /apostille/security`; else insert link in-txn (with the row's `is_relay` flag), audit `method_linked` with `hash_str(sub)`.
- Unknown sub + logged-out → `decide_unknown_sub(usable)`: `usable = email_usable_for_matching(Identity{Apple, verified_email, is_relay}) && usable_email_owner(email).is_some()`. Relay rows: Apple verifies relay addresses, so store `verified_email=1, is_relay=1` — and both gates (login.rs method+relay exclusion AND the `usable_email_owner` `is_relay=0` clause) exclude them, so relay logins always mint fresh accounts unless the sub is known; joining happens only via explicit link flow (rule 3 falls out of existing code, zero new branches). Real-address rows: `is_relay=0`, verified flag per claim; verified duplicates → `link_required` (rule 5, no merge, no disclosure).
- Login by sub works regardless of `email_verified`; unverified rows just never match (rules 2–3).

Rule-6 note (mirrors §9.4): an Apple-authenticated session counts as fresh auth for the link-mode window but does **not** set `step_up_at` (no UV signal).

### 10.5 Config

Four registry keys: `apple.client_id` (text — Services ID, e.g. `ai.andrii.signin`), `apple.team_id` (text — 10-char Team ID), `apple.key_id` (text — Key ID), `apple.private_key` (**secret** — full `.p8` PEM text including `-----BEGIN/END PRIVATE KEY-----` lines and newlines). `Secret` accepts any non-empty bytes ≤ 8 KiB — a ~250-byte multiline PEM fits; `xtask config-set --value-from-stdin` carries multiline values (paste with real newlines, not `\n`-escaped). `RuntimeConfig` gains 4 `Option` fields; `AppState` gains `apple_cfg: Option<AppleCfg>` (id, team, kid, key, derived `redirect_uri`, overridable base URLs for the port-0 stub harness). **Boot always succeeds** without them; any missing/empty half → endpoints fail closed (start `503 {"error": "apple login unavailable"}`, callback `oauth_unavailable`). Registry `ALL` grows by 4 here (+2 Meta = 36→42; count test updated once). Redirect URI derived (`https://{auth.origin}/auth/apple/callback`), never configured.

### 10.6 Errors, rate limits, PII

- Start: same JSON shapes as §9.6 with `s/google/apple/`. Never log `auth_url`, codes, tokens, `client_secret` JWTs, emails, or subs — hashes only. The private key lives only in the sealed row and `SecretText` (redacting `Debug`); the per-exchange JWT is built inside `spawn_blocking` and dropped.
- Callback: D4 codes, fixed same-origin targets, no open redirect.
- Rate keys mirror §9.6: start `30/min/IP` (`as:{ip}`); callback `60/min/IP` (`ac:{ip}`) + `10/min/account` (`ac:{user_id}`); unlink's existing `un:{user_id}` covers Apple unlinks. Same bucket struct.
- Audit: `method_linked`/`method_unlinked`/`oauth_login` + `account_locked` tail; anonymous failures `apple_denied`/`apple_expired`/`apple_failed` (reason = static sub-cause tag).

### 10.7 Staging acceptance (extends §7)

Apple login green (new account → session + cookie rotation); known-sub login green; verified real-email duplicate → `link_required`, accounts stay separate (pre-account-takeover fixture red→green); **relay fixture**: relay login mints a fresh account and never matches the same user's real-address account by string; relay account joins only via explicit link flow; link-mode without fresh step-up refused; state replay → `oauth_expired`; binding stripped → `oauth_failed`; cancel at Apple → `oauth_denied`; unknown-JWKS-`kid` → `oauth_failed`; keys unset → start 503 + callback `oauth_unavailable`, boot clean; last-method Apple unlink refused; non-customer Apple login refused; `cargo xtask ci` green. Portal note: Hide-My-Email tested with a real Apple ID (synthetic subs/JWKS in unit tests via the stub harness, which serves token + keys bodies).

### 10.8 Table-expressiveness note

Fits the current schema — no DDL: `kind='apple'` is free text, `is_relay` already exists, `usable_email_owner`/`proof_fresh`/`disable_link`/`live_method_count` are relay-aware and method-generic. Deliberately unexpressed: `client_secret` JWTs (minted per exchange, never stored), Apple JWKS (fetched per verify, no cache table in v1 — a future cache should be in-memory with TTL like the rate buckets, never a table), refresh tokens (never requested), raw emails (hashes only).

## 11. Meta — Login with Meta (Phase 3b)

Scope: customer login + linking, apex branch only. Server-side code flow; no Meta JS SDK (tokens never touch the browser, no SDK dependency). "Login with Meta" on web is Facebook Login. **No email scope is requested and no email is ever fetched**: `debug_token` returns no email, a further `/me` call would buy nothing since Meta email never matches (login.rs exclusion), so the flow is sub-only by design — one less permission, no review surface, one less round trip. The deauthorize + data-deletion callbacks (playbook §4 step 4) are OUT of this slice (OPEN-13); `signal_action` rows already exist.

### 11.1 Endpoints

- `POST /auth/meta/start` → `200 {"auth_url": "https://www.facebook.com/v26.0/dialog/oauth?…"}`. Same DTO/guards/link-mode rule-4 shape as §9.1 with `s/google/meta/`. Auth params: `client_id` = App ID, `redirect_uri` (derived, exact match — must be listed in Client OAuth Settings), `state`; **no `scope` param** (default `public_profile`; see above). Graph API version is pinned as URL consts mirroring `GOOGLE_AUTH_URL` — currently `v26.0`, the highest valid version probed live on 2026-10-28 (recorded in code + the operator doc; versions sunset on ~2-year terms — OPEN-12 watches expiry).
- `GET /auth/meta/callback?code=…&state=…` → 302s per D4. User-cancelled (`error` param) → `oauth_denied` + `meta_denied` audit. No `deny_unknown_fields`.

### 11.2 State: `webauthn_challenge` with `kind='meta'`

D2/D3: same `{"mode","binding"}` shape, 5-min TTL, consume-before-exchange, sweep on start, `hb_meta` binding cookie, cleared on all exits.

### 11.3 Exchange + verification (no id_token — two HTTPS GETs)

On `spawn_blocking` through `HttpTransport` + `UreqTransport` (both calls documented GETs; secrets travel in query strings here, so URLs are never logged — same rule as the tokeninfo GET):

1. Exchange: `GET https://graph.facebook.com/v26.0/oauth/access_token?client_id=&redirect_uri=&client_secret=&code=` (Meta's documented code-exchange call; pairs encoded by the transport). Take `access_token` (a user token, opaque); it is verified and dropped, never stored.
2. Validate via `debug_token`: `GET https://graph.facebook.com/v26.0/debug_token?input_token={user-token}&access_token={app-id}|{app-secret}&appsecret_proof={proof}` (app access token = `client_id|client_secret` pipe-joined, built in memory, never logged; `proof = hex(HMAC-SHA256(app_secret, input_token))`, always sent — required when the app's "Require App Secret Proof" is on, ignored otherwise). Require `data.is_valid == true`, `data.app_id == meta.client_id`, `data.user_id` non-empty (the stable app-scoped id = our `provider_sub`), `data.expires_at > now`. Any mismatch → `oauth_failed` (reason `debug`).

Failures → `oauth_failed` (reasons `exchange`/`debug`); transport errors → `oauth_failed` (reason `transport`); statuses kept for audit, bodies never (they can echo secrets).

### 11.4 Account resolution (rules 1–5; Meta is sub-only)

`LoginMethod::Meta` (`'meta'`) + `identity_link(user_id, 'meta', app_scoped_id, NULL, 0, 0, NULL)` — email is never fetched so the hash is always NULL and `verified_email` is always 0 (uniform with the never-match rule, enforced twice by existing code: `email_usable_for_matching` excludes `LoginMethod::Meta`, and `usable_email_owner` has a `method != 'meta'` clause).

- Known live `(meta, sub)` → login: locked → `account_locked`; non-customer → `oauth_failed` + audit; else session + rotation + `clear_fallback` + audit `oauth_login`; stamp the `kind='meta'` row for `proof_fresh`.
- Unknown sub + link mode: owned-by-other → `already_linked`; already-own → idempotent success; else insert + `method_linked` with `hash_str(sub)`, `302 /apostille/security`.
- Unknown sub + logged-out → **always `CreateFreshAccount`** (routed through `decide_unknown_sub` for uniformity, but the Meta gate makes `usable` false by construction) — `link_required` NEVER fires for Meta. The explicit link flow is the only way a Meta identity joins an existing account.
- Rule-6 note: Meta session counts as fresh auth for the link window, never sets `step_up_at`.

**App-review implications (decided here):** no permissions are requested beyond the default `public_profile`, so there is no review surface at all. While the Meta app is in dev mode only registered test users can complete login (staging acceptance adds test users, mirroring the Google test-user story). Secret rotation follows the revoke-after-verify order — except Meta secret reset is IMMEDIATE with no overlapping-secret grace, so reset-then-verify in one maintenance window (brief Meta-login outage is inherent; other methods unaffected).

### 11.5 Config

Two registry keys: `meta.client_id` (text — App ID) and `meta.client_secret` (**secret**). `RuntimeConfig` +2 `Option` fields, `AppState` + `meta_cfg: Option<MetaCfg>` (same shape as `GoogleCfg`: id + secret + derived `redirect_uri` + overridable base URLs for the stub harness). Boot always succeeds; missing halves → start `503 {"error": "meta login unavailable"}`, callback `oauth_unavailable`. Registry `ALL` 36→42 together with Apple (§10.5). Redirect URI derived (`…/auth/meta/callback`).

### 11.6 Errors, rate limits, PII

- Start: §9.6 shapes with `s/google/meta/`. Never log `auth_url`, codes, user/app tokens, `appsecret_proof` inputs, or subs — hashes only.
- Callback: D4 codes, fixed targets.
- Rate keys: start `ms:{ip}` 30/min; callback `mc:{ip}` 60/min + `mc:{user_id}` 10/min; unlink via existing `un:{user_id}`.
- Audit: `method_linked`/`method_unlinked`/`oauth_login` + tail; anonymous `meta_denied`/`meta_expired`/`meta_failed` with static sub-cause reasons.

### 11.7 Staging acceptance (extends §7)

Meta login green (sub-only path: account + session, NULL email hash, no crash); known-sub login green; same-address Apple/Google account stays separate and NO `link_required` fires (always-fresh-account fixture red→green); explicit link flow joins correctly; link-mode without step-up refused; replay → `oauth_expired`; binding stripped → `oauth_failed`; cancel → `oauth_denied`; `debug_token` app_id mismatch → `oauth_failed`; keys unset → 503 + `oauth_unavailable`, boot clean; last-method Meta unlink refused; non-customer refused; `cargo xtask ci` green. Portal: dev-mode test users registered.

### 11.8 Table-expressiveness note

Fits today — no DDL: `kind='meta'`/`method='meta'` free text, NULL email hash + `verified_email=0` + `usable_email_owner`'s `method != 'meta'` clause give the never-match rule with zero new branches, `proof_fresh`/`disable_link`/counts generic. Deliberately unexpressed: user + app access tokens (verified and dropped), `appsecret_proof` values (per-call, never stored). **Not expressible yet (later slice):** Meta data-deletion receipts — the `{url, confirmation_code}` status tracker playbook §4 requires has no table; OPEN-13 designs it with the deauthorize/deletion endpoints.

### 11.9 Data-deletion callback (closes §11.8's deletion half; deauthorize stays in OPEN-13)

Dashboard "Data Deletion Request Callback" URL:
`https://andrii.ai/auth/meta/delete-request` (flat, like the
login callbacks). Meta POSTs form-encoded `signed_request`
(`sig_b64url.payload_b64url`, HMAC-SHA256 over the payload
segment under the app secret); `algorithm` must read exactly
`HMAC-SHA256`. One failure shape (400) for every malformed or
badly-signed request — no oracle. Rate budget `md:{ip}`
10/60; unconfigured Meta 503s; boot never refuses.

On verify, the app-scoped `user_id` resolves via
`identity_link(method='meta', provider_sub)`. Known users are
purged synchronously and the response is exactly Meta's shape:
`{url, confirmation_code}` with a 128-bit hex code and
`url = {auth.origin}/auth/meta/delete-status?code=…`. Unknown
subs get the same success shape with status `unknown` (Meta:
disregard — nothing to delete).

Purge (synchronous, in-request): identity rows first
(sessions, identity links, passkey credentials, account —
`auth::passkey::erase_identity`), then handbook data
(submissions incl. `values_json`, shipments, labels,
label defaults, and the captures + blob files those rows
point at — `handbook_store::erase_customer_data`, id-0
sentinel never touched). Kept by design, stated on the
receipt and in the privacy policy: append-only security
audit rows (opaque actor ids, unlinked once the account
dies) and payment/invoicing records (none held today;
retained per tax schedule when billing ships).

Receipts live in `meta_deletion(code, status, created_at,
completed_at)` — no user id or Meta sub is kept. `GET
/auth/meta/delete-status?code=` renders the status as
server-side HTML (no JS, crawler-safe); unknown codes 404
with the same explanatory shape and never echo input.

Acceptance: valid signed request → 200 shape + rows gone +
receipt `completed`; unknown sub → 200/`unknown`;
tampered payload or wrong secret → 400; unconfigured → 503;
status page renders prose for completed/unknown/404;
`cargo xtask ci` green.

## Link/unlink generalization (all OAuth methods)

- `POST /auth/profile/unlink` method gate extends from `{passkey, google}` to `{passkey, apple, google, meta}`; OAuth methods all take the `disable_link` path (no credential row). `{"method": "apple"|"meta", "id": "<sub>", "reauth_proof": …}` shapes mirror Google's.
- The fresh-session `"session"` re-auth proof generalizes from Google-only to **all OAuth methods** (the `body.method == "google"` condition becomes `body.method != "passkey"`): same accepted rule-6 tradeoff — an OAuth-only user of any provider cannot run a passkey ceremony; the remaining-method lock and 600s session-freshness window are unchanged. Stamped `kind='apple'`/`'meta'` rows already satisfy `proof_fresh` (kind-generic).
- `GET /auth/profile/methods` lists `apple`/`meta` rows generically today (label `""`); richer display folds into OPEN-8's successor OPEN-14. The link-mode session-ownership + never-merge + idempotent-success logic is reused per provider (parameterize over the find/insert fns).
- Security page: per-method link buttons + unlink calls reuse the Google wiring with the method string swapped; unlink for OAuth-only users passes `"reauth_proof": "session"` (fresh session required).

## New OPEN rows (appended to §8)

OPEN-9..14 (Apple + Meta) live in the §8 table — the canonical
list. New rows always land there, not here.

## 12. Magic link — email fallback (Phase 5)

Scope: customer login + implicit email verification on the **apex
branch only**. Fallback of last resort per playbook §5: offered
only when passkey/Apple/Google/Meta all fail. No link mode, no
account creation, no admin-branch routes (staff stay
passkey on the admin host per §5). Flow: `POST /auth/magic/start` (email in,
always-200) → email with link → SPA reads the token from the
URL **fragment** → `POST /auth/magic/consume` (JSON, 200 + session
cookies). Reuses verbatim: `mint_magic_token`/`token_hash`/
`MAGIC_LINK_TTL_SECS` (`login.rs`), `RateBuckets`, `audit_auth`,
`create_session` + `PUB_COOKIE` + CSRF rotation.

### 12.1 Endpoints (public router, alongside §2)

- `POST /api/v1/auth/magic/start`, DTO (`deny_unknown_fields`):
  `{"email": "…"}`. No session required. Guards: Origin check;
  mints `hb_csrf` like every §2 start. Malformed email →
  `400 {"error": "email malformed"}` (mirrors `register_start`;
  shape errors reveal nothing).
- `POST /api/v1/auth/magic/consume`, DTO: `{"token": "<64-hex>"}`.
  Guards: **Origin check only — no CSRF echo** (deliberate:
  consuming from a different browser is the point, so no
  double-submit cookie can exist on first touch). Success →
  `200 {"user_id": …, "fallback": true}` + session cookies.
- **POST, not GET — decided (D35).** `request_id_and_log`
  (`web/main.rs`) logs the full `Uri` including query and asserts
  our URLs never carry tokens. A magic token is a standalone
  Bearer [REDACTED] full login grant; unlike the OAuth `code`
  (useless without the server-side `client_secret`), it must never
  appear in a query string. Hence the emailed link carries the
  token in the **fragment** — `https://{auth.origin}/
  apostille/login#t=<64hex>` (origin-derived, never
  configured) — which never leaves the browser, and the SPA
  POSTs it in the JSON body (bodies are never logged). No
  `?token=` form exists anywhere. No 302 vocabulary: magic is
  JSON-only, so no D4 codes are added or reused.
- **No binding cookie — deliberate D3 deviation.** Playbook §5:
  cross-browser consume is the point. Security is the 256-bit
  single-use token + 900s TTL + email-channel authentication.

### 12.2 State: NEW `magic_token` table

**A dedicated table, not `webauthn_challenge` `kind='magic'` reuse:**
(1) TTL differs and `store_challenge` hardcodes 300s — magic
needs 900s; parameterizing risks silent ceremony TTL changes.
(2) Lookup key points the opposite direction (hash of a
client-presented bearer, `staff_invite` pattern). (3) Payload is
relational (email binding, per-email invalidation) — queryable
columns, not opaque JSON. (4) Lifecycle (per-email invalidation,
send-failure delete) fits neither challenge sweep nor
`proof_fresh`, whose kind-genericity would turn stamped magic
rows into unlink proofs. (5) Precedent: `staff_invite` chose
exactly this shape.

```sql
CREATE TABLE IF NOT EXISTS magic_token(
  token_hash TEXT PRIMARY KEY,  -- SHA-256 hex of the 256-bit token; Bearer [REDACTED] lives here
  email_hash TEXT NOT NULL,     -- hash_str(lowercased email); owner resolved at consume
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,  -- created_at + 900 (MAGIC_LINK_TTL_SECS)
  consumed_at INTEGER           -- set on first consume attempt (take-and-burn)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_magic_email ON magic_token(email_hash);
CREATE INDEX IF NOT EXISTS idx_magic_exp ON magic_token(expires_at);
```

Lands in `migrate_auth_tables` (single-writer rule, §1 header).
No `user_id` column by design: the owner is resolved fresh at
consume (§12.4), so a lock/disable landing between request and
consume still fails closed. Store fns in `auth::magic` (mirrors
the `auth::oauth` flow+transport module): `store_magic_token`,
`take_magic_token` (atomic guarded `UPDATE … WHERE consumed_at
IS NULL AND expires_at > ?`; zero rows → invalid),
`sweep_magic_tokens`, `invalidate_email_tokens`,
`owners_by_email_hash`, `verify_email_rows`, `set_fallback`.
Sweep + per-email invalidation (re-request burns outstanding rows
for that `email_hash`, so exactly one live token per address) run
on every request.

### 12.3 Request flow (anti-enumeration core)

Known, unknown, and ambiguous addresses all return `200
{"ok": true}` with indistinguishable bodies. The path always does
hash + owner lookup; the send happens last and only when ≥1 live
owner exists. Unknown/ambiguous addresses store nothing and send
nothing. (No "no account here" email is ever sent — that would
turn the endpoint into a spam oracle and an inbox oracle.)
"Fallback-only" gating is a UX concern (when to *offer* the
option), not a server refusal — refusing known-but-passkeyless
users would itself enumerate.

- Mailer unconfigured → `503 {"error": "magic login
  unavailable"}` (mirrors §9.1's `google login unavailable`;
  config state reveals nothing about accounts). Boot always
  succeeds (lenient overlays + one stderr WARNING).
- Provider send failure → row deleted + `magic_send_failed` audit
  (static tag, never provider bodies), still `200` (a failure is
  indistinguishable from unknown-address silence by design).
- Rate keys: `mr:{ip}` 10/min + `mr:{email_hash-prefix-16}`
  3/hour (hash-prefix keying mirrors §5.7 `ef:`; never raw
  email; the hourly per-target cap bounds inbox harassment).
  Exceeded → `429 {"error": "rate limited"}`.
- No audit on request (mirrors OAuth/passkey starts — buckets are
  the abuse control, and per-spam rows would be a DB-growth
  vector).

### 12.4 Consume flow (consume-before-session)

Order (mirrors §2.2/§5.2): (1) take-and-burn the token —
miss/expired/consumed → `410 {"error": "link invalid or
expired"}` (single code for all three: no replay-vs-expired
oracle, and both advise "request a new one"); malformed shape
(non-64-hex) → `400 {"error": "link invalid"}` (shape errors
reveal nothing); (2) resolve owners (§12.5); (3) per-account rate
gate `mx:{user_id}` 10/min (`mx:{ip}` 60/min runs pre-identify,
mirroring `gc:`); (4) locked → `410`-shaped (same body — a
locked-account consume burns the token and the user re-requests
after recovery; fail-closed, same tradeoff as §5.2 invite
consume), non-customer → `410` + audit; (5) flip verification
(§12.5), set `fallback_until` (§12.6), `create_session` + CSRF
rotation + `PUB_COOKIE`; (6) `200 {"user_id", "fallback": true}`.

- **Prefetcher hazard (accepted in v1, watched):** Gmail link
  prefetchers may burn the single-use token before the user
  clicks. Failure mode is safe but annoying (user re-requests;
  per-email invalidation keeps re-request clean). A
  confirmation-step follow-up is OPEN-27, conditioned on staging
  evidence — not built blind.

### 12.5 Account resolution (playbook §6 rules 1–5)

`owners_by_email_hash`: `SELECT DISTINCT user_id FROM
identity_link WHERE email_hash = ? AND disabled_at IS NULL` —
any method, any verified flag, including relay (Meta hashes are
NULL and never match). Three outcomes:

- **0 owners → `410`** (the token was valid but the account
  vanished or was disabled mid-flight — no disclosure, no
  creation: magic never mints accounts; it is a login path for
  existing customers, and request-time unknown addresses never
  receive tokens).
- **1 owner → login.** Then `verify_email_rows`: `UPDATE
  identity_link SET verified_email = 1 WHERE user_id = ? AND
  email_hash = ? AND verified_email = 0 AND disabled_at IS NULL`.
  Playbook §6 rule 2 made mechanical — inbox control proven, so
  the unverified `method='email'` hint row left by passkey
  register (§2.2) flips to verified and joins the
  `usable_email_owner` surface for future OAuth `link_required`
  hints. Relay rows (already `verified=1`) and other-method rows
  are unaffected. **No new `identity_link` row is ever
  inserted** — no `magic_link` method rows exist in v1, so the
  unlink method gate, profile list, `live_method_count`, and the
  `"session"` proof (rule 6: a magic session counts as fresh
  auth but never sets `step_up_at` — no UV signal) behave
  correctly with zero new branches.
- **≥2 distinct owners → `410` + audit `magic_failed` reason
  `ambiguous`.** Rule 5 permits duplicate-email accounts across
  methods; a bare email cannot disambiguate them, so login fails
  closed (same accounts-stay-separate guarantee as
  `link_required`). Distinct-user counting (not row counting)
  keeps the common one-user-two-rows case (hint + Google row)
  green.
- Rules map: rule 1 untouched (no new links); rule 2 via the
  flip; rule 3 preserved (relay resolves to its own row's owner
  — inbox control of the relay mailbox — never by string-join);
  rule 4 N/A (no link mode); rule 5 via never-create +
  ambiguous-fail-closed; rules 6/7 via §12.8.

### 12.6 `fallback_until` semantics (OPEN-2 resolved)

**OPEN-2 decision (author-confirmed 2026-10-28):** magic consume
sets `fallback_until = now + FALLBACK_WINDOW_SECS`; while
`fallback_until > now`, the customer idle timeout tightens from
24h to `FALLBACK_IDLE_SECS`.

- `FALLBACK_WINDOW_SECS = 604_800` (7 days): covers a lost-device
  week including a weekend; bounded; each magic login re-arms it,
  so chronic-fallback users stay in the tighter regime until a
  strong login graduates them. Helper `set_fallback` mirrors
  `set_step_up`/`clear_fallback`.
- `FALLBACK_IDLE_SECS = 3_600` (1 hour): 24× tighter than
  customer idle, staff-adjacent strictness (staff 30m) without
  breaking active fallback use (`last_seen` touch keeps live
  sessions alive); it actually binds, since customer absolute
  (12h) already dominates the normal 24h idle — during fallback
  the 1h idle becomes the binding constraint. Absolute stays 12h.
- Enforcement (code, no DDL): `Session::valid` is role-only
  today, so `session_usable(conn, session, now)` (customer role
  only: fetch `fallback_until`, idle 3600 when future, else role
  default; other roles unchanged) replaces every `session.valid`
  call site — the HTTP extractor/`session_of`/`me` path and
  `ws::authorize` alike. Stale past `fallback_until` counts as
  inactive (compare `> now`, not non-NULL). Graduation unchanged:
  passkey/OAuth login calls `clear_fallback`; magic re-arms.
- Playbook §5 `auth_fallback=true` audit mark is expressed
  without schema change: `magic_login` audits carry reason
  `"fallback"` plus the `fallback_until` set — grep-able,
  joinable, zero DDL.

### 12.7 Relay-address interaction

Magic links **to** `@privaterelay.appleid.com` addresses are
attempted uniformly (same-200; relay rows resolve per §12.5), but
Apple delivers to relay addresses only from **registered
senders** — so the sending identity must be registered in the
Apple Developer portal or relay-address mail fails provider-side
(indistinguishable by design; staging validates with a real relay
address). The playbook §2 portal checklist gains that
registration step at implementation (OPEN-6/OPEN-18 precedent).
Relay toggles (`email-enabled/disabled`) stay owned by the future
Apple events slice (OPEN-11), not this one.

### 12.8 Mailer seam + Gmail decision (OPEN-25 resolved)

**OPEN-25 decision (author, 2026-10-28): send via Gmail SMTP for
andrii.ai initially; host/port/credentials/From stay configurable
rows so a future slice can swap in our own sender once IP
reputation is built — no code change for the swap.** The spec
imposes the seam contract; the swap is config-only by construction.

- `auth::magic::Mailer` trait mirrors the `HttpTransport`
  precedent: `send(to, subject, text_body) -> Result<_,
  MailError>` (secrets/links never appear in errors). Live
  `SmtpMailer` lives in `web` next to the handlers (the
  `oauth.rs` precedent) over **lettre 0.11 SMTP with the ring-free
  pin** (`default-features = false`, `smtp-transport` + `builder`
  + `rustls` + `rustls-no-provider` + `aws-lc-rs` +
  `webpki-roots` + `hostname`; the `rustls-tls` feature pulls
  `ring` and is forbidden — same one-provider rule as every
  `Cargo.toml` TLS comment). TLS policy is fixed: STARTTLS
  required (Gmail `smtp.gmail.com:587`); no plaintext SMTP, no
  mode key. Blocking send runs on `spawn_blocking` (no new async
  runtime deps — ureq precedent). Dev/test get a `FileMailer`
  (~one 0600 file per message under `{store}/mail/`); CI never
  sends real mail (stub asserts To-preservation +
  zero-sends-for-unknown).
- Config keys (registry conventions; count test updated once):
  `mailer.provider` text (`"smtp"` | `"file"`; unset/unknown →
  warn + unconfigured = 503, fail closed — no silent default:
  fresh stores 503 until dev sets `file` or prod sets `smtp`,
  the OAuth unset→503 contract), `mailer.host`,
  `mailer.port` (text, ports-are-text precedent), `mailer.username`
  text, `mailer.password` **secret** (Gmail App Password —
  never a mailbox password), `mailer.from` text (header/envelope
  From on andrii.ai; this exact address gets relay-registered
  per §12.7), `mailer.dir` text (default `"mail"`, store-root
  relative). Missing/partial SMTP set → request `503`, boot clean.
- No link/token logging: neither the full URL, the token, nor
  the raw address appears in app logs, audit rows, or provider
  metadata beyond the required `To:` delivery. Audit uses
  `email_hash`; rate keys use hash prefixes.
- Template: single link, 15-minute-expiry copy, no
  account-existence language ("if you requested this…"),
  plain-text + minimal HTML.
- Bounce/complaint handling is out of scope (no webhook receiver
  in v1) — OPEN-26.

### 12.9 Session / timeout / audit rules

- Session: `create_session` role customer, `customer_ref =
  user_id`, `PUB_COOKIE`, CSRF rotation — verbatim §2.4 tail.
  Fallback idle (§12.6) applies from the first request. No
  `step_up_at` (mirrors OAuth rule-6 notes).
- Audit actions: `magic_login` (actor `user_id`, reason
  `"fallback"`); `magic_expired` (reason `expired`) /
  `magic_failed` (actor `anonymous`, static-tag reasons: `replay`
  for consumed, `invalid` for unknown well-formed tokens, `shape`
  for malformed, `orphaned` for valid tokens whose owner
  vanished, `ambiguous`, `role`, `store`); `magic_send_failed`
  (actor `anonymous`, `transport` | `rejected`); `account_locked`
  tail reuses the existing shape. Hashes only, per §6.
- Rate summary: request `mr:{ip}` 10/min + `mr:{email_hash-16}`
  3/hour → JSON 429; consume `mx:{ip}` 60/min + `mx:{user_id}`
  10/min → JSON 429. All four prefixes are free (`mr`/`mx`
  collide with nothing in §§2/5/9–11).
- Link/unlink: no changes — method gate, `live_method_count`
  semantics, profile list, and the `"session"` proof all behave
  correctly with zero new branches (§12.5).

### 12.10 Staging acceptance (extends §7)

Request always-200 for known/unknown/ambiguous with identical
bodies; stub harness asserts zero sends for unknown; malformed →
400; mailer unconfigured → 503 + clean boot; consume green →
session + `fallback_until` set + 1h idle enforced (clock advanced
past idle → 401, before → 200) + 7d window re-armed on second
consume; passkey login clears `fallback_until`; replay +
post-900s consume → `410`; malformed token → `400`;
different-browser consume green (Origin-only, no CSRF echo);
locked → `410`-shaped; staff-account magic → `410`;
ambiguous-dup fixture → `410`, accounts stay separate;
unverified-hint flip → later OAuth duplicate fires `link_required`
(rule-2 chain red→green); relay request delivered via registered
sender (real relay address at staging); per-IP + per-email limits
trip correctly on both endpoints (hourly per-target cap asserted
with a short window override in tests, real window at staging);
audit/log grep shows hashes only, no token/link/address; Gmail
send proves end-to-end delivery at staging; `cargo xtask ci` green.

### 12.11 Table-expressiveness note

Fits today, no DDL: `identity_link` flip (plain `UPDATE`),
`account.fallback_until` (already there), `audit` (reason tag),
rate buckets (in-memory), `mint_magic_token`/`token_hash`/
`MAGIC_LINK_TTL_SECS`, `LoginMethod::MagicLink`. Needs DDL —
exactly one item: `magic_token` (§12.2) in `migrate_auth_tables`.
Needs code but no DDL: fallback-aware `session_usable` in all
`session.valid` call sites, the five store fns, the mailer trait +
SMTP/file impls. Deliberately unexpressed: raw emails (hashes
only, everywhere), Bearer [REDACTED] (hashes only),
provider tokens/queue IDs (send-and-drop), bounces (OPEN-26),
`mail.*` credential keys beyond §12.8's SMTP set (the swap is
config-shaped by a later slice if the next sender is not SMTP).

