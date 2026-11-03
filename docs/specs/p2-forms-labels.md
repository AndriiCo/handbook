# P2 Requirements: In-Browser Forms + Shipping Labels (draft for review)

- Status: draft (prompt 007, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child
  session log).
- P1 = evidence store + call console (per `docs/design.md`,
  `docs/web-stack.md`). P2 = customer-facing order path below.
- Stack constraints inherited: Rust/axum 0.8 API + SvelteKit SPA,
  WS-first realtime (`{type, …}` envelopes over
  `tokio::sync::broadcast`), sub-path serving under `/apostille`,
  pure-Rust deps only (macOS local / Ubuntu 26.04 prod parity), one
  binary + `store/`, secrets by env (`STORE_PATH`, `BASE_PATH`, …).
- P2 scope (hard): **in-browser apostille-form fill-in with
  server-rendered PDF (NO Adobe/download-blank-form path)** +
  **ready-to-use FedEx/USPS shipping labels ONLY**. Physical
  door-kits (envelopes delivered to door) and full-service tiers are
  P3/ops — not spec'd here.

## 1. Goals and scope

- P2-G1: A customer completes an apostille request fully in the
  browser: pick authority + destination use, fill the authority's
  request form guided, get a filled print-ready PDF, and get outbound
  + return shipping labels.
- P2-G2: Every prefilled value (authority address, fee, form edition)
  traces to an accepted `fact` + citing `capture` (design.md §1);
  stale template or conflicting facts block submission with a visible
  reason, never silent wrong output.
- P2-G3: No customer PII in the code repo, ever (design.md §0 extends
  to P2). PII storage location is Open Question OQ-1.
- Out of scope: see §11.

## 2. User flows

- P2-U1 — Happy path (domestic USA first): (1) customer picks
  authority (pilot: Washington state) and document destination
  country; (2) SPA fetches active form template + prefilled authority
  facts (mailing address, fee, current form edition); (3) customer
  fills fields in-browser with inline validation; (4) signature step
  per §6; (5) review screen shows field summary + replica-acceptance
  notice (§5) + shipping summary; (6) submit → server validates,
  persists submission, renders filled PDF; (7) shipping step: customer
  enters/confirms from/to addresses, picks FedEx or USPS service,
  gets ready-to-use labels (purchase model per §7); (8) confirmation
  page + WS updates: `pdf.ready`, `label.ready`, tracking numbers.
  Customer prints the filled form, signs in ink if required, mails
  with labels.
- P2-U2 — Template stale/superseded: if the authority revised its
  form after the customer started (template `superseded`), submission
  is refused with "form updated on <date>, please review <changed
  fields>"; entered values carry over field-by-field where keys
  match; audit row records the refusal.
- P2-U3 — Replica not accepted: if the authority requires its
  original printed form (`accepts_replica = no`), the flow still fills
  in-browser but the review screen and the PDF header state "print
  single-sided and mail this original printout — photocopies not
  accepted per <citation>"; no alternate procurement path in P2
  (door-kit is P3).
- P2-U4 — Conflicting facts: if two `accepted` facts conflict on a
  prefilled topic (address/fee, design.md §7 query), prefill is
  withheld for that field, shown as "needs confirmation — we found
  two addresses", and a `task(kind=correction)` is opened/linked;
  customer may type a value manually, flagged
  `customer_override=true`.
- P2-U5 — Correction loop: customer or staff reports a wrong prefill
  → `capture(kind=comment)` → `task` → fact re-validation (P1
  machinery); affected active templates flip to `needs_review` until
  staff re-approves.

## 3. Per-authority form-template model

- P2-F1 — One template row per authority form edition:
  `form_template(id, authority_id, code, title, version,
  authority_revision TEXT, status, schema_json, layout_json,
  accepts_replica, signature_mode, source_capture_id, fact_ids_json,
  created_by, created_at, supersedes_id)`.
  - `code`: stable slug, e.g. `wa-apostille-request`.
  - `version`: our internal monotonic int (1, 2, 3…).
    `authority_revision`: the authority's own edition mark ("Rev
    06/2025", "Form SOS-001 10/2024", or `unmarked+first_seen_date`).
    Never conflate the two.
  - `status`: `draft | active | needs_review | superseded | retired`.
    Exactly one `active` per (`authority_id`, `code`).
  - `source_capture_id`: the ingested official blank form (design.md
    `capture(kind=file/scan)`). `fact_ids_json`: facts backing
    prefilled values.
