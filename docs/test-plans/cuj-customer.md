# Customer CUJs J1–J6 and D1–D11

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

## D1–D11 — the apostille guide page (R2)

Page: `/apostille/authority/usa/washington` (every authority page
shares it). The guide answers the customer's questions in the order
they ask them; each D-journey also serves the stories J1–J9 in
[`docs/specs/user-journeys.md`](../specs/user-journeys.md). In the
table, "story J3" names one of those stories, not this file's CUJ J3;
the last row, J1, is this file's CUJ J1.

Spec: `web/e2e/cuj-dossier.spec.mjs` (lands with the page), one
scripted check per row, with the `helpers/cuj.mjs` action/input
budgets, `page.clock`, and a WA-shaped fixture that uses the canonical
fixture boxes 40220/40234, never values copied from production facts.
The digits are real Washington boxes (40234 is the division's, 40220
the agency's main office; see resolution 9 in `docs/specs/README.md`).
Data coverage for the same page is measured separately by
`web/scripts/dossier-metrics.mjs` against
[`wa-checklist.json`](wa-checklist.json).

Source: the approved plan, section 1, with the section 12 changes
folded in (print budget, plain-text listed site, provider line on the
label card, no "open now" during a notice).

| ID | Customer question (story) | Done when | Budget | Resolved by |
|---|---|---|---|---|
| D1 | Is this the right office? (story J3 federal misroute, story J5) | Issuer, scope (WA-issued or WA-notarized), "federal documents go to the US Department of State", and the "Not valid in the U.S." warning are all visible. The page says it is an unofficial guide, not a government website. | 0 actions | lede, step 1, warning text |
| D2 | What will N documents cost? (stories J1, J4) | 3 expedited = $145.00. 12 same-day = $430.00, only once the tier fact is accepted. 25 = "ask the office". The payee copies exactly. Values show verbatim. | 1 input + 1 click | fee table, a tier calculator that refuses unprinted tiers, Copy payee |
| D3 | Where exactly do I send it? (story J1 "which PO Box is current") | The USPS branch never shows a street. The courier branch never shows a PO box. A disputed box shows every variant plus a safe workaround (call the office first, then the courier route), with no Copy and no label. After a published staff decision backed by the office's answer (a call, an email or a visit), the box says "Our recommendation (staff, …): use …", marks that row and gives it alone Copy; labels stay off. The glance and the route cards follow the same street and PO-box rules. | 0 actions; 0 wrong-address outputs | send branches, ConflictBox, orders gating, JSON-LD omission |
| D4 | Can I walk in today? (story J7 urgent) | At Mon 10:00 PT: Tumwater cut-off 3:30 p.m.; Thursday: Cheney closed; Nov 11: closed; Nov 13: cut-off 1:30 p.m. Calculated only from an accepted OfficeSetV1 with a confirmed timezone; otherwise the printed rule verbatim. "Open now" and calculated cut-offs are hidden while a notice is active. | 0 actions | office cards, closures, overrides |
| D5 | How long will it take? | Range per route, verbatim with the office's hedge ("Typically…") | 0 actions | glance, step 6 |
| D6 | Get the form (story J9, no printer) | Official form link, labelled "Download (PDF)" only when the cited URL is a PDF, in step 3 and in the glance's Form row; when the office's pages word the form differently, the link comes from the worded values' citations. "Fill it in online — free" appears only when a fillable form map is accepted (`/plan` `forms[].fill_supported`), and the filled PDF needs no order. | ≤1 action to the PDF; 0 dead ends | step 3, order Form step "Download the filled form" |
| D7 | Where does this come from? (trust; judges) | The button or Alt/⌥+C turns on `[n]` markers. Hover, focus or tap shows the quote, URL, captured and approved dates, and evidence kind. The keyboard can reach the endnote and come back. | ≤2 actions | citations mode |
| D8 | Print it | Off: 0 markers and 0 endnotes. On: every marker has an endnote with a URL or contact and dates. All critical values print, in light ink; no record ids in the print header. | ≤4 Letter pages off, ≤8 on | print CSS, PrintHeader |
| D9 | Is the office running? (11/02 outage showcase) | An active notice sits directly under the h1. A resolved one (`notice__<id>` paired with `restored__<id>`) appears only as a status line (7 days) and in Updates. "No open service notice in the office's notice log (last read …)" shows only when a paired notice from that log exists. | 0 actions | `notice`/`restored` slots |
| D10 | Buy a prepaid label (story J1, tier b) | The free route is complete first. The label card names its provider ("Paid service by the Apostille Handbook, not the Secretary of State") and its live price; no paid offer while the fee slot isn't `ok`. A disabled offer says why and points back to the free route. | ≤1 action | courier label card |
| D11 | An office we haven't checked (508 of 510 pages) | No values shown. Exits: the listed official site as plain text, "not yet checked by us", and "Browse offices we've checked". | 0 dead ends | EmptyState |
| J1 | Existing lookup (this file's CUJ J1, `cuj-lookup.spec.mjs`) | Unchanged: the region "Fees" shows the unkeyed fixture fact | `{actions:2, inputs:1}` | "Pay the fees" is the only region whose name contains "fees" |
