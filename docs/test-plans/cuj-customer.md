# Customer CUJs J1–J6

Public SPA (`web/build`, base `/apostille`). Mock API at
`**/apostille/api/v1/**`; serve with `{ base: '/apostille' }`.

## J1 — Anonymous lookup → dossier (E2E proven)

Goal: requirements for one authority, conflicts visible.
Spec: `web/e2e/cuj-lookup.spec.mjs`. Budget **2/1**
(arrival, one filter term, one dossier click — irreducible).

Steps: open `/apostille` → type filter → open dossier →
topic tabs render, conflict Notice (not silent contradiction)
when siblings disagree. Assertions: index request fired,
client-side filter narrows, facts request fired on dossier,
tab content visible. Optimization state: minimal already —
no clicks left to remove.

## J2 — Register / login / manage methods (queued)

Goal: account + sign-in that never dead-ends.
Pages: `/apostille/login`, `/apostille/security`.

Plan: passkey register (HAVE a credential stub — see gaps)
→ login → re-register recovery → list methods → link OAuth
→ unlink with proof → last-method 409 copy. Each method
(OAuth ×3, magic link) gets a start→callback→session case
with callbacks mocked at the page level (provider redirect
simulated by navigating to the 302 target with canned
`?code=`). Budget TBD on first spec; target ≤4 actions for
returning passkey login (arrival, enter identifier, two
ceremony confirms are browser UI, not counted).

## J3 — Fill form → PDF (queued)

Goal: completed authority form as PDF.
Pages: `/apostille/forms`, `/forms/{id}`, `/forms/ds-4194`.

Plan: pick authority+form (mock template+prefill) → withheld
fields typed → inline validation errors as copy → submit →
review screen notices → PDF download request → WS
`pdf.ready` refresh (mock socket or skip-live variant).
Budget inputs too: prefill coverage = fewer fills; assert
prefilled fields arrive filled (no retype). DS-4194 pilot:
print-render assertions only (print-only by design).

## J4 — Shipment → labels (queued)

Goal: printed labels for the mail hops.
Page: `/apostille/shipping`.

Plan: create shipment (mock) → purchase (USPS-only
consulate-hop error case as copy) → download URL present →
paste-back tracking → void → defaults round-trip. Budget
TBD; watch the purchase hop: carrier mistakes are the known
footgun, so the error-copy case is mandatory, not optional.

## J5 — Uploads → precheck (blocked: no backend)

`POST /api/v1/uploads` does not exist (UI gets JSON 404);
F8/F9/F10 are spec-only. Plan lands with the backend:
pick → local validation → upload progress → pass/retake
verdict with hints → (F8) rights-grant + submit →
quarantine notice. No E2E against the current shell.

## J6 — Order → track → pay (blocked: shells only)

Order submit, orders list, billing list are unwired shells;
no orders/billing endpoints or pricing spec. Plan lands
with the order API: start order → attach uploads+precheck
→ forms+labels inside the order → track with live updates
→ invoice pay. Until then: no E2E.
