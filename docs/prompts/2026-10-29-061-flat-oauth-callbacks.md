# Prompt 061 — server-level OAuth return URLs

- Prompt timestamp (UTC): 2026-10-29T03:25:00Z
- Response timestamp (UTC): 2026-10-29T03:39:29Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

Please change Apple Auth return URL to be server specific, not
the /apostille/ one. I need it to be
https://andrii.ai/auth/apple/callback Same idea for all others
Google/Meta - it's logging in to the entire server of andrii.ai
and not specific sub-service we currently testing market for.

## Response

Done (`3f14f34`, CI 599/0, not deployed). The three callbacks
moved from `/apostille/api/v1/auth/<p>/callback` to flat
`/auth/<p>/callback` on the public-router root; `redirect_uri`
derives `{auth.origin}/auth/<p>/callback`. Start endpoints
stay nested (internal API — the SPA calls them via its
`/apostille/api/v1` prefix, so no SPA change needed).

Pre-existing alignment found: spec D1, the handler doc
comments, and the Meta derived-URI line already named the
flat form — only the server routes, Google/Apple derived
lines, wizard guides, and playbook were stale. All fixed
(11 URL replacements, zero old callback paths left in
source/docs). Playbook webhook URLs (apple events, google
risc, meta deauthorize) went flat too — unbuilt endpoints,
but provider-registered, so the same principle applies
(spec already names flat `/auth/apple/events`).

Operator note: prod still serves the OLD callback until the
next deploy, and the prod wizard still shows the old guide
URL. Register the FLAT urls in provider consoles; if a
provider was already registered with the old path, edit it
to the flat form before the first real login.

## Files changed

- `crates/web/src/main.rs` — routes moved to router root (3
  off inner, 3 on outer) + 3 redirect_uri formats
- `crates/web/src/oauth.rs`, `oauth_apple.rs`,
  `oauth_meta.rs` — test URIs + cfg values (9/10/9)
- `web/src/lib/admin/setup-guides.js` — 3 guide URLs
- `docs/specs/customer-auth.md` — Google + Apple derived lines
- `docs/auth-playbook.md` — 6 provider-console URLs

## Commits

- `3f14f34` Flat server-level OAuth callbacks
- (this log; hash on commit)

## Verification

- `cargo xtask ci`: exit 0, 599/0 (oauth 31/31)
- `grep`: 0 old callback paths in crates/web/docs source
