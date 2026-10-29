# Prompt 059 — `csrf check failed` on the setup wizard

- Prompt timestamp (UTC): 2026-10-29T02:05:00Z
- Response timestamp (UTC): 2026-10-29T02:20:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

×
Setup unavailable.
csrf check failed

## Response

Real server bug, now fixed and deployed (`f4f44ac`,
`0.1.0+f4f44ac`). `setup_steps` — a GET handler — enforced
an Origin check, but `origin_allowed` fails closed on a
missing Origin and browsers never send `Origin` on
same-origin GET fetches. Every real browser 403d on wizard
steps while curl drills (which send Origin manually) and
unit tests (helper always sets Origin) passed — the bug was
invisible to both. The setup key was never the problem.

Fix: drop the Origin check from the steps GET (safe method +
SetupAuth key gate + unforgeable custom header = the CSRF
protection, per spec §9.1). The POST save keeps its Origin
check (browsers send Origin on JSON POSTs; harmless there).
`csrf_guard` comment in `admin.rs` updated to match.

Prod proof (browser-faithful GETs, no Origin): no key →
401 `setup key required`; wrong key → 403 `setup key
invalid or window closed` — the key vocabulary, not `csrf
check failed`. A correct key now 200s.

Lesson: drill scripts and test helpers must replicate
browser header semantics (no Origin on GET, no CSRF cookie
pre-enrol), not curl defaults.

## Files changed

- `crates/web/src/setup.rs` — removed Origin gate from
  `setup_steps`; added `browser_get` helper +
  `steps_load_without_origin_like_a_browser` test
- `crates/web/src/admin.rs` — `csrf_guard` comment: only the
  setup POST does its own Origin check

## Commits

- `f4f44ac` Drop Origin check on setup steps GET
- (this log; hash on commit)

## Verification

- `cargo xtask ci`: exit 0, 0 failed (6/6 setup tests)
- `ops-deploy.sh ... --skip-accept`: `deployed f4f44ac`
  (skip announced; no content/schema surface)
- Prod: `/healthz` → `0.1.0+f4f44ac`; steps GET without
  Origin reaches SetupAuth (401/403 key errors, no CSRF
  failure)
