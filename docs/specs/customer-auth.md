# Customer Auth: passkeys + linked methods (draft for review)

- Status: draft (for author review; Phase-1 implementation alongside)
- Date: 2026-10-28
- Grounding: [auth-playbook.md](../auth-playbook.md) (§1 passkeys, §6 linking rules), [spa-separation.md](spa-separation.md) §1.4 (sessions/cookies/roles), [SECURITY.md](../SECURITY.md) §1–§2, `crates/auth/src/` (`lib.rs` sessions+roles, `login.rs` policy, `csrf.rs`, `ws.rs`), `crates/web` session/cookie handling, and webauthn-rs 0.5.5 API (builder + ceremony shapes + error variants, verified against vendored sources).
- Out of scope: OAuth/OIDC handlers for Apple/Google/Meta and the magic-link sender (playbook §2–§5 portal checklists stand; this spec defines the tables and link/unlink seams they plug into). Staff enrolment endpoints are Phase 4 (spec §6 here, tables land with the code).

## 0. Decisions embodied

1. Library: `webauthn-rs` 0.5.5. Features: default (`attestation`) + `danger-allow-state-serialisation`. The `danger` feature is our documented safe case: ceremony state persists server-side in SQLite (single-use + 5-min expiry), never in cookies — exactly what the crate docs bless. (`conditional-ui` arrives with §2.5 in Phase 4.) No `ring` from this stack (verified: crypto backend is openssl; the only `ring` in the graph is the pre-existing `rcgen` test-cert path).
2. OpenSSL is vendored (`openssl-sys` `vendored` feature, unified from the web crate): hermetic local builds (Mac system OpenSSL is v4, which openssl-sys may reject) and working `cargo zigbuild` cross builds (no target system libs). Precedent: the workspace already builds C deps (rusqlite bundled, aws-lc-sys).
3. RP ID `andrii.ai` serves **both** `https://andrii.ai` and `https://admin.andrii.ai`: `WebauthnBuilder::new(rp_id, rp_origin)` requires only that rp_id be an effective domain of rp_origin, and `.allow_subdomains(true)` accepts subdomain origins (verified in vendored builder docs + `allow_subdomains` semantics). Belt-and-braces: also `.append_allowed_origin(https://admin.andrii.ai)`. Browser-side RP-suffix rule still needs a live ceremony on each origin (OPEN-1).
4. Passkey ceremonies live under `/apostille/api/v1/auth/…` on **both** branches (public router for customers now; admin router for staff enrolment/login in Phase 4). Sessions, cookies, roles, CSRF, and mTLS composition are **reused unchanged** from `crates/auth` (§5).
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

Base paths: public branch `/apostille/api/v1/auth/…`; admin branch `/apostille/api/v1/admin/auth/…` (staff passkey enrolment/login only — Phase 4). All mutating endpoints require Origin check + double-submit CSRF. All DTOs use `#[serde(deny_unknown_fields)]` (SECURITY.md input-validation rule).

Wire note: challenge responses (`CreationChallengeResponse`, `RequestChallengeResponse`) and credential payloads (`RegisterPublicKeyCredential`, `PublicKeyCredential`) serialise per the crate's serde impls and pass through verbatim inside our envelopes.

### 2.1 `POST /auth/passkey/register/start`

Two modes: (a) logged-out first passkey (creates the account at finish), (b) logged-in add-passkey (explicit link flow, rule 4).

Request: `{"display_name": "Andrii", "label": "MacBook Pro"}` — `display_name` required in (a), ignored in (b); `label` optional, ≤64 chars. Mode (b) requires session cookie.

Success `200`: `{"challenge_id": "<opaque>", "options": { "<CreationChallengeResponse>" }}`. Server: `start_passkey_registration(user_uuid, name, display, exclude)` where `exclude` = existing non-revoked credential IDs (mode b) or `None` (mode a); persist state (`kind='register'`).

Errors: `400` bad display_name/label; `401` mode-(b) without session; `403` CSRF/Origin fail; `429` rate-limited; `500` store unavailable.

### 2.2 `POST /auth/passkey/register/finish`

Request: `{"challenge_id": "<opaque>", "credential": { "<RegisterPublicKeyCredential>" }, "email?": "…", "label?": "…"}` (client echoes the optional email + label from start; mode (a) stores the email as an unverified `identity_link` row for login resolution + Phase-3 recovery).

Server: load + consume challenge (missing/expired/consumed → `410`); `finish_passkey_registration(&cred, &state)`; assert credential ID globally unregistered (crate docs MUST — conflict → `409`); insert `account` (mode a) + `webauthn_credential` + `identity_link` in one transaction; mint session (mode a) or audit (mode b); audit row (`passkey_registered`, no PII).

