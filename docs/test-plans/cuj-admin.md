# Admin CUJs 1–6

Admin SPA (`web/build-admin`, base `/`). Mock API at
`**/apostille/api/v1/**`. Staff pages gate on session:
specs mock `GET .../admin/me` + the page endpoints (the
passkey ceremony itself is covered by handler tests; full
ceremony E2E waits on a virtual authenticator — see gaps).

## CUJ-1 — First boot → enrolled (configure slice proven)

Goal: bare server configured, first staffer in, window burned.
Spec: `web/e2e/cuj-wizard.spec.mjs` (configure slice).
Budget **3/3 per step** (arrival, key paste+submit amortized,
fills, one Next with auto-save).

Steps: shell mint (by design, untested in browser) → key
entry → steps 1–8 → review → enrol `?setup=1` (manual:
authenticator) → burn verified (`setup_required=false`,
old key 403s — handler-tested). History: per-field Saves
cost 5 actions/step; auto-save on navigation (prompt 063)
cut it to 3, and fixed unset-key edits never enabling Save
(the left-join: all rows live in `$state`). Full 9-step +
enrol E2E stays manual until virtual-authenticator support.

## CUJ-2 — Proposed → accepted fact (queued)

Goal: one reviewed fact reaching customers.
Pages: `/review` (primary),  `/triage` (legacy).

Plan: queue list (mock enriched item: diff, citations,
`reasoning_untrusted`, blast radius, provenance) → accept /
accept-with-edit / reject-with-suppress (+ rule fields) →
card collapses → concurrent-decide 409 copy → fact visible
in dossier mock. Triage path: one legacy accept/reject case
until the page is removed. Budget TBD; target ≤3 actions
per decision (open, decide, confirm-none — no confirm
dialog by design, undo via re-proposal).

## CUJ-3 — Watch → capture → draft (queued)

Goal: source change detected and drafted.
Page: `/workers` (+ shell `xtask analyze`).

Plan: status/depths render → sweep (mock ≤10 items) →
one-off fetch → retry failed capture → new proposals appear
in review queue mock. The analysis run itself stays shell
(no browser trigger by design); E2E asserts handoffs, not
the analyzer. Suppression-skip visibility asserted via the
suppressions-log table mock.

## CUJ-4 — Call → transcript → quote (queued)

Goal: phone-verified draft from a live call.
Pages: `/calls`, `/calls/[id]`.

Plan: dial form (E.164, authority, questions, consent —
refusal 422 case mandatory) → detail: checklist, transcript
poll, record/hangup controls → segment multi-select → quote
(topic+statement) → proposed fact → review handoff. No live
Telnyx: stub-transport fixtures per voice-telnyx §11.
Playback/sweep/fix-annotate E2E lands with §11 build-out.

## CUJ-5 — Reject → suppressed (queued)

Goal: junk never re-triaged. Page: `/review` rules panel.

Plan: reject-with-suppress (scope+topic) → standalone rule
create → skip row in suppressions log (mock) → revoke
over-broad rule → claim re-triageable. Smallest CUJ: target
≤2 actions for the inline path (decide carries the rule).

## CUJ-6 — Secret rotation (queued)

Goal: re-key a provider without breaking its lane.
Page:  `/config` normal mode.

Plan: locate key → step-up-expired re-login redirect →
set-new secret (write-only: Set flips, value never echoes)
→ audited `config_set` → dependent-lane check mocked green.
Target ≤3 actions (open, set, verify) plus the re-login
when the 600 s window lapses.