- P2-F2 — `schema_json`: ordered field list. Field: `{key, label,
  help, type, required, max_len, pattern, options[], prefill_topic,
  pdf: {page, x, y, w, h, font_size, multiline, checkbox_size}}`.
  Types: `text | textarea | email | tel | date | number | money |
  select | radio | checkbox | address_block`. `address_block` expands
  to name/street/city/state/zip sub-keys. `prefill_topic` references
  `fact.topic` (`address|fee|form|…`); null = customer-entered only.
- P2-F3 — Validation runs twice from one rule source: SPA (instant, TS
  port) and server (authoritative, Rust `serde` + `validator`-style
  checks: required, max_len, regex, enums, date/money parse, address
  completeness). Server rejection returns per-field `{key, code,
  message}`; SPA renders inline. No submission persists while
  invalid.
- P2-F4 — Versions vs authority revisions: new authority edition →
  new row (`version+1`, `supersedes_id` → old, old → `superseded`),
  never in-place edit of an `active` template except typo fixes to
  `label/help` (which bump `version` too but keep
  `authority_revision` and auto-carry submissions). Changed-key diff
  (`added/removed/renamed`) computed at publish and shown to in-flight
  users (U2). All publishes append `audit` rows.
- P2-F5 — Template home (recommendation + OQ): templates are
  **content, not code** → live in the **content repo**
  (`forms/<authority-slug>/<code>/v<n>.json`: schema + layout +
  `accepts_replica` + `signature_mode` + source citation), with the
  code repo holding only the JSON-schema types, validator, renderer,
  and migrations. `build`/`audit` rows record both commits as today.
  Rationale: matches design.md §0/§6 two-repo split; lets non-Rust
  staff update forms without a code deploy. Blocked on OQ-2
  (content-repo location); interim fallback: `store/forms/`
  (gitignored, seeded by `xtask dev-seed` with synthetic templates).
- P2-F6 — SLA fields: each template records `last_verified_at` +
  `verify_cadence_days` (default 90); expiry flips `active →
  needs_review` and triggers a `task(kind=validate_fact)`.

## 4. Filled-form rendering (pure-Rust server-side PDF)

- P2-R1 — Server renders the filled PDF; the SPA never builds PDFs
  and there is **no blank-form download** path. `POST
  /form-submissions` → validate → store → render → `GET …/pdf`
  returns `application/pdf` with `Content-Disposition: attachment;
  filename="<code>-v<n>-<submission>.pdf"`.
- P2-R2 — Recommendation: **`printpdf`** (pure Rust, satisfies
  web-stack §1 parity rule; fixed-layout absolute-position drawing
  fits government replica forms; small dep, no system libs, compiles
  on macOS + Ubuntu identically). Parent verified: docs.rs printpdf
  0.12.8 (Sept 2026), MIT, normal deps all Rust (lopdf, flate2, …).
  Layout comes from each field's `pdf` rect (§3). Fonts: bundle one
  metric-compatible serif/sans pair (e.g. Helvetica built-in) — no
  external font fetch at runtime.
- P2-R3 — Alternatives rejected: `typst` (excellent layout but a
  heavyweight full typesetter for fixed government forms; larger
  binary, slower builds); `lopdf`-overlay onto the official PDF
  (would require bundling/redistributing official PDFs — licensing
  risk — and breaks when the authority re-exports); client-side
  jsPDF (violates "server is source of truth", duplicates layout
  logic, breaks audit hash). `printpdf` + our own rect layout is the
  smallest thing that works.
- P2-R4 — Replica fidelity: clean-room redraw (our boxes/lines/text
  at measured coordinates), not a scan trace. Header block on page 1:
  template code + version + authority revision + "generated <utc>
  from accepted facts as of build #<n>" + replica notice when
  applicable. Every PDF's sha256 stored on the submission row; bytes
  in file store `blob/` (dedupe free), served via `ServeFile`
  semantics (ETag/Range per web-stack §2).
