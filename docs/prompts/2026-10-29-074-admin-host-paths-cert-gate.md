# Prompt 074 — admin paths owned by host + cert gate; whole-project status

- Prompt timestamp (UTC): 2026-10-29T07:31:00Z
- Response timestamp (UTC): 2026-10-29T07:54:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

Can you please remove second admin from routes on admin domain  https://admin.andrii.ai/admin/console and use host  to differentiate it, make client certificate is required to access https://admin.andrii.ai (can be a config option) to protect all data, html and JS for admin console.  Resume work toward complete all. List progress so far and that next items can be done in 6-12 hours.

## Response

Implemented both asks; reported whole-project status in chat.

Author decisions (asked first): hard cut on old paths (no
redirects); cert required for everything except the setup
surface + Telnyx webhook; report scope = whole project.
Recorded as spec decisions 46-47 (plus parent decision 48:
`admin.require_cert` defaults off, refuses boot without CA).

Flatten: API `.../v1/admin/*` → `.../v1/*`; pages
`/admin/console` → `/console` etc.; staff login →
`/staff-login` (customer `/login` owns that path in the
shared bundle); `/` → `/console` redirect; dashboard page
removed (layout nav covers it). A routes-admin build split
was tried and reverted: it irreducibly breaks `svelte-check`
(one sync sees one route group; the full tree then has real
`/`+`/login` conflicts).

Cert gate: `admin.require_cert` key (58th registry key, 6th
bool), `require_client_cert` middleware on admin/workers/
voice-staff routers, webhook exempt by construction (no
layer), setup API + `/config` + `/_app/*` exempt by path.
Presence-only at edge; `AdminAuth` keeps registration.
Assumption: no audit row per denied asset fetch (access log
keeps ID/route); denial is JSON 403 even for pages.

Status probe: prod holds 0 authorities / 0 facts /
0 captures / 1 account — the loop has never run in prod.

## Files changed

53 files in `11fe6c9`: backend routes + guards + fixtures
(`admin.rs`, `admin_workers.rs`, `admin_voice.rs`, `main.rs`,
`staff_passkey.rs`, `setup.rs`, `queue.rs`, oauth/magic/p2
fixtures), config crate (`admin.require_cert` + refusal),
xtask invite/setup URLs, admin SvelteKit routes (5 moves, 1
delete, 1 rename), lib clients + node tests, e2e, smoke
manifest assertions, spec/ops/test-plan path references,
decisions 46-48.

## Commits

- `11fe6c9` (this work)
- (this log; hash on commit)

## Verification

- `cargo test -p web`: 225/0; `-p config`: 28/0.
  (One transient voice-test flake passed on rerun alone
  and in the full suite; not this change.)
- `node --test web/test/`: pass; `svelte-check`: 0/0.
- `cargo xtask ci`: exit 0 (fmt, clippy `-D warnings`,
  36 Rust suites, node, import gate, both builds, smoke
  OK on rewritten assertions, Playwright e2e, tailwind).
- Not deployed (no deploy asked this turn).

## Follow-ups

- Deploy `11fe6c9` (hard cut: old `/admin/*` paths 404;
  staff bookmarks/invite links minted before change 404 —
  re-mint invites).
- Operator opt-in: set `tls.client_ca_file` (already set
  on prod) + `admin.require_cert=true`, restart, verify
  403-without-cert on `/console` and API.
- Consider HTML (vs JSON) 403 for page denials; grow
  `error_kind` taxonomy; staging box by Oct 31.
