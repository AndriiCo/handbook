# Test plans

Every user-visible capability has a test plan: what guards it
today (Rust handler tests, node client tests, Playwright E2E)
and what is queued. Critical User Journeys additionally carry
**interaction budgets** — asserted in E2E, ratcheting down.

- [Customer journeys J1–J6](cuj-customer.md) — anonymous
  lookup, auth, forms, labels, uploads, orders.
- [Admin journeys CUJ-1–6](cuj-admin.md) — first boot,
  review, watch, voice, suppression, re-key.
- [Gaps](gaps.md) — thin/missing coverage and queued specs.

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

Customer (details: [cuj-customer.md](cuj-customer.md)):

| ID | Journey | Status | Budget A/I |
|---|---|---|---|
| J1 | Anonymous lookup → dossier | E2E proven | 2 / 1 |
| J2 | Register / login / manage methods | queued (needs credential stub) | TBD |
| J3 | Fill form → PDF | queued | TBD |
| J4 | Shipment → labels | queued | TBD |
| J5 | Uploads → precheck | blocked (no backend) | — |
| J6 | Order → track → pay | blocked (shells only) | — |

Admin (details: [cuj-admin.md](cuj-admin.md)):

| ID | Journey | Status | Budget A/I |
|---|---|---|---|
| CUJ-1 | First boot → enrolled | configure slice proven | 3 / 3 per step |
| CUJ-2 | Proposed → accepted fact | queued | TBD |
| CUJ-3 | Watch → capture → draft | queued (shell step) | TBD |
| CUJ-4 | Call → transcript → quote | queued | TBD |
| CUJ-5 | Reject → suppressed | queued | TBD |
| CUJ-6 | Secret rotation | queued | TBD |

CUJ-1 note: the configure slice went 5 actions → 3 per
step (auto-save on navigation, prompt 063). Full CUJ-1
(9 steps + enrol ceremony) stays manual until a virtual
authenticator lands (see [gaps](gaps.md)).

## Capability coverage matrix

From the 063 audit (Rust ~590, node ~95, E2E 3):

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

Thin rows and the E2E queue live in [gaps](gaps.md).
Shell-only UIs (orders, billing, uploads POST) and
spec-only items (F7 chains, F8/F9/F10, MCP tools, autofill)
get E2E only after their backends land — no E2E against
"not wired yet" placeholders.
