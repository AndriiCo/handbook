# Staging plan (box arrives later; gate enforced from day 10)

## What staging is for (in priority order)

1. **Acceptance gate for prod.** Every release: deploy to
   staging → run the full acceptance suite FROM staging
   (against staging URLs + staging DB) → stamp →
   prod. No stamp, no prod. This is the control that replaces
   the day-1–10 direct path.
2. **Migration proving ground.** Every migration runs first on
   a staging DB cloned from prod data SHAPE (never prod data
   itself — staging holds synthetic fixtures only). A
   migration that fails or slows on staging never ships.
3. **External-integration sandbox.** Carrier label buys, Stripe
   test mode, Meta API calls, ACME staging certs — all
   third-party traffic is proven on staging with test
   credentials before prod keys are touched.
4. **Load + failure drills.** Single-box limits (SQLite WAL
   under WS fan-out), disk-full behavior, OOM behavior,
   unit restarts, cert renewal dry-runs.
5. **NOT for:** dev experiments (use the Mac), content
   authoring (use fixtures), or anything holding real
   customer PII (staging is synthetic-only by policy).

## Shape (when the box lands)

- Same OS/image as prod (Ubuntu 26.04+, x86_64), same systemd
  unit except hostnames/ports + ACME staging CA
  (`https://dv.acme-v02.test-api.pki.goog/directory`, own EAB
  secret — browsers will NOT trust it; curl needs `--cacert
  test_roots.pem` from `https://pki.goog/test_roots.pem`).
- Hostname TBD by author (suggestion: `staging.andrii.ai` or a
  path on an internal host — never the prod IPs).
- Data: synthetic fixtures checked into the repo
  (`crates/web/tests/`, handbook fixtures) + generated volume
  data. A `xtask seed-staging` task (to build) loads a known
  state; acceptance asserts against it, then wipes.

## Acceptance suite (from staging, before prod)

`cargo xtask accept --base https://<staging>/apostille`
(to build, Day 8–9): health/version shape, SPA shell + deep
links 200, demo login → me → 401 matrix, analyze round-trip
on fixtures, retention sweep accounting, migration
forward-roll on a scratch DB, web-budget gate, secrets-scan,
plus every user-journey happy path in `user-journeys.md`.
Exit 0 writes a stamp file
(`acceptance/<sha>.json`: sha, time, staging host, suite
version); `xtask release` requires a stamp for the exact sha
being shipped. Stamps live in the repo (audit trail, no PII).

## Cutover checklist (day 10)

- [ ] Staging box online, same unit shape as prod.
- [ ] `xtask accept` green twice in a row on staging.
- [ ] One full drill: staging deploy → accept → stamp →
      prod deploy → smoke → rollback drill.
- [ ] Direct Mac→prod path documented as forbidden; `xtask
      release` enforces the stamp.
