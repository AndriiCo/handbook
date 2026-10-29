# E2E suite (Playwright, chromium)

Browser tests for the built SPAs — the seams unit tests cannot
see (clicks firing requests, notices rendering). API traffic is
route-mocked per spec; pages are the real `build`/`build-admin`
output. Each spec serves its build dir over 127.0.0.1 on port 0
via `helpers/serve.mjs` (repo rule: no fixed test ports) with
the adapter-static SPA fallback.

One-time setup (CI boxes and fresh clones):

```sh
cd web && npx playwright install chromium
```

Run (builds must be fresh — `cargo xtask ci` builds first):

```sh
cd web && npm run test:e2e            # bundled chromium (what CI runs)
cd web && PW_CHANNEL=chrome npm run test:e2e   # system Chrome (local iteration)
```

Conventions for later suites: one spec file per flow, API
mocked at `**/apostille/api/v1/**`, synthetic fixture values
only, no fixed ports, no real network.
