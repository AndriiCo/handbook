# Test plans

Every user-visible capability has a test plan: what guards it
today (Rust handler tests, node client tests, Playwright E2E)
and what is queued. Critical User Journeys additionally carry
**interaction budgets** — asserted in E2E, ratcheting down.

- [Customer journeys J1–J6](cuj-customer.md) — anonymous
  lookup, auth, forms, labels, uploads, orders.
- [Admin journeys CUJ-1–6](cuj-admin.md) — first boot,
  review, watch, voice, suppression, re-key.
- [Gaps](#gaps) — what is still thin or untested, at the end of
  this page.

## Interaction metric

A CUJ spec drives the real built SPA through
`web/e2e/helpers/cuj.mjs`, which counts two numbers:

- **actions** — clicks, checks, selects, submits,
  navigations. The "how many clicks to the goal" number.
- **inputs** — text fills, counted separately. Typing is
  irreducible, but prefill, options, and defaults reduce it.

Rules:

1. Each CUJ spec ends with `expect(journey.summary())`
   against its budget. A UI change that adds steps fails CI
   until the budget line is consciously raised **with a
   recorded reason** — budgets ratchet down, never drift up.
2. API traffic is route-mocked (hermetic, synthetic fixtures
   only); the page is the real build. Budgets measure the
   UI, not the backend.
3. Plain-language corollary ("understandable under 30"):
   every counted step must read at a glance — one verb per
   button, no dead ends, errors as copy, never codes. The
   budget enforces it structurally: fewer steps leave no
   room for cleverness.

## Journey catalog

Statuses name the Playwright spec under `web/e2e/` that drives the
journey (route-mocked API, real built SPA). Budgets live in the
journey docs; a journey without a `journey.summary()` budget yet says
so.

Customer (details: [cuj-customer.md](cuj-customer.md)):

| ID | Journey | Status |
|---|---|---|
| J1 | Anonymous lookup → dossier | E2E, budgeted (`cuj-lookup`, `cuj-dossier`) |
| J2 | Register / login / manage methods | E2E for staff passkeys (`admin-first-run`, `admin-staff`, credential stub); customer sign-in via `order-fill` |
| J3 | Fill form → PDF | E2E (`order-fill`) |
| J4 | Shipment → labels | E2E (`admin-labels-autofill`, `admin-labels-errors`) |
| J5 | Uploads → precheck | demo walk only (`demo`); no dedicated spec |
| J6 | Order → track → pay | E2E (`commerce`, against the contract fixtures) |

Admin (details: [cuj-admin.md](cuj-admin.md)):

| ID | Journey | Status |
|---|---|---|
| CUJ-1 | First boot → enrolled | E2E (`admin-first-run`, `setup-wizard`, `cuj-wizard`) |
| CUJ-2 | Proposed → accepted fact | E2E (`admin-bulk`, `admin-considerations`, `admin-compose`) |
| CUJ-3 | Watch → capture → draft | E2E for the compile run (`admin-compile`); the sweep itself is an xtask test |
| CUJ-4 | Call → transcript → quote | not in E2E; Rust handler tests + `xtask voice-replay` |
| CUJ-5 | Reject → suppressed | not in E2E; Rust handler tests |
| CUJ-6 | Secret rotation | E2E for step-up and write-only secrets (`admin-config-stepup`, `admin-secrets`); rotation is an xtask test |

## Capability coverage matrix

From the 063 audit (2026-10-28: Rust ~590, node ~95, E2E 3). The E2E
column has grown since (see the catalogue above); the Rust and node
counts are not re-measured.

| Area | Rust | node | E2E | Verdict |
|---|---|---|---|---|
| Magic link | 29 | 9 | — | covered |
| OAuth ×3 | 36 | 7 | — | covered |
| Customer passkey | 25 | 7 | — | covered |
| Staff enrol/login/step-up | 19+ | 14 | — | covered |
| Sessions/CSRF/mTLS/host | ~32 | 1 | — | covered |
| Setup wizard | 34 | 7 | 2 | covered |
| Review queue | ~24 | 7 | — | covered |
| Calls/voice | 38 | 8 | — | covered |
| Labels | 48 | — | — | covered (no JS tests) |
| Forms P2 | 29 | 14 | — | covered |
| Mail poll/Gmail | 25 | — | — | covered (server only) |
| Ingest/sweep/HCCH | ~47 | — | — | covered |
| Public handbook | ~9 | 11 | 1 | covered |
| Admin config UI | ~28 | 5 | 1 | covered |
| Media upload/redact | 12 | — | — | thin |
| WS live updates | 12 | 4 | — | thin |
| MCP transport | 2 | — | — | thin |
| Publish/static build | ~8 | — | — | thin |
| Demo drill | ~8 | — | — | thin |

## Gaps

- **E2E still missing:** J5 uploads → precheck as its own spec;
  CUJ-4 call → quote and CUJ-5 suppression; a full-ceremony passkey
  run with a CDP virtual authenticator (handler tests cover the
  crypto today).
- **Thin server/client rows:** media upload/redact/transcribe, WS
  live updates per topic, the MCP transport, the publish/static
  build failure matrix (partial write, concurrent build), and the
  labels/media/publish node clients (thin fetch wrappers).
- **Deliberately untested in a browser:** shell steps
  (`setup-key-mint`, `xtask analyze`, KEK rotation; covered by xtask
  Rust tests) and provider consoles (Google/Apple/Meta, Telnyx;
  mocked at our boundary, confirmed by the operator runbooks).
