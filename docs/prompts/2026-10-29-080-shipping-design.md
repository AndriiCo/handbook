# Prompt 080 — deep-research shipping design (USPS/FedEx)

- Received (UTC): 2026-10-29T17:20:00Z (approx; workflow invoked prior turn)
- Answered (UTC): 2026-10-29T17:33:44Z

## Prompt (verbatim)

Figure out all the requirements for making shipping using USPS/Fedex
APIs. Have design ready for all aspects - validating address for
shipping, mayne even show OoenStreetMap locator pin and Google Street
View preveiew for return address to double-check. Also autocomplete
for addresses, saved addresses for customers, label formats avaialble
for free printing, and even guiding customer how to fill the envelope
correctly. [truncated in handoff] ... reports (if recieved via
non-trackable mail in mailbox).

(Full brief per workflow args: address validation + OSM/Street View
previews, autocomplete, saved addresses, free label formats, envelope
guidance, checklist UI, outbound + return tracking, customer tracking
numbers, transit stats, non-trackable receipt reports, improvements.)

## Response summary

Deep-research workflow (6 streams + critic + gap follow-up + synthesis)
ran to terminal `completed` with partial parent status; all six stream
briefs, the critic report, and the inline gap-followup evidence were
recovered from session handoff files in this turn (the 18 KB synthesis
text itself spilled and was re-synthesized by the parent from primary
evidence instead of trusted sight-unseen).

Delivered:

1. `docs/specs/shipping-design.md` (new, §§1–17): build-ready design
   for every requested aspect — validation (Rust-first + Addresses
   API v3), native-autofill token fixes, OSM embed + Street View
   link-out, `saved_address` table, free PDF print mapping, envelope
   placement guide, server-persisted checklist, `tracking_events` +
   poller/webhook worker, customer paste-back endpoint, stats
   pipeline, mailbox-receipt flow, verified carrier-client deltas,
   PO-Box/service decisions, quotas/abuse + OPEN-32 queue, P3 scope
   ruling SHP-0 (option A recommended), consolidated SHP-0..16 OPEN
   table, CODE build order.
2. `docs/specs/carrier-apis.md`: §5 OQ-4 (USPS Tracking v3 REST path)
   resolved — `GET
   https://apis.usps.com/tracking/v3/tracking/{trackingNumber}?expand=DETAIL`
   (+ v3r2 batch), verified against the official USPS/api-examples
   README by an independent parent fetch, not just the child claim.
   Same fetch verified the domestic-label shape
   (`X-Payment-Authorization-Token` header + `imageInfo{labelType:
   4X6LABEL, ...}`), proving two repo divergences in
   `crates/labels/src/client.rs` (body token at :418-422,
   `labelSpecification{paperSize}`) — recorded as CODE deltas in
   shipping-design §12, not fixed in this spec turn.
3. Repo citations spot-checked before writing: payment-token lines,
   cancel POST + self-flag comment (:382-386), TEM/FedEx-sandbox
   bases, carrier-apis §1/§5 text, P2 §11 non-goals.

Key open decisions for the author: SHP-0 (P2 slice A vs B — blocks
all CODE), OQ-1/OQ-7 (PII store/retention — block maps, saved
addresses, receipt photos), SHP-11 (LabelSummary price/date ban
revisit — blocks ETA UI), SHP-6 ($0 copy vs no-prices d10), SHP-13
(stats methodology), SHP-7 (sandbox: labelType enum + return
mechanism), FedEx cancel PUT-vs-POST (carrier-apis OQ-3).

## Files changed

- `docs/specs/shipping-design.md` (new)
- `docs/specs/carrier-apis.md` (§1 tracking bullet + §5 OQ-4 resolved)

## Commits

- `0d378d0` — spec: build-ready shipping design (prompt 080)

## Verification performed

- `cargo xtask ci` — green (exit 0; fmt, clippy -D warnings, secrets
  scan, Rust tests, web e2e 3 passed, tailwind tokens OK).
- Independent corroboration: fetched official USPS/api-examples
  README via web_fetch; grepped the saved page for
  `tracking/v3/tracking/{Tracking Number}?expand=DETAIL`,
  `X-Payment-Authorization-Token`, `imageInfo`, `labelType 4X6LABEL`
  — all present, matching child claims before recording OQ-4 as
  resolved.
- `git status` clean except the two intended files; secrets-scan
  clean on staged files (hook).
