# 10-URL load check (roadmap item 11)

Proves one modest server can sweep the Day-2 watch seed in a bounded
window with per-domain politeness (F1 acceptance 5, scoped to 10).
Runs ON the box (research traffic may originate ONLY from the prod
IPs — never from dev machines).

## The 10 seed URLs (drawn from `packs/` watch seeds)

1. `https://www.hcch.net/en/instruments/conventions/status-table/?cid=41` (hcch, daily)
2. `http://www.sos.wa.gov/about-office/contact-us` (usa-wa, daily)
3. `https://www.sos.wa.gov/sites/default/files/2025-01/Apostilles.Request.Form_.12.2024.pdf` (usa-wa, weekly; expect 404)
4. `https://dos.ny.gov/apostillecertificate-authentication-request-forms` (usa-ny, daily)
5. `https://www.businessexpress.ny.gov/app/answers/cms/a_id/3423` (usa-ny, weekly)
6. `https://www.sos.ca.gov/` (usa-ca, weekly)
7. `https://www.sos.state.tx.us/` (usa-tx, weekly)
8. `https://dos.fl.gov/` (usa-fl, weekly)
9. `https://travel.state.gov/content/travel/en/passports/resources/authentications-sidebars.html` (usdos, daily; expect 403 until headless capture lands)
10. `https://egyptembassy.net/consular-services/egyptian-consulate-in-us/` (egypt-consular, daily)

The remaining 6 pack watch seeds (3 more DC legalization pages, 2 LA
pages, 1 Viet Nam-attributed HCCH seed) join the sweep once the
10-URL gate is green; the full 500-URL crawl is Week 3+ (F1).

## Procedure (after `fetch-url` lands; loop track owns the task)

```sh
# On the box, scratch store seeded from packs:
export STORE=/tmp/watch-check && rm -rf $STORE
cargo xtask packs-load --store $STORE
# Sweep the 10 seeds 3x, recording wall time per pass:
for i in 1 2 3; do
  time cargo xtask fetch-url --store $STORE --limit 10
done
```

## Pass criteria

- All 10 attempted per pass; no silent skips.
- Byte-identical re-fetches create no new work (sha256 dedup);
  pass 2–3 must add zero `capture` rows when nothing changed.
- Expected failures (seed 3's 404, seed 9's 403) refresh a
  `task(kind=validate_fact)` each — never a crash, never a skip.
- Whole 10-URL pass completes in < 5 minutes including
  per-domain politeness delays; any slower → file a perf task.
- Results logged in the drill log (`drill.md`) with date + build.

## Interim (before `fetch-url` exists)

Do NOT curl the seeds from a dev machine. The interim check is
`cargo xtask packs-validate` (seeds well-formed, every host has a
verdict row) plus `packs-load` into a scratch store (16 `source`
rows, 0 `fact` rows). Network timing starts with `fetch-url`.
