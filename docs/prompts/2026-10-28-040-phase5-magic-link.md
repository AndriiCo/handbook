# Prompt 040 — Phase 5 magic-link fallback (streams + parent implementation + prod deploys)

- Prompt timestamp (UTC): 2026-10-28T06:47:00Z
- Response timestamp (UTC): 2026-10-28T14:50:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Resume work towards the goal
```

(+ two `request_user_input` rounds answered in-turn: mailer
provider, fallback window.)

## Response

Phase 5 (magic-link fallback — the last auth method) completed
end to end: 4 read-only streams (spec, backend, UI, guide —
drafts recovered from child session logs to
`/tmp/phase5-{spec,backend,ui,guide}.txt` after truncated
delivery), parent single-writer implementation per AGENTS.md
§6, CI + web gates green, committed, cross-built, staged, and
deployed to prod twice (feature + one follow-up fix), all
verified live.

Author decisions (asked, answered in-turn):

- Mailer: Gmail SMTP for andrii.ai initially (rejected
  API-via-ureq, SMTP-via-lettre-as-framed, and file-only, then
  picked Gmail SMTP on the re-ask). Host/port/creds/From stay
  config rows for a future own-sender swap (OPEN-25 resolved).
- Fallback window: 7 days + 1h idle (OPEN-2 resolved).

Parent reconciliation (drafts conflicted hard):

- POST-only JSON endpoints, NOT the spec draft's GET consume:
  `request_id_and_log` logs the full `Uri` including query and
  asserts URLs never carry tokens; a magic token is a
  standalone bearer (unlike the OAuth `code`, useless without
  the server-side `client_secret`). Token travels in the URL
  fragment (`#t=`) + POST body. UI draft's POST design won.
- Owner resolved fresh at consume (spec's
  `owners_by_email_hash`, any method incl. relay, excl.
  disabled; ambiguous → fail closed); backend's request-time
  `account_by_any_email` (LIMIT-1 silent pick, misses relay)
  rejected.
- No CSRF echo on consume (Origin-only): backend's echo
  requirement would break cross-browser consume (the point).
