# Prompt 062 — Playwright suite for the wizard save flow

- Prompt timestamp (UTC): 2026-10-29T03:42:00Z
- Response timestamp (UTC): 2026-10-29T03:50:46Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

OK. Have you tested Save UI in any of test-cases? Can we add
Playwrith test-suite for this part of app (and later for
others parts)?

## Response

No, the Save click-to-fetch seam was untested (node tests
mock fetch; Rust tests cover the handler if called). Added
the suite (`0ef6336`, CI green incl. the new gate).

Suite shape: `web/playwright.config.mjs` (chromium, no
webServer — specs serve builds on port 0 per repo rule),
`web/e2e/helpers/serve.mjs` (static + SPA fallback server),
`web/e2e/setup-wizard.spec.mjs` (key → steps → edit → Save
→ POST URL/header/body asserted → Saved notice),
`web/e2e/README.md` (one-time browser install, conventions).
`npm run test:e2e` wired into `cargo xtask ci` after smoke
(builds fresh from the same run). `PW_CHANNEL=chrome` runs
system Chrome for local iteration.

The suite found a real bug on its first run: the post-save
progress refetch reset `saved:false` on every row, wiping
the "Saved." notice instantly. `loadSteps` now carries the
flags across (3-line fix in `+page.svelte`).

Note: this notice bug is NOT the prod save failure (prod
audit shows zero save POSTs reaching the server at all —
client-side, exact error text still requested). The new spec
guards the seam so any click-to-fetch breakage fails CI.

## Files changed

- `web/playwright.config.mjs`, `web/e2e/` (new suite)
- `web/package.json`, `web/package-lock.json` (+runner)
- `web/.gitignore` (run output)
- `xtask/src/main.rs` (ci step)
- `web/src/routes/(admin)/admin/config/+page.svelte`
  (saved-flag carryover)

## Commits

- `0ef6336` Playwright E2E suite + Saved notice fix
- (this log; hash on commit)

## Verification

- `npx playwright test`: 1/1 green (bundled chromium)
- `cargo xtask ci`: exit 0, Rust 599/0 + E2E 1/1 in-gate
- Suite failed pre-fix (notice wiped), passes post-fix
