# SPA Separation: anonymous / customer / admin (draft for review)

- Status: draft (prompt 007, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (recovered from workflow delivery;
  the delivery truncated the draft's tail — everything below §1.5, if
  the researcher wrote more, was lost and is marked for re-draft on
  request). Companion: [analysis-agent.md](analysis-agent.md) (the
  second spec from the same stream, recovered separately in full).
- Context grounding: Rust workspace, axum 0.8 + tower-http, SvelteKit
  SPA + axum API (web-stack.md §7), WS-first realtime, single Ubuntu
  26.04 + Caddy TLS, BASE_PATH=/apostille, Mac local dev, no Docker,
  pure-Rust deps. Repo holds code+schema only, never customer data
  (design.md §0).

## 1.1 Goals

- Anonymous page load downloads extremely small JS (target: <40 KB
  brotli total for first view: Svelte runtime + router + public page).
  Customer and especially admin code must never be in that bundle.
- Admin UI not discoverable from public host: no admin routes, chunks,
  or links in public build; ideally separate origin.
- axum enforces all authz; frontend hiding is UX only, never a security
  boundary.
- Keep single-server ops: one axum binary + Caddy, same `cargo xtask
  dev/ci` gates on Mac and Ubuntu.

## 1.2 SvelteKit layout (one codebase, two deploy units)

- Single SvelteKit project, three route groups:
  `src/routes/(public)/...` — anonymous: handbook pages,
  country/authority view, pricing, Viet Nam demo landing;
  `src/routes/(customer)/...` — logged-in: orders, uploads,
  document-photo precheck status, walk-in report submit;
  `src/routes/(admin)/...` — staff only: review-conflicts queue,
  fact/task triage, call console, batches, audit view.
- Shared UI primitives in `src/lib/public-ui` (tiny, dependency-free).
  Customer-only and admin-only component libs in
  `src/lib/customer/*` and `src/lib/admin/*`; forbid cross-import via
  eslint rule + `xtask check-imports` (simple grep over imports; fail
  CI if `(public)` imports customer/admin).
- Code-splitting: SvelteKit already splits per route; additionally:
  - No static import of customer/admin libs from `(public)` layouts.
    Use `await import()` only after auth check for customer widgets
    embedded on public pages (e.g. order-status pill).
  - Vendor-heavy deps (WS client wrapper is fine; photo-annotation
    canvas, audio recorder, charts) live only inside
    `(customer)`/`(admin)` routes so Vite puts them in lazy chunks.
  - `export const prerender` / static handbook pages for anonymous
    where possible so first paint needs ~zero JS; hydrate only
    search/nav island.
- Two builds from same source (preferred over one build):
  - `web-public`: excludes `(admin)` routes (SvelteKit `routes` filter
    or separate svelte.config with `files.routes=src/routes-public`
    symlinking `(public)`+`(customer)`). Deployed at
    andrii.ai/apostille.
  - `web-admin`: full routes, deployed only on admin host. Admin
    chunks therefore never exist on public origin — no URL guessing.
  - Simpler interim (hackathon): one build, but Caddy/axum on public
    host refuses to serve `/apostille/admin*` and
    `/_app/immutable/...admin...` chunk names. Weaker (chunk names
    leak); upgrade to two builds before handling real customer PII.
  - Parent note: exact SvelteKit config keys for the two-build split
    must be verified against the installed SvelteKit version at
    scaffolding; the interim refusal rule is the fallback.
- Budget gate: `xtask web-budget` builds public bundle, asserts brotli
  bytes of anonymous entry < threshold, fails CI otherwise. Uses `cargo
  xtask` to wrap `npm run build` (the one sanctioned Node exception,
  web-stack.md §7.1).

## 1.3 Hosts and TLS (flag: admin.andrii.ui typo?)

- Decided 2026-10-26 (prompt 008): the admin host is
  `https://admin.andrii.ai` (`.ui` was a typo). TLS certs, Origin
  allowlist, and the mTLS client-CA all pin to this hostname.
- Public: `https://andrii.ai/apostille` → Caddy → axum
  127.0.0.1:PORT (plain HTTP, web-stack.md §1).
- Admin: `https://admin.andrii.ai/` (no sub-path; separate origin ⇒
  separate cookies, no public JS can touch admin session). Same
  physical server, Caddy SNI-routes to second axum listener (e.g.
  127.0.0.1:PORT_ADMIN) or same listener with `Host` guard. Prefer two
  listeners so firewall/Caddy policy differs.
- mTLS on admin host only: Caddy `tls { client_auth { mode
  require_and_verify; trusted_ca_cert_file ... } }`. Caddy forwards
  `X-Client-Cert-DN` + `X-Client-Cert-SHA256` on localhost only; axum
  rejects those headers from any non-loopback peer and rejects
  requests without them on admin listener. Fallback if Caddy
  client-auth proves painful: in-app rustls via axum-server on admin
  listener only (web-stack.md §1 already names it as fallback); public
  stays plain-HTTP-behind-Caddy.
- Local dev: `cargo xtask dev` serves both on localhost with different
  ports (public :3000/apostille, admin :3001/) and `ADMIN_MTLS=off` +
  printed warning; CI asserts prod config has it on.

## 1.4 axum auth design

- Sessions, not bare JWTs: opaque session id in cookie
  (`__Host-hb_pub`, `__Host-hb_adm`; HttpOnly, Secure, SameSite=Lax,
  Path=/). Server stores session row in SQLite (new tiny table
  `session(id, user_id, role, created_at, expires_at, ip_hash,
  ua_hash)` — never in git). Lookup per request via middleware;
  15-min idle timeout, 12-h absolute for customer, 8-h absolute +
  re-auth for admin destructive actions.
- Roles: `anonymous | customer | staff | admin`. Every API route
  declares min role via extractor `Auth(User{role})`; admin listener
  additionally requires verified-cert header + role staff/admin
  (defense in depth: cert alone gives nothing without login; login
  alone without cert rejected on admin listener).
- Login (decided 2026-10-26, prompt 008; full checklist in
  [../auth-playbook.md](../auth-playbook.md)):
  - Customer: passkey first, Sign in with Apple, Sign in with
    Google, email click-to-login (magic link) as fallback only.
    Account linking follows the playbook's verified-email-only +
    explicit-link-flow rules (no silent auto-merge, no
    relay-string matching); Apple/Google revocation signals
    (consent-revoked/account-delete, RISC sessions-revoked et al.)
    auto-disable the affected identity link + sessions, all
    audited.
  - Staff/admin: method still open (recommend passkey or
    password+TOTP) PLUS mTLS cert. Cert identifies device; login
    identifies person; audit logs both.
- CSRF: cookie + `Origin`/`Host` check + double-submit token for
  mutations (SvelteKit fetch wrapper adds `X-CSRF` from
  cookie-readable second cookie).
- WS auth: upgrade request carries same session cookie; axum validates
  session BEFORE upgrade, binds topics to identity
  (`authority:{slug}` public-read ok; `call:{id}` staff-only;
  `order:{id}` owner-or-staff). No token-in-query (avoids log leaks).
  Presence events (web-stack.md §3) include user id; anonymous
  presence is random ephemeral id, never linkable.
  - Parent note: topic nouns were reconciled after this draft — the
    union is `authority:{slug}`, `call:{id}`, `submission:{id}`,
    `shipment:{id}` (P2 endpoint nouns win over `order`); freeze at
    scaffolding. See [README.md](README.md).
- Admin cert verification details: axum middleware `RequireAdminCert`
  checks loopback-only headers, compares cert SHA256 against
  `admin_cert` table (fingerprint, owner, revoked_at); revoked/expired
  ⇒ 403 + audit row. Cert issuance/rotation is manual `xtask
  admin-cert-{issue,revoke}` (small CA via rcgen/rustls-pemfile, pure
  Rust); CA key lives in STORE_PATH outside repo, backed up offline.
- Audit: every login/logout/session-expire, role change, fact/task
  status change, trust promotion, batch send, call consent, admin cert
  event appends `audit` row (design.md §6) with actor, cert
  fingerprint when present, before/after refs.

## 1.5 Threat notes (customer data)

- Repo never holds customer data (design.md §0) — enforce with `xtask`
  dev-seed only + CI grep for PII-looking fixtures; uploads land in
  store/blob outside git with 0600 perms.
- IDOR: authority/capture/task/order ids are sequential integers in
  design.md — every read checks (trust tier + owner + role);
  staff-only captures (audio, unredacted photos) never served to
  customer routes; ServeDir only exposes built web assets, never
  store/.
- XSS/PII theft: Svelte auto-escapes; forbid `{@html}` of handbook
  content without sanitizer (pull ammonia-like crate or server-side
  render); sensitive pages set `Cache-Control: no-store`.
- Upload abuse: size caps, sniff MIME (not extension), EXIF preserved
  only in originals server-side (design.md §2), derivatives stripped;
  SAM 3.1 face/plate masking before any community photo becomes
  visible to others.
- WS hijack: cookie+Origin check on upgrade; per-topic authz; server
  caps connections/IP; no sensitive data in envelope `type` names
  logged verbatim.
- Admin exposure: separate origin + mTLS + login; no admin chunk on
  public host (two builds); Caddy rate-limits admin login; failed
  logins audit + backoff.
- Supply chain: vendored JS only, no CDN (web-stack.md §2); Node used
  only at build time, never in prod runtime.

(Delivery truncated here — see header note. The companion
[analysis-agent.md](analysis-agent.md) fully covers the agent half of
this stream.)

## Open questions (from this stream)

- Decided in prompt 008: admin host (`admin.andrii.ai`), customer
  login (passkey/Apple/Google + magic-link fallback — see
  [auth-playbook](../auth-playbook.md)), Meta API spec (see
  [meta-api](../meta-api.md)).
- Content repo exact location + customer-data retention (only voice
  365d decided)?
- mTLS CA: who issues admin client certs, revocation story, how many
  admins?
- Egypt chain + Viet Nam demo: what PII enters captures and where is
  the redaction boundary?