- No audit on request (OAuth/passkey-start precedent;
  backend's always-audit rejected as a DB-growth vector).
- Single 410 for miss/expired/replay (no 409 branch);
  malformed shape → 400. Honest audit underneath via a
  take-failure peek (`replay`/`expired`/`invalid`).
- Flip-only: no `magic_link` identity rows (spec won); the
  guide draft's link/unlink staging items rewritten to match.
- Rate keys `mr:`/`mx:` with the backend's hourly
  per-target anti-harassment window (3/hour).
- Table `magic_token`, no `user_id` (resolve fresh).
- Request endpoint named `start` (2-vs-1 + §2 vocabulary).

Bug caught by walking the prod deploy (fixed forward as
`1afa66a`, no history rewrite): the shipped default
`mailer.provider = "file"` would silently write real users'
links to server-side files on an unconfigured prod box. Now
unset-by-default → loud 503 (the OAuth unset→503 contract);
dev sets `file` explicitly. `342d142` was verified live
BEFORE the fix (200/400/410 vocabulary, new login chunk,
zero stray mail files), then the fix deployed over it.

Byte budget: the dedicated `/login/magic` route missed the
gate (+3816 raw; the UI estimate used gzip, the gate measures
raw). Folded consume into the login page (option A, saves a
whole chunk; success UX now mirrors passkey login), then
bumped 221→223KB documented in xtask (precedent: every phase
bumps with a comment). 227556/228352. Also fixed two stale
`220KB gate` comments naming the gate summaries changed.

Files changed (work commit `342d142`):

- Spec/guide: `docs/specs/customer-auth.md` (§12 + OPEN-2
  resolved + OPEN-25 resolved + OPEN-26/27/28),
  `docs/specs/README.md` (D35), `docs/auth-playbook.md`
  (§5 Gmail + §2 relay-sender step), `docs/ops/config-values.md`
  (mailer section + 12-item staging + rollback),
  `docs/ops/runbook-release.md` + `docs/ops/deploy.md` (gate
  number), `xtask/src/main.rs` (223KB).
- Backend: `crates/auth/src/magic.rs` (new: trait, link
  build, store fns, fallback mgmt),
  `crates/auth/src/lib.rs` (`magic_token` DDL, fallback
  consts, `valid_with_fallback`, `session_usable`),
  `crates/config/src/lib.rs` (7 keys, 42→49),
  `crates/web/src/magic.rs` (new: SMTP/file mailers,
  handlers, 18 tests), `crates/web/src/main.rs` (routes,
  AppState, boot), `crates/web/Cargo.toml` (lettre ring-free
  pin), `crates/web/src/passkey.rs` (`session_of` +
  `valid_email` visibility), `crates/web/src/oauth.rs`
  (`ensure_provider` shared), `crates/web/src/ws_bus.rs` +
  `p2.rs` (`session_usable`), 8 test-literal files,
  `Cargo.lock` (lettre 0.11.23; rustls 0.23.46 unified,
  aws-lc-rs; only ring edge remains grandfathered rcgen;
  base64 0.23 dup noted).
- UI: `web/src/lib/customer/magic.js` (new),
  `web/test/magic.test.mjs` (new, 9 tests),
  `web/src/routes/(customer)/login/+page.svelte` (form +
  folded consume).

Files changed (fix commit `1afa66a`): default provider ""
+ fresh-boot warning + tests, spec §12.8 + guide to match.

Verification performed (all observed, none assumed):

- `cargo xtask ci` exit 0 (fmt, clippy `-D warnings`,
  full suite incl. 11 new auth + 18 new web tests).
- `node --test web/test/` 66/66 (57 + 9 new).
- `svelte-check` 0 errors 0 warnings; `check:imports` OK.
- `npm run build` + `build:admin` + `smoke` OK.
- `web-budget` 227556/228352 green; `packs-validate`
  clean (9); `secrets-scan` clean (322 files).
- `cargo tree` ring-free proofs (lettre→rustls 0.23.46 +
  aws-lc-rs; single ring edge = grandfathered rcgen).
- `cargo zigbuild --release -p web -p xtask --target
  x86_64-unknown-linux-gnu` (ELF verified, embedded short
  SHA matches HEAD, old SHA absent on rebuild).
- Staged `/home/hal/apostille/staging/{342d142,1afa66a}/`
  (8 entries each, 40-hex SHA, no nested-dir issue).
- Deployed as hal via `ops-deploy.sh --skip-accept`
  (announced both times: zero packs diff, no numbered
  migration — `magic_token` self-migrates idempotently at
  boot, old binary ignores it; additive-only rollback).
- Live on `342d142`: healthz `0.1.0+342d142`; start
  unknown → 200, malformed → 400 `email malformed`;
  consume garbage → 410; login 200 + new chunk
  `7.ClDG3iUS.js` serves the form; `/var/lib/apostille/mail/`
  empty.
- Live on `1afa66a`: healthz `0.1.0+1afa66a`; start
  unknown → 503 `magic login unavailable` (the fix);
  consume garbage → 410.

Deploy wart (finding, not fixed — release-tooling scope):
`xtask deploy` cuts over BEFORE pruning, so the `1afa66a`
deploy went live then reported ABORT on
`prune c1960da: Permission denied` (5 pre-hal-era releases
are root-owned: c1960da, 2856e16, 69480d3, ebf2bc5,
f5d199f). Prod is correctly on `1afa66a`; releases/ sits at
11 until root removes the 5 stale dirs (command handed to
the operator; keeps current + 5 recent hal-owned rollback
targets). After that cleanup every future hal prune
succeeds. Consider making prune failures warn instead of
ABORT-post-cutover in a later xtask slice.

## Commits

- `342d142` Phase 5: magic-link fallback (spec §12, backend,
  UI, guide)
- `1afa66a` Fix: mailer.provider unset by default (no silent
  file default in prod)
- (this log entry commits separately, per convention)

## Follow-ups

- OPERATOR (root, one command): remove the 5 root-owned
  stale releases so the tree is back within the 10-cap and
  hal owns the full lifecycle:
  `sudo rm -rf /srv/apostille/releases/c1960da
  /srv/apostille/releases/2856e16
  /srv/apostille/releases/69480d3
  /srv/apostille/releases/ebf2bc5
  /srv/apostille/releases/f5d199f`
  (keeps 1afa66a live + 342d142/f2848a8/9c28dd9/bb54d04/1c6856c
  hal-owned history).
- OPERATOR (prod keys): paste Gmail `mailer.*` rows
  (`provider=smtp`, host, port, username, App Password,
  from) + restart before any real magic-link use; prod
  currently 503s by design. Apple relay-sender registration
  required before relay-address delivery (guide step 5).
- Staging drill (12 items in config-values.md) still needs a
  staging host + test inbox (box lands ~Oct 31 per
  staging-plan; prefetch watch OPEN-27 conditions on it).
- OPEN-26 (bounces), OPEN-28 (email-row removal) deferred
  by design; OPEN-9..24 unchanged (see spec §8).
- Pre-existing operator items unchanged: Telnyx keys + 8am
  proof call, www SAN, Google/Apple/Meta portal work,
  `apostille-ship` grant removal.
