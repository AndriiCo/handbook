# Prompt 038 — Phase 3 Apple + Meta OAuth (streams + parent implementation)

- Prompt timestamp (UTC): 2026-10-28T04:53:23Z
- Response timestamp (UTC): 2026-10-28T04:56:01Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Resume work towards the goal.
```

## Response

Phase 3 (Apple + Meta OAuth) completed end to end: 4 subagent
streams ran first (spec, backend, UI, guide/testplan — launched
under the prior resume prompt; drafts recovered to
`/tmp/phase3-{spec,backend,ui,guide}.txt`), then the parent
implemented everything as single writer per AGENTS.md §6.

Decisions and assumptions:

- Flat paths `/auth/apple/*`, `/auth/meta/*`; separate state
  kinds (`kind='apple'/'meta'`, the confused-deputy boundary)
  + per-provider binding cookies (`hb_apple`/`hb_meta`); one
  shared 302 code vocabulary and provider-agnostic SPA copy
  (decision 33 in `docs/specs/README.md`).
- Apple ES256 via `jsonwebtoken 11` on the `aws_lc_rs`
  backend (manifest verified ring-free; hand-rolled JWS
  rejected); Meta `appsecret_proof` via `aws-lc-rs` HMAC
  (RFC-4231 test vector in-suite). No new crypto code.
- Meta is sub-only by design: no email scope requested, no
  email fetched, NULL email hash, `link_required` never fires.
- Graph API `v26.0` pinned (highest valid version probed live
  2026-10-28); Apple `response_mode=query` (OPEN-9 confirms at
  first real login); no Apple nonce (state+binding covers it);
  no Meta JS SDK.
- Registry 36→42 keys
  (`apple.client_id/team_id/key_id/private_key`,
  `meta.client_id/client_secret`); boot always succeeds, halves
  fail closed (503 / `oauth_unavailable`).
- UI: `google.js` → provider-parameterized `oauth.js`, one
  agnostic error map; login/security pages share one helper
  per action across all three providers.
- web-budget raised 220→221KB (documented in xtask): the
  +456B net is approved Phase-3 scope (two providers'
  buttons/copy + shared helper); chunks stay code-split.
- Test P-256 keys are generated per test via aws-lc-rs (no
  embedded key material — the pre-commit secrets-scan forbids
  even synthetic PEM armor; armor assembled from fragments).

Files changed (commit `27bef58`):

- Spec: `docs/specs/customer-auth.md` (shared D1–D5, §§10–11,
  link/unlink generalization, OPEN-9..14),
  `docs/specs/README.md` (decision 33).
- Guide: `docs/ops/config-values.md` (Apple/Meta sections),
  `docs/auth-playbook.md` (no-nonce, no-JS-SDK, sub-only
  notes; §5 Meta mention).
- Config: `crates/config/src/lib.rs` (6 keys, overlays, 42-key
  test).
- Auth: `crates/auth/Cargo.toml` (jsonwebtoken 11 aws_lc_rs,
  aws-lc-rs, base64 dev), `src/lib.rs` (mods),
  `src/oauth.rs` (kinds, kind-param state fns, `get_query`),
  new `src/apple.rs` + `src/meta.rs` with unit suites.
- Web: `crates/web/Cargo.toml` (dev-deps), `src/main.rs`
  (mods, AppState fields, routes, cfg builders), new
  `src/oauth_apple.rs` + `src/oauth_meta.rs` (handlers +
  port-0 stub suites), `src/oauth.rs` (`get_query` impl),
  `src/passkey.rs` (unlink gate + session proof generalized,
  new test), AppState literals in `admin_voice.rs`,
  `admin_workers.rs`, `p2.rs`, `p2_labels.rs`.
- UI: `web/src/lib/customer/oauth.js` (renamed from
  `google.js`), `web/test/oauth.test.mjs` (renamed),
  `login/+page.svelte`, `security/+page.svelte`.
- Gates: `xtask/src/main.rs` (221KB budget), `Cargo.lock`
  (+jsonwebtoken, +simple_asn1, +zeroize_derive only).

Verification performed (all observed this turn):

- `cargo xtask ci` → exit 0 (fmt, clippy `-D warnings`,
  29 suites ok: auth 51, web 140 incl. 21 new OAuth + new
  unlink test, config 24 incl. 42-key registry).
- Tamper-fixture flake caught and fixed (deterministic sig
  flip; 8/8 repeat runs green).
- `node --test web/test/` → 46/46 pass (oauth suite 9/9).
- `npm run check` → 0 errors; `check:imports` OK; `build` +
  `build:admin` done; `smoke` OK.
- `cargo xtask web-budget` → 225690/226304 green;
  `packs-validate` clean (9 packs).
- `cargo xtask secrets-scan` → clean (313 files); `--staged`
  clean.
- `cargo tree -p jsonwebtoken -e normal` → aws-lc-rs backend,
  no ring; `cargo tree -p auth -e normal -i ring` → empty;
  ring still enters only via rcgen (unchanged).
- No lingering `google.js`/`startGoogle` references in
  `web/src`, `web/test`, `web/scripts`.

## Commits

- `27bef58` — Phase 3 work (this entry describes it).
- (This log entry + README index: separate commit, next.)

## Follow-ups

- OPEN-9..14 carried in spec §8 (Apple wire confirmation,
  JWKS cache, events receiver, Graph sunset watch, Meta
  deletion tracker, profile display).
- Stage + deploy `27bef58` as hal, verify live
  (503/302 vocabulary on the new routes), then run the
  config-values staging checklists (needs operator: Apple
  test IDs, Meta dev-mode testers, portal registrations).
- Operator still pending from before: Telnyx keys + 8am proof
  call, www 3rd SAN, Google console live ceremony, legacy
  `apostille-ship` grant removal.
