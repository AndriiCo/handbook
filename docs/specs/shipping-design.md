# Shipping design: USPS/FedEx APIs, address UX, labels, tracking, stats (build-ready)

- Status: partly built (prompt 080). FedEx quote and buy ship in
  `crates/labels` and `crates/web/src/fedex_live.rs`; the planned
  `xtask/src/fedex.rs` and `crates/labels/src/validate.rs` were never
  created.
- Date: 2026-10-29
- Sources: six parallel research streams (USPS APIs, FedEx APIs,
  address UX, label print, tracking+stats, checklist gaps) + critic
  review, all evidence-backed against repo files; USPS tracking-path
  and label-request shape independently verified against the
  official [USPS/api-examples](https://github.com/USPS/api-examples)
  README by the parent in the same turn.
- Scope: domestic USA first (P2-U1 happy path, pilot Washington
  state). Cross-border/customs stays P3 (P2 §11). Several aspects
  below exceed P2 §11 non-goals — see §15 for the P3 scope decision.

Conventions: EXIST = in repo today (verified path:line); SPEC =
existing spec text; GAP = missing; OPEN = needs a decision
(consolidated in §16).

## 1. Address validation

EXIST: server completeness-only checks
(`crates/labels/src/purchase.rs:80-100`); web shipping From/To blocks
with HTML tokens (partial — §2). No postal-code shape check, no
directional-suffix handling repo-wide, no carrier API validation
wired.

Design (server-side canonical, Rust-first per port pattern):

1. `POST /shipments` preview keeps a pure-Rust pre-check:
   completeness + US ZIP shape (`^\d{5}(-\d{4})?$`) + directional
   suffix normalization (N/S/E/W, Saint/St) before any carrier call.
2. New `crates/labels/src/validate.rs`: USPS Addresses API v3
   Address Standardization (ZIP+4 / delivery point) as the canonical
   normalizer for USPS shipments; FedEx Address Validation API for
   FedEx shipments (REST path still unconfirmed — do not hard-code
   `/address/v1/addresses/resolve`, see carrier-apis §5 OQ-3).
3. Client UX: show a standardization diff ("you entered X → carrier
   standardized Y") with explicit customer accept/reject; never
   silently rewrite. Native-language address line is a separate
   field (see §6), never merged into the validated Latin block.
4. ZIP+4: pass through the Addresses API response shape; payload
   and render layers currently pass postal through raw — wire the
   standardized value into `payload_json` at preview time.

OPEN: SHP-1 (validation strictness: block vs warn on mismatch),
SHP-2 (FedEx validation response/match-code shape — needs spec
sample).

## 2. Address autocomplete

EXIST (partial): `web/src/lib/forms/AddressBlock.svelte:56-115` has
full tokens; shipping From has tokens, To block mostly lacks them;
FROM/TO state+country lack `address-level1`/`country`; no
`section-*` prefixes so from/to collide in browser autofill;
DS-4194 emits no autocomplete; order page has no `tel`.

Design (no third-party JS in P2 — native autofill only):

1. Add missing tokens: To block full set; `address-level1` +
   `country` on both From/To state and country inputs;
   `section-from` / `section-to` prefixes on every token in the two
   blocks so browsers keep them distinct.
2. DS-4194: emit the same token set as AddressBlock.
3. Order/contact page: `tel` (+ `tel-national` where appropriate).
4. No Google Places / autocomplete widget until P3 (unpriced, needs
   PII review); native autofill + server validation (§1) is the P2
   story.

No open decisions; pure CODE work once §15 scope is confirmed.

## 3. Map previews (OSM pin + Street View)

EXIST: none. No geocode code, no CSP `frame-src` for map iframes,
no location-permission copy anywhere.

Design:

1. Return-address confirmation card: OSM embed iframe pinned at the
   standardized address (server-side geocode preferred — see OPEN;
   client-side Nominatim avoids sending customer addresses from our
   servers but is rate-limited and needs a policy read).
2. Google Street View: link-out (`https://www.google.com/maps?...`)
   in P2, not an embed (no API key / billing coupling, no extra PII
   flow to Google on page load). Embed becomes a P3 option with a
   priced Places/API budget.
3. CSP: add `frame-src https://www.openstreetmap.org
   https://*.openstreetmap.org` (embed only if §15 approves).
4. Consent + retention: explicit opt-in copy before any map render
   ("show my address on a map"); GPS/photo fall under PII OQ-1
   placement and OQ-7 retention — no implementation until those are
   decided (reuse F9 SAM-masking precedent before any public render).

OPEN: SHP-3 (server vs client geocode — Nominatim policy read +
PII), SHP-4 (Street View embed vs link-out), blocked-by OQ-1/OQ-7.

## 4. Saved addresses

EXIST: none — only `profile_label_defaults` (formats, not
addresses). No address-book table; CRUD unspecced.

Design:

1. New table `saved_address` (placement per PII OQ-1): id, owner
   account id, label ("Mom's house"), full postal block + country,
   validated flag + validated-at, `is_default_from` /
   `is_default_to`, created/updated, soft delete.
2. CRUD endpoints under the customer surface; checkout offers
   "save this address" after a successful validation (§1); defaults
   prefill From/To.
3. Never prefill across accounts; staff view shows masked values
   only (F9 precedent).

OPEN: blocked on OQ-1 (store placement) and OQ-7 (retention); SHP-5
(max saved addresses per account — propose 10).

## 5. Label formats + free printing

EXIST: `crates/labels/src/format.rs` (Letter default, 4×6, A4),
`printable.rs` (Option A null-tracking printable, Option B manual
paste-back), `display.rs:18-37` (LabelSummary price/date ban),
`purchase.rs`, `web/src/p2_labels.rs` routes. P2 needs PDF —
supported by both carriers.

Design:

1. USPS request shape (verified against official USPS/api-examples
   README, domestic-label example): `POST
   {base}/labels/v3/label` with `X-Payment-Authorization-Token`
   header + `Authorization: Bearer`, body `imageInfo{imageType:
   "PDF", labelType: "4X6LABEL", receiptOption, suppressPostage,
   suppressMailDate, returnLabel}` + `toAddress`/`fromAddress` +
   `packageDescription{mailClass, rateIndicator, weightUOM, weight,
   dimensionsUOM, processingCategory, mailingDate, extraServices,
   ...}`. `labelType: "4X6LABEL"` confirmed; a second official
   example shows `"6X4LABEL"` — confirm the enum in sandbox.
   **Repo divergence (CODE must fix):** `client.rs:418-422` sends
   `paymentAuthorizationToken` in the body and
   `labelSpecification{paperSize}` — both contradict the official
   shape (see §12).
2. FedEx: `labelSpecification` imageType PDF per Ship docs; certify
   the exact image type + printer (cert coupling, carrier-apis §4.5).
3. Free-print UX: PDF letter default; 4×6 for thermal; "print both
   PDFs" (outbound + return) stays the happy path.
4. Suppression flags: expose `suppressPostage` / `suppressMailDate`
   as staff options, default false.

OPEN: SHP-6 ("$0.00" printable copy vs no-prices rule README d10 —
even $0 needs author signoff); SHP-7 (labelType enum + return
mechanism confirmed in sandbox: `POST /labels/v3/returnlabel` vs
`imageInfo.returnLabel` flag — both sourced, undecided).

## 6. Envelope guidance (hand-addressing + native language)

EXIST: none. `PostalAddress` is EN-only with no native-language
field; placement/script/carrier-acceptance undecided; no spec/UI
home for a placement guide.

Design:

1. Non-trackable path (§11) gets a hand-addressing placement guide:
   sender block top-left, recipient center, stamp/postage top-right,
   delivery-point barcode clear zone — as a printable one-pager next
   to Option A.
2. Native-language line: add an optional field rendered *below* the
   Latin block (never replacing it — carrier acceptance is Latin).
   Script allowlist + placement decided in SHP-8.
3. Vietnamese diacritics and CJK placement are P3 unless the demo
   slice (OQ-8) needs them.

OPEN: SHP-8 (native-language scope: field + script allowlist +
carrier acceptance + spec/UI home).

## 7. Checklist UI

EXIST: shipping 3-step sections (`(customer)/shipping/+page.svelte`
h2 "1.Shipment/2.Purchase/3.Labels", gated by shipmentId null) with
NO checklist, progress bar, or aria-current. Best in-repo patterns
to reuse: admin config wizard (`(admin)/config/+page.svelte:344-360`
nav+ol, aria-current=step, per-step done/total, Review with
missing-key jump links); mock order stepper (aria progress, mock
only); precheck live-status pill (aria-live); Notice component for
banners; i18n infra en/es/fr (header strings only).

Design:

1. Server-persisted checklist: reuse the PATCH draft-autosave
   pattern (P2 §9 L321) with per-step completion rows; render native
   checkboxes in fieldset/legend, aria-current step,
   reduced-motion-aware.
2. Steps mirror P2-U1 (authority → fill → sign → review → ship →
   confirm) plus P2-U4 "confirm flagged field" items for conflicted
   prefill; label lifecycle states (draft|labels_pending|ready|
   voided) map to ship-step states.
3. Journeys backlog (from user-journeys.md, none implemented):
   hop-labeled per-step labels (J6), back-planner (J3/J7), QR/scan +
   shareable-link checklist (J9), ES-first checklist + ES mailing
   instructions (J8 — needs ES string builds).
4. Orders page (`(customer)/orders/+page.svelte`) becomes the
   progress-report home once the orders endpoint exists; until then
   keep the honest empty state.
5. Offline: localStorage checklist draft + queued "mark received"
   with sync (no PWA/offline story exists today; needed for J9
   mobile-only + mailbox-receipt flows).

OPEN: SHP-9 (which journey items are in the demo slice vs P3 —
QR/share and ES-first are the expensive ones).

## 8. Tracking: outbound + return

EXIST: static `tracking_url` links only; `shipment` +
`shipping_label` schema with our-side return pairing (shipment row
links two label rows); WS bus with `label.pending/ready/voided` +
`pdf.ready` (P2 §9); realtime topics `shipment:{id}`. No events
table, no poll/subscribe worker.

Design:

1. New `tracking_events` table: id, shipping_label id, carrier,
   tracking no., event code, carrier status text, city/state/country,
   event-at (carrier), received-at, raw JSON (redacted). Poller
   worker (server-side background, same pattern as the instruction
   worker timer) polls active labels on a backoff schedule; webhook
   receivers (USPS Subscriptions API, FedEx webhooks) write the same
   table. Either path emits WS `tracking.update` on
   `shipment:{id}` for label.* parity.
2. USPS Tracking (RESOLVED prompt 080, verified against official
   USPS/api-examples README): `GET
   https://apis.usps.com/tracking/v3/tracking/{trackingNumber}?expand=DETAIL`
   (single; `?expand=SUMMARY` for list views); batch variant `GET
   .../tracking/v3r2/tracking/` with trackingNumber body. TEM base
   swap for test. Push via USPS Subscriptions-Tracking API 3.2;
   receiver URL must be IP-whitelisted — confirm whitelisting steps
   in portal (SHP-10).
3. FedEx Track: `POST https://apis.fedex.com/track/v1/trackingnumbers`
   (known, carrier-apis §2). Return modes: PRINT_RETURN_LABEL fits
   "print both PDFs" best; PENDING (FedEx emails label, expires
   ~1 month, no test-env emails) is the fallback.
4. Return pairing stays our-side: one `shipment` row links outbound
   + return `shipping_label` rows (no USPS-side pairing construct
   exists). USPS return mechanism still undecided (§5 SHP-7).
5. LabelSummary price/date ban (display.rs:18-37) currently blocks
   even ETA/expectation display — needs author revisit (SHP-11):
   the tracking UI cannot show "expected Thursday" until this lifts.

OPEN: SHP-10 (webhook receiver provisioning: IP whitelist steps,
any per-call fees), SHP-11 (LabelSummary ban revisit), plus FedEx
cancel/validation shapes in §12.

## 9. Customer tracking numbers (paste-back)

EXIST: paste-back is staff DEMO-scoped and requires the label PDF
(`manual.rs:80-100`). No customer-facing tracking-entry endpoint;
auth/validation/ownership rules unspecced.

Design:

1. New customer endpoint `POST /shipments/{id}/tracking` accepting
   `{carrier, tracking_number}` for customer-bought postage
   (customer ships with their own label, we track it).
2. Validation: carrier-specific shape check (USPS 20/22/30-digit,
   FedEx 12/15-digit) + live lookup at entry time (USPS v3 §8.2 /
   FedEx track) — reject unknown numbers with a clear error, do not
   store junk.
3. Ownership: number binds to the submitting account's shipment;
   staff can reassign; duplicate numbers across accounts flag for
   staff review (possible fraud signal, not auto-merge).
4. Stored numbers feed the same `tracking_events` + stats pipeline
   (§8, §10).

OPEN: SHP-12 (rate limits on the entry endpoint + duplicate-number
fraud policy).

## 10. Transit stats

EXIST: none. No transit distributions, percentiles, min-n, or
staleness spec; `postage_cost_cents` recorded (L4) but no
aggregation.

Design (pipeline; methodology needs author signoff):

1. Source events: `tracking_events` (carrier scans) + customer
   receipt reports (§11) + customer-entered numbers (§9).
2. Aggregates per (carrier, service, origin ZIP3, dest country):
   n, p50/p90 transit days, on-time %, lost/damaged rate; min-n
   threshold (propose 20) below which the UI shows "not enough data"
   instead of a number; nightly recompute + 24h staleness badge.
3. UI: "usually X–Y days" on the service picker; admin dashboard
   table. No per-customer stats surface (PII minimization).

OPEN: SHP-13 (stats methodology: per-method distributions,
percentiles, min-n, staleness window, who sees what).

## 11. Non-trackable receipt reports

EXIST: none. Printable Option A shows "No tracking yet" with no
expectation/receipt fallback.

Design:

1. Customer flow: "I received it in my mailbox" button on the
   shipment (available after the expectation window, §8.5/§10) →
   optional photo of envelope (F9 masking before any staff render) →
   feeds actuals stats (§10) + closes the shipment loop.
2. Staff flow: manual "mark received" with note for phone/email
   confirmations; queued offline (§7.5).
3. Attestation + retention: who may attest (customer self-report vs
   staff verify) and how long photos/GPS live follow OQ-1/OQ-7 —
   no photo/GPS storage until decided.

OPEN: SHP-14 (receipt attestation owner + UX), blocked-by OQ-1/OQ-7
for photo/GPS.

## 12. Carrier-client code deltas (verified this turn)

Sandbox probes + CODE fixes required before any real buy:

1. USPS payment token: move from body (`client.rs:418-422`) to the
   `X-Payment-Authorization-Token` header (verified official shape,
   §5.1). Update the `pay-synthetic` fixture assertion
   (`client.rs:756`).
2. USPS label shape: replace `labelSpecification{paperSize}` with
   `imageInfo{imageType, labelType, receiptOption, suppressPostage,
   suppressMailDate, returnLabel}` (verified §5.1). Update the
   paperSize assertion (`client.rs:876`).
3. FedEx cancel: repo `POST .../ship/v1/shipments/cancel`
   (`client.rs:386`, self-flagged unverified) vs documented `PUT
   /ship/v1/shipments/cancel` — confirm body + error codes in spec
   before the first real cancel (carrier-apis §5 OQ-3).
4. FedEx prepaid path unwired: SandboxGuard passes only TEM/loopback
   so FedEx fails closed; `LabelCfg` is USPS-only; `FedExClient` is
   never constructed. Wire sandbox allowlist + config + construction
   behind the same guard discipline.
5. Enum/spec samples still needed: `FEDEX_ENVELOPE` /
   `pickupType` / `labelStockType` spellings, Address Validation
   response + match-code shape, rate transit-time field,
   `labelType` 4X6-vs-6X4, return mechanism (SHP-7).
6. Update carrier-apis §3 mapping row "label format" if the sandbox
   contradicts `imageType`-PDF (currently consistent).

## 13. PO-Box routing + service levels

PO-Box rule (undecided): force USPS vs allow FedEx Ground Economy
(FedEx delivers to PO boxes only via Ground Economy / SmartPost
handover; repo sample payload uses a PO Box without a rule).
Propose: PO Box / APO / FPO / DPO addresses force USPS at preview
time with an explanatory note — SHP-15.

Service levels (carrier-apis §5 OQ-7, still open): repo picked
STANDARD_OVERNIGHT / FEDEX_2_DAY without product signoff. Offer set
for the demo slice needs author choice: USPS Ground Advantage /
Priority / Express; FedEx STANDARD vs PRIORITY overnight, 2Day vs
2Day AM, Ground vs Ground Economy. Cost + transit implications feed
§10 stats later.

## 14. Ops, abuse, staff queue

EXIST: no rate-limit/quota/CAPTCHA on label routes (only mail-client
backoff in crates/mail); `postage_cost_cents` recorded but no
free-label quota; no staff manual-queue list route (OPEN-32).

Design:

1. Per-account caps: free labels/day (propose 5, SHP-16), tracking
   lookups/min on customer endpoints, address validations/min;
   staff-manual throttle rules for the paste-back queue.
2. Staff manual queue: OPEN-32 list route (pending paste-backs,
   duplicate-number flags from §9.3, receipt attestations from
   §11.2) — prerequisite for (B) at any volume.
3. Abuse signals: duplicate tracking numbers across accounts,
   rapid label void/reissue cycles, mismatched From addresses —
   flag for staff, never auto-ban.

OPEN: SHP-16 (quota numbers + throttle rules), OPEN-32 (manual-queue
route scope).

## 15. P3 scope decision (blocks CODE)

P2 §11 non-goals explicitly exclude address-validation APIs,
rate-shop, void/refund automation, and checkout. This design's §§1,
3, 4, 8 (webhooks), 9, 10, 11 need a scope ruling before CODE:

- Option A (recommended): P2 demo slice = §§2, 5 (PDF print), 6,
  7 (core checklist, no QR/share/ES), 8 (poll-based tracking +
  customer entry), 12 (client fixes), 13 (PO-Box + service picks),
  14 (quotas + OPEN-32). Everything validation/geocode/webhook/
  stats/receipt-photo moves to P3.
- Option B: pull validation (§1) + customer tracking entry (§9)
  into P2 as well (demo needs live tracking of customer-bought
  postage).

SHP-0 (author): pick A or B; unpicked sections stay P3-gated and
CODE must not build them.

## 16. Consolidated OPEN items

Pre-existing (still blocking): P2 OQ-1 (PII store), OQ-3 (labels
buy path — staff-manual recommended), OQ-7 (PII retention), OQ-8
(demo slice), OPEN-32 (manual queue); carrier-apis §5 OQ-1/2/3/5/6/7
(OQ-4 resolved this turn — §8.2); user-journeys B1-B3/C1 pricing.

| ID | Question | Owner | Blocks |
|---|---|---|---|
| SHP-0 | P2 slice A vs B (§15) | author | all CODE |
| SHP-1 | Validation strictness: block vs warn | author | §1 |
| SHP-2 | FedEx validation response shape (spec sample) | research | §1 FedEx |
| SHP-3 | Server vs client geocode | author (+OQ-1) | §3 |
| SHP-4 | Street View embed vs link-out | author | §3 |
| SHP-5 | Max saved addresses (propose 10) | author | §4 |
| SHP-6 | $0-printable copy vs no-prices d10 | author | §5 |
| SHP-7 | labelType enum + return mechanism (sandbox) | sandbox probe | §5, §8 |
| SHP-8 | Native-language scope | author | §6 |
| SHP-9 | Journey items in demo vs P3 | author | §7 |
| SHP-10 | Webhook provisioning (IP whitelist, fees) | portal/ops | §8 push |
| SHP-11 | LabelSummary price/date ban revisit | author | §8 ETA UI |
| SHP-12 | Paste-back rate limits + fraud policy | author | §9 |
| SHP-13 | Stats methodology | author | §10 |
| SHP-14 | Receipt attestation owner + UX | author | §11 |
| SHP-15 | PO-Box routing rule (propose force-USPS) | author | §13 |
| SHP-16 | Quota numbers + throttle rules | author | §14 |

## 17. Suggested CODE build order (after SHP-0)

1. §12 client fixes (payment header, imageInfo, fixtures) + §13
   PO-Box rule + service enum signoff — unblocks every later step.
2. §2 autocomplete tokens (small, isolated).
3. §7 core checklist + §8 poll-based tracking + `tracking_events`
   + WS `tracking.update`.
4. §9 customer tracking entry + §14 quotas + OPEN-32 queue.
5. §5 free-print polish + §6 envelope guide.
6. P3-gated: §§1, 3, 4, 8-push, 10, 11-photo (each needs its SHP +
   OQ-1/OQ-7 first).

## 18. Live FedEx (SA-19, implemented in-branch)

Live FedEx buys (OAuth, rate quote, outbound + return label, void)
behind the `labels.live` master switch. Code: `crates/labels/src/live.rs`
(gate checks, OAuth/rate shapes, idempotent buy-pair + void, sandbox
fixtures, packet cover renderer); proof: recorded synthetic fixtures in `crates/labels/fixtures/fedex/`
(the planned `cargo xtask fedex` was never built); staff UI:
`web/src/routes/(admin)/labels/+page.svelte` (admin build only).

1. Gates (checked in this order, fail closed): `labels.live` on,
   staff session, destination ZIP on `live_ship_to_allow`, day count
   under `live_daily_cap` (default 2), quote under `live_max_usd`
   (default USD 60). Quote is read-only and uncapped so staff can see
   the over-cap number; the buy path enforces the cost cap.
2. Idempotency: ledger key `order_id:leg` (`fedex_live_ledger`,
   `CREATE TABLE IF NOT EXISTS` only — see `live.rs` `LEDGER_DDL`).
   Re-buying a bought leg replays the stored label without touching
   the carrier and without consuming day cap. Voids keep the leg
   closed (re-issue under a new order id). Every attempt appends a
   `fedex_live_audit` row (`AUDIT_DDL`); audit rows are append-only.
3. Sandbox discipline: `SandboxTransport` replays recorded synthetic
   fixtures (USD 12.50 rate, `SYNTHETIC…` tracking, `%PDF-` stub
   bytes). No live credentials anywhere in repo, fixtures, or logs;
   OAuth/client-secret/account values arrive per call from
   secure-config at runtime.
4. Packet: `render_packet_cover` renders the prefilled packet form
   (shipper/recipient blocks, both tracking numbers, staff sign-off;
   deterministic, no price by construction) — print to PDF from the
   browser alongside the two label PDFs.
5. Resolves §12.3: FedEx cancel stays `POST …/ship/v1/shipments/cancel`
   (the `PUT` variant from public docs is unverified; confirm against
   the sandbox before the first real cancel).

## 19. Config keys for live FedEx (builder: register these)

| Key | Type | Hint |
|---|---|---|
| `labels.live` | bool | Master switch, default false. |
| `labels.live_daily_cap` | int | Max live labels per UTC day, default 2. |
| `labels.live_max_usd` | number | Max quoted cost per label in USD, default 60. |
| `labels.live_ship_to_allow` | text | CSV of allowed destination ZIPs (exact match; empty allows nothing). |
| `labels.fedex_client_id` | secret | FedEx OAuth client id (write-only, Set/Unset display). |
| `labels.fedex_client_secret` | secret | FedEx OAuth client secret (write-only). |
| `labels.fedex_account` | text | FedEx account number billed for labels (server-side only). |

`xtask/src/main.rs` wiring (builder applies — SA-19 does not edit it):

```rust,ignore
mod fedex; // alongside the other `mod` lines at the top
// inside the match in `main()`:
Some("fedex") => fedex::run(&args[1..]),
```

Backend routes for the staff UI (builder wires under the staff API —
SA-19 ships the page only): `GET /fedex-live/caps`, `POST
/fedex-live/quote`, `POST /fedex-live/purchase`, `POST /fedex-live/void`.