Success `200` (mode a): `{"user_id": "…", "methods": ["passkey"]}` + `Set-Cookie`. Mode (b): `{"linked": true, "credential_id": "…", "methods": […]}`.

Errors: `410` challenge invalid/consumed; `409` credential already registered; `422` verification failed (log variant, return generic `{"error": "verification failed"}` — never leak crate internals).

### 2.3 `POST /auth/passkey/login/start`

Non-discoverable login. Request: `{"user_id": "…"}` (unknown user / zero creds → `401` generic, no enumeration).

Success `200`: `{"challenge_id": "…", "options": { "<RequestChallengeResponse>" }}` via `start_passkey_authentication(&[Passkey…])` (deserialised from `passkey_json`); state (`kind='login'`).

### 2.4 `POST /auth/passkey/login/finish`

Request: `{"challenge_id": "…", "credential": { "<PublicKeyCredential>" }}`.

Server: consume challenge; `finish_passkey_authentication(&cred, &state)` → `AuthenticationResult`; if `needs_update()`, `update_credential(&res)` + re-persist `passkey_json`; update `last_used_at`; clear `fallback_until`; `create_session` + cookie; rotate CSRF token (SECURITY.md: rotate on login); audit (`passkey_login`).

"Credential not found" (platform-side deletion, playbook §1): unknown credential ID → `404 {"error": "credential unknown", "action": "re-register"}` — the SPA routes to register, never an error dead-end.

Other errors: `401` verification failed (generic); `403` account locked (`locked_at` set — manual recovery); `410` challenge bad; `429` + backoff.

### 2.5 Discoverable login (conditional mediation / autofill) — Phase 4

Deferred: `finish_discoverable_authentication` takes `&[DiscoverableKey]`, which can only be built from attested resident-key types — not from the plain stored `Passkey`s. Wiring autofill needs the `resident-key-support` + attested ceremony path evaluated as its own spike (Phase 4, with staff login). Until then login is identifier-first (§2.3: `user_id` hex or linked email; the SPA remembers the last `user_id` in `localStorage`). The `webauthn_challenge.kind='discoverable'` value stays reserved. Planned shape when built: `POST /auth/passkey/discover/start` (no auth, `start_discoverable_authentication()`) and `POST /auth/passkey/discover/finish` (`identify_discoverable_authentication` → user → `finish_discoverable_authentication` → same session/audit path as §2.4). Unknown user/cred → same `404 re-register` shape.

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

## 5. Staff passkeys (Phase 4)

- Enrolment is **invitation-gated**: an admin mints `staff_invite` via `xtask admin-invite <role> <cn>` (prints single-use URL, stores `token_hash` only). Enrolment requires valid invite + UV ceremony + the invited client-cert CN presented. Finish consumes the invite, creates `account(role=staff|admin)` + credential + `identity_link`, audits with cert fingerprint. (Staff method decided 2026-10-28: passkey + mTLS cert.)
- Login: same ceremony shapes, UV required. Login alone grants nothing: every admin request still passes `AdminAuth` — session role staff/admin **AND** verified client-cert fingerprint in `admin_cert`.
- Destructive actions require fresh step-up (UV ceremony or re-login within 10 min).
- Offboarding: `revoke_admin_cert` + `revoke_user_sessions` + `webauthn_credential.revoked_at` + audit.

## 6. Rate limits, logging, PII

- Login/register finish 10/min/account + 60/min/IP with backoff; start endpoints 30/min/IP; in-memory mutex bucket, atomic check-and-decrement.
- Logs/audit: actor + action + entity + credential-ID/email **hashes** only; never session IDs, challenges, cookies, raw emails, or key material.

## 7. Verification (staging, before prod)

Extends playbook §8: register → logout → autofill login on Safari/Chrome/iOS; revoked-credential → re-register (never dead-end); cross-origin check — passkey registered on apex logs in on `admin.andrii.ai` and vice versa (OPEN-1 closes here); relay/Meta no-match fixtures; pre-account-takeover fixture red→green; last-method unlink refused; `cargo xtask ci` green.

## 8. OPEN items

| ID | Question | Owner | Blocks |
|---|---|---|---|
| OPEN-1 | Live-ceremony proof that RP ID `andrii.ai` + `allow_subdomains(true)` + `append_allowed_origin(admin)` serves both origins end-to-end | CODE (staging) | §5 admin login, Day-6 gate |
| OPEN-2 | Magic-link `fallback_until` window length + tighter idle value (playbook §5 says "tighter" without numbers) | SEC | Phase 3 timeouts |
| OPEN-3 | Staff enrolment if passkey stalls: fallback scope | Author | §5 enrolment |