- P2-R5 — Determinism: same submission → byte-identical PDF (fixed
  timestamps in PDF metadata = generation time recorded separately,
  not embedded variably). Required so `facts_hash`-style audit stays
  meaningful.
- P2-R6 — Print roadmap (decided prompt 009): v1 ships as specced
  above (server-rendered PDF via printpdf is the submission
  artifact). The ultimate goal is **HTML-print-first**: form data is
  collected in a validated web UI and printed from a nicely styled
  HTML/CSS page at the correct page size (`@page` rules, print
  stylesheet per [site-theme](../site-theme.md)), without touching
  PDFs at all; a PDF file becomes an **on-request option**, never a
  mandatory step. Keep the template `pdf` rects and the renderer
  behind a trait so the HTML print view and the PDF render share
  one field layout source; add `GET /form-submissions/:id/print`
  (HTML) alongside `…/pdf` when the roadmap lands. PDF-on-request
  keeps the R4 header block and sha256 audit semantics.

## 5. Replica-PDF acceptability flag

- P2-A1 — Per-template `accepts_replica`: `yes | no | unknown` +
  `accepts_replica_capture_id` (evidence: authority FAQ "print this
  form", rejection report, or staff call transcript). Default
  `unknown` for new templates.
- P2-A2 — Behavior: `yes` → normal flow. `no` → U3 notice
  (in-browser fill still allowed; printout IS the submission
  artifact; no photocopy warning). `unknown` → banner "we haven't
  confirmed <authority> accepts printed forms — we show the official
  source and a prefilled review; confirm by <link>" + auto-open
  `task(kind=validate_fact, topic=form)`; submission allowed but
  flagged `replica_unconfirmed=true`.
- P2-A3 — Authority-level default: `authority` row gains
  `replica_default (yes|no|unknown)` used only when a template value
  is missing; template value always wins. Seed: `unknown` everywhere;
  Washington pilot must reach `yes`/`no` with citation before P2
  demo.

## 6. Signature model

- P2-S1 — Options: (a) **wet-ink line**: PDF renders an empty
  signature + date line; customer prints and signs with a pen. (b)
  **canvas-drawn**: customer draws on `<canvas>` in the SPA; PNG
  stored and embedded into the PDF at the signature rect.
- P2-S2 — Recommendation: **(a) wet-ink line as the P2 default and
  only required mode**. Reasons: universally accepted by authorities
  (zero rejection risk from e-signature policy); no drawn-signature
  PII retention problem; ~10 lines of renderer code vs canvas capture
  + image embed + tamper story. Template `signature_mode`: `wet_ink
  (default) | drawn_optional | drawn_required`. P2 implements
  `wet_ink` fully; `drawn_optional` embeds the PNG only where
  `accepts_replica=yes` AND staff recorded drawn-signature acceptance
  (a second flag `accepts_drawn_signature`, default `no`).
  `drawn_required` is not implemented in P2 (no authority is expected
  to require it).
- P2-S3 — Drawn-signature rules (if enabled per template): canvas ≥
  600×200, stored as PNG blob + sha256, embedded at fixed rect, PDF
  notes "signed electronically by <typed name> on <date>"; retention
  follows PII policy (OQ-1); never reused across submissions without
  explicit re-draw.

## 7. Shipping labels (ready-to-use ONLY)

- P2-L1 — P2 ships an outbound label (customer → authority) + a
  return label (authority → customer) as print-ready PDFs with
  tracking numbers surfaced in the SPA and over WS. No envelopes, no
  packing, no door delivery orchestration (P3).
- P2-L2 — Data needs (both carriers): sender/recipient name, street,
  city, state/province, postal, country, phone; parcel weight + dims
  (default letter/flat envelope preset + override); service code;
  return-label pairing id; tracking number (post-purchase). FedEx
  extras: account number, service type
  (`FEDEX_OVERNIGHT|FEDEX_2_DAY|FEDEX_GROUND`), label format (PDF),
  pickup type (drop-off default — no pickup scheduling in P2). USPS
  extras: service (`PRIORITY|EXPRESS|GROUND_ADVANTAGE`), cubic/weight
  pricing inputs, return service via prepaid return label. P2 scope:
  domestic USA services first; Canada/Mexico cross-border (customs
  forms) explicitly P3.
- P2-L3 — Label-purchase options: (A) **carrier-API server buy** —
  axum calls FedEx Ship API + USPS eVS directly (needs carrier
  accounts, OAuth/API keys, billing instruments, address-validation +
  rate-shop + buy + void flows, sandbox certification). (B)
  **staff-manual from payloads** — server generates a ready-to-buy
  payload per order (prevalidated addresses + service + dims as
  JSON/CSV + a one-screen staff "buy this label" view) and staff buys
  in the carrier portal, then pastes back tracking + uploads label
  PDFs; WS notifies the customer.
- P2-L4 — Recommendation: **(B) staff-manual for P2**. Reasons: zero
  carrier-account/approval lead time inside a 10-day build; no
  billing/PCI-adjacent card handling in our binary; no
  rate/void/refund edge cases; the customer-visible artifact (ready
  label PDF + tracking) is identical. Build (A) as P3 once volume
  justifies it; keep the payload schema carrier-neutral so (A) reuses
  it. **Author decision required (OQ-3)**: Andrii confirms
  manual-vs-API, who holds carrier accounts, and who pays (customer
  prepaid at checkout vs absorbed) — P2 implements no payment flow;
  it records `postage_cost_cents` + `paid_by` fields and leaves
  charging to existing Andrii Co. booking/billing.
- P2-L5 — Label storage: label PDFs as blobs + `shipping_label` rows
  (§8); tracking numbers indexed; void/reissue is staff-only via
  audit-logged state transitions (`issued → voided → reissued`).

## 8. Schema deltas to design.md §1

Design.md §1 has 13 tables (+1 FTS). P2 adds 4 tables; 1 column
family on `authority`. All timestamps INTEGER unix seconds; `audit`
covers every mutation.

```sql
-- P2-D1: §3 template registry (authoritative copy in content repo; table is the indexed cache).
-- Non-PII: lives in handbook.sqlite regardless of the OQ-1 PII decision.
form_template(id, authority_id, code, title, version, authority_revision,
  status,                       -- draft|active|needs_review|superseded|retired
  schema_json, layout_json,     -- layout may fold into schema pdf rects; split kept for clarity
  accepts_replica,              -- yes|no|unknown
  accepts_replica_capture_id, accepts_drawn_signature,  -- default 'no'
  signature_mode,               -- wet_ink|drawn_optional
  source_capture_id, fact_ids_json, supersedes_id,
  last_verified_at, verify_cadence_days,
  created_by, created_at);
CREATE UNIQUE INDEX idx_ft_active ON form_template(authority_id, code) WHERE status='active';

-- P2-D2: one customer fill-in (PII! → OQ-1 placement)
form_submission(id, form_template_id, authority_id, customer_ref,
  values_json,                  -- keyed by schema key; overrides flagged inline
  customer_override, replica_unconfirmed,
  signature_mode_used, signature_capture_id,  -- drawn PNG capture or NULL
  pdf_capture_id, pdf_sha256, status,         -- draft|submitted|superseded|voided
  created_at, submitted_at);

-- P2-D3: outbound+return pair per submission
shipment(id, submission_id, authority_id, status,  -- draft|labels_pending|ready|voided
  from_json, to_json, parcel_json,       -- addresses + weight/dims preset
  postage_cost_cents, paid_by,           -- recorded, not charged (L4)
  created_at, updated_at);

-- P2-D4: one label PDF + tracking per direction
shipping_label(id, shipment_id, direction,  -- outbound|return
  carrier,                                  -- fedex|usps
  service, tracking_no, label_capture_id, payload_json,
  status,                                   -- pending|issued|voided|reissued
  created_at, issued_at);
CREATE INDEX idx_label_tracking ON shipping_label(tracking_no);

-- P2-D5: authority additions
-- authority.replica_default (yes|no|unknown, default unknown)
-- authority.ship_to_fact_id (preferred mailing-address fact, nullable)
```

- P2-D6 — PII placement (recommendation + OQ-1): put
  `form_submission`, `shipment`, `shipping_label`, and
  drawn-signature/PDF blobs' rows in a **separate `store/p2.sqlite`**
  (same `rusqlite`+bundled, same migration style), cross-referencing
  `handbook.sqlite` ids as opaque integers. Reasons: evidence DB
  stays publishable/shareable for builds; PII DB gets its own
  retention (`xtask pii-sweep`: voided/draft expiry, drawn-signature
  purge), stricter file perms, and independent backup/wipe story.
  Same-file tables are the fallback if two-DB joins prove painful —
  decide in OQ-1.

## 9. Axum endpoint + WS event sketch

Base: `/apostille/api/v1` (JSON; auth = the handbook login stack
per [auth-playbook](../auth-playbook.md), built by the web/auth
stream — P2 adds no auth code of its own).
Static/label-PDF downloads via `tower-http` range/ETag semantics.

```text
GET  /authorities/:slug/forms                 → active templates + replica/signature flags + prefill facts
GET  /form-templates/:id                      → full schema_json + layout version
POST /form-submissions                        → {template_id, values} → 201 {id} | 422 per-field errors
PATCH /form-submissions/:id                   → draft autosave (last-write-wins + audit, web-stack §3)
GET  /form-submissions/:id/pdf                → filled PDF (printpdf render, cached by pdf_sha256)
POST /shipments                               → {submission_id, from, to, parcel, carrier, service} → payload preview
POST /shipments/:id/request-labels            → staff queue entry (model B) → {status: labels_pending}
POST /staff/shipments/:id/labels              → staff attaches {tracking_no, label_pdf} (model B; admin listener only)
GET  /shipments/:id                           → status + tracking + label download URLs
```

WS topics (extend web-stack §3 `authority:{slug}`, `call:{id}` with):

```text
submission:{id}  → form.validated | pdf.ready {sha256, url} | form.superseded {new_version, changed_keys}
shipment:{id}    → label.pending | label.ready {direction, tracking_no, url} | label.voided
authority:{slug} → template.published {code, version} | template.needs_review {code, reason}
```

Envelope `{type, …}` unchanged; presence ("staff viewing this
shipment") falls out of tracked WS connections per web-stack §3.
(Reconciled topic nouns: `submission`/`shipment` win over `order` —
see [README.md](README.md).)

OPEN-32 (owner: author; blocks: Slice D manual-label queue UI): no
staff list route exists for the label queue — `queue_manual` is a
server-side helper, not a handler, and §9 sketches only per-shipment
writes. The ship-run quote-to-draft surface (voice §7) is unaffected.

### Proposed resolution (pending author review — do NOT implement before approval)

**Route:** `GET /apostille/api/v1/shipments` — NOT the
`/staff/shipments` sketch from the OPEN-32 note: no `/staff/...`
prefix exists anywhere in the tree; every staff handler lives under
`/apostille/api/v1/...` on the admin listener.

**Purpose:** staff work queue for manually-bought labels, defaulting
to shipments awaiting staff action (`labels_pending`). Admin listener
only (same merge block as `admin::router`).

**Query params** (all optional; shape follows `list_captures`):

| Param | Meaning | Default |
|---|---|---|
| `status` | `draft \| labels_pending \| ready \| voided` | `labels_pending` |
| `authority` | Authority slug filter | none (all) |
| `limit` | Page size, clamped 1..200 | 50 |
| `before_id` | Keyset cursor (`id < before_id`) | none (first page) |

Fixed newest-first sort; no `sort` param in v1. Out-of-range `limit`
clamps, never rejects (both existing list handlers clamp).

**Response (200):** `{"shipments": [{id, submission_id, authority_id,
status, customer_ref, created_at, updated_at, pending_count,
labels[]}], "count"}` — `labels[]` holds pending-status labels only
(the actionable set); per-row tracking/file summaries omitted (pending
labels have none yet — they ride the per-shipment GET once issued);
`payload_json` is the stored buy payload; `count` is page-local
(matching both existing list handlers). Full addresses do NOT ride the
list. Reads write no audit row.

**Counts:** sibling route `GET
/apostille/api/v1/shipments/counts` →
`{"counts": {"draft": n, "labels_pending": n, "ready": n, "voided":
n}}` (the `admin_workers` precedent: lists return page-local `count`,
aggregate depths live on `workers/status`; the badge polls counts
without fetching rows).

**Auth:** `AdminAuth` extractor + admin host guard + mTLS seam,
identical to `approval::list_facts`. GETs skip CSRF (`csrf_guard`
passes `GET|HEAD|OPTIONS`).

**Status codes:** 401 missing/invalid/expired session; 403 wrong host,
below-Staff role, or strict-mode mTLS failure; 404 `unknown authority`
(the `list_captures` precedent — no owner-mismatch 404: staff are
deliberately unscoped); 422 `unknown status` (the `list_facts`
precedent); 503 either DB fails to open.

**Unresolved (author calls):** U1 `authority_slug` in rows needs a
second cross-DB lookup per row (shipments live in `p2.sqlite`, slugs in
`handbook.sqlite`, no FK) — draft emits `authority_id` only; U3 the
queue UI is blocked on TWO more missing pieces, not just this route —
a staff `GET .../shipments/:id` (full addresses/payloads) and a
staff paste-back (`paste_back` takes the customer DEMO caller, not
`AdminAuth`); U4 `customer_ref` + address-bearing `payload_json` on a
staff list assumes mTLS-listener PII visibility is allowed (OQ-1/OQ-7
still open); U5 whether the queue needs `postage_cost_cents`/`paid_by`
columns; U6 fixed newest-first (FIFO oldest-first not requested).
Evidence: `p2_labels.rs` (Shipment 268-276, `queue_manual` 1007-1079,
owner-404 564-572, 422s 526-532), `approval.rs:115-186`,
`admin_workers.rs:96-250`, `admin.rs` guards (CSRF passthrough at 242),
P2 DDL `migrations/p2_schema_baseline.sql`.

### Proposed addition: staff printing + full-service packet
(author direction 2026-10-28; still proposed — approve together with
the list route above, not separately)

**Business rule.** Most labels are printed by customers (they hold the
documents; existing customer routes unchanged). Staff can print any
label when needed, and full-service customers get a complete printed
packet from the staff side. Print location is an ops concern, not an
API concern: the endpoints below are location-agnostic, so any staff
host (or a future satellite location with DB access) can serve them.

**Staff label print:**
`GET /apostille/api/v1/shipments/{id}/labels/{label_id}/download`
— same bytes as the customer download (one shared loader; the customer
route at `p2_labels.rs:1083` stays the reference behavior), but
`AdminAuth` + admin host guard, deliberately unscoped (staff see all
shipments, the U-decision above). 404s mirror the customer route (`no
such shipment` / `no such label` / `label file not available yet`);
401/403/503 as the list route. Every print writes an audit row
(`label_print`, staff user + shipment/label ids — IDs only, never
bytes; the admin banner promises every action is audited). Idempotent
GET; no state change besides the audit row.

**Staff form print:**
`GET /apostille/api/v1/submissions/{id}/pdf` — staff twin of the
customer submission PDF (`p2.rs:1086`, same scoping problem, same fix).
Same auth, same 404 vocabulary (`unknown submission`), same audit rule
(`submission_print`).

**Full-service packet:**
`GET /apostille/api/v1/shipments/{id}/packet` → manifest JSON,
not a merged PDF (composition over new render work):
`{"shipment_id", "service_tier", "pieces": [{"kind":
"label|form|packing_slip", "label_id?", "submission_id?", "url",
"sha256"}]}` — `url` points at the staff twins above (absolute path,
same host). The staff UI prints piece by piece. `service_tier`:
`self_serve` (default) | `full_service`, stored on the shipment (D37
leaves no order container, so the shipment is the only home — U7).
packing_slip is the one new render (printpdf packing list: pieces,
counts, destination office, no prices — pricing table still open; U8
owns its exact contents). 404 when the shipment is unknown; 422
`not a full-service shipment` when tier is `self_serve` (printing one
piece stays available via the twins — the packet is the bundle).

**New unresolved:** U7 confirm `service_tier` lives on the shipment
(vs waiting for an order container — recommended: shipment, per D37);
U8 packing-slip contents and whether it needs a build stamp like the
form PDFs; U9 print-audit retention (IDs-only rows — recommend the
standard audit retention, no new rule).

## 10. P1 dependencies

- P1-D1: `authority` rows + slugs for pilot (Washington) with
  official `source`/`domain` rows and validated mail-to
  `channel`/`fact(topic=address|fee)` rows — P2 prefills read these.
- P1-D2: ingested official blank forms as `capture(kind=file/scan)` +
  extracted `text/` sidecars, so template authors measure `pdf`
  rects against the real edition.
- P1-D3: axum + SvelteKit scaffold serving `/apostille` with the WS
  bus, `audit` writer, and `xtask dev-seed` extended with one
  synthetic authority + template + submission + labels.
- P1-D4: conflict query (§7) + `task`/`batch` flow, powering U4/U5
  and template `needs_review` flips.
- P1-D5: `build`/`facts_hash` stamping so each PDF cites the build it
  rendered from.

## 11. Non-goals (P2 explicitly does NOT include)

- No blank-form/Adobe download path; no fillable AcroForm/XFA
  handling.
- No door-kits: no envelope stuffing, no prepaid-packet-to-door
  fulfillment, no pickup scheduling.
- No full-service tiers (staff files on customer's behalf, concierge
  review) — ops/P3.
- No carrier-API purchase, rate shopping, address validation APIs,
  voids/refunds automation, or payment/checkout (records cost only).
- No cross-border customs paperwork (CN22/23, HS codes);
  Canada/Mexico labels beyond domestic-US shape are P3.
- No drawn-signature legal-compliance claim; no DocuSign/
  qualified-e-signature integration.
- No customer identity/auth build (reuses Andrii Co. booking auth by
  reference).
- No MCP exposure of P2 objects; no auto-publish of submissions to
  handbook pages.
- No HTML-print-primary flow in P2 v1 (roadmap per P2-R6, not scope).

## 12. Open questions

- OQ-1 (PII store): separate `store/p2.sqlite` (recommended, §8) vs
  same-file tables in `handbook.sqlite`? Separate needs a small
  cross-DB join convention; same-file risks PII leaking into
  build/share flows.
- OQ-2 (content-repo location): where does the content repo live (new
  GitHub repo? path? access)? Templates (§3) and label payload
  presets assume it; until decided, `store/forms/` gitignored
  fallback.
- OQ-3 (AUTHOR DECISION — labels): staff-manual purchase
  (recommended, §7) vs carrier-API buy in P2? Plus: whose FedEx/USPS
  accounts, who pays postage, any markup, and whether existing
  booking billing covers it.
- OQ-4 (replica ground truth): who researches `accepts_replica` per
  pilot authority (staff calls via P1 batches vs author knowledge),
  and is Washington `yes`/`no` confirmable before the demo?
- OQ-5 (drawn signatures): is `drawn_optional` needed for the demo at
  all, or is wet-ink-only acceptable for P2 (recommended:
  wet-ink-only)?
- OQ-6 (customer auth): resolved prompt 011 — P2 consumes the
  handbook login stack ([auth-playbook](../auth-playbook.md):
  passkey/Apple/Google/Meta + magic-link fallback) built by the
  web/auth stream; the booking-auth-reuse idea is dropped (it
  contradicted the playbook).
- OQ-7 (PII retention): retention for submissions/labels/drawn
  signatures (voice got 365 days in design.md; customer PII
  undecided) — propose submitted 2 years, drafts 90 days, drawn PNGs
  1 year?
- OQ-8 (demo slice): is the P2 demo Washington→Viet Nam-use
  domestic-US labels only, or must USPS-vs-FedEx choice both be
  clickable in the demo?
