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
