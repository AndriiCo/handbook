# Customer Payments: Stripe + Apple Pay + Billing Tab (research draft for review)

- Status: draft (prompt 010, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child
  session log). All Stripe/Apple claims cite the linked docs below.
- Context: P2 records `postage_cost_cents` + `paid_by` but charges
  nothing ([p2-forms-labels.md](p2-forms-labels.md) §7–§8). This
  draft specs the payments layer P2 deliberately deferred.

## 1. Recommended architecture (one paragraph)

Use **Stripe as the single money-moving processor**: axum creates
Stripe Checkout Sessions or PaymentIntents, the SvelteKit SPA
confirms them with Stripe.js (Payment Element, which includes Apple
Pay / Google Pay / Link automatically), and axum learns the outcome
only from **signed webhooks**. Keep our own `invoice` / `payment`
tables as a thin mirror of Stripe objects (ids, amounts, status),
never as a parallel ledger. This keeps PCI scope at SAQ A, reuses
one refund/dispute flow, and lets P2's `paid_by` field graduate
from a manual note to a Stripe-derived value without a schema
rewrite.

## 2. Rust Stripe integration options

**Primary: `async-stripe` (+ `stripe-webhook`).** Community-standard
async bindings (`github.com/arlyon/async-stripe`, crates
`async-stripe-*` split by API surface: core, payment, billing,
checkout). Webhook handling is now isolated in the `stripe-webhook`
crate (`Webhook::construct_event(payload, sig_header, secret)`).
Fits axum/tokio directly; one pattern is an axum extractor wrapping
signature verification that yields a typed `Event` (see Shuttle's
axum example, [Using Stripe Payments with
Rust](https://www.shuttle.dev/blog/2024/03/07/stripe-payments-rust)).
Pure-Rust HTTP via reqwest/rustls keeps the `web-stack.md` §1
parity rule (no OpenSSL). (Parent note: crate choice gets a final
version/API check at scaffolding.)

**Fallback: raw `reqwest` + `serde_json` against the REST API.**
Stripe's API is plain HTTPS + JSON ([API
reference](https://docs.stripe.com/api)); a hand-rolled client for
just `POST /v1/checkout/sessions`, `POST /v1/payment_intents`,
`GET /v1/invoices` is ~200 lines and removes a large dependency
tree. Prefer this only if `async-stripe` version churn hurts;
otherwise the typed crate wins on webhook event enums.

**What the server needs:** secret key from env (`STRIPE_SECRET_KEY`,
test vs live by `sk_test_` / `sk_live_` prefix), idempotency keys
on all mutating calls (axum generates per order attempt; Stripe
dedupes retries), and API version pinning (set `Stripe-Version` /
crate feature so upgrades are explicit).

## 3. Checkout vs PaymentIntent vs Invoices — which to use

| Path | When | Who builds the UI |
|---|---|---|
| **Checkout Sessions** (hosted) | Fastest P2-follow: one order = one session, redirect to Stripe, return | Stripe hosts; ~zero frontend |
| **Payment Element + PaymentIntent** | Custom in-SPA checkout, Apple Pay inline | Stripe.js Element in our SvelteKit page |
| **Stripe Invoicing** | Itemized invoice emailed + hosted pay page, auto-charge saved card | Stripe Dashboard/API; see [How invoicing works](https://docs.stripe.com/invoicing/overview) |

Recommendation: **Checkout Sessions for the first paid slice**
(least code, Apple Pay works with zero domain config on hosted
pages — see §5), then graduate the Billing tab to **Payment
Element** for saved-card reuse, and use **Stripe Invoices** as the
invoice system of record (§6) rather than building our own
numbering/PDF pipeline.

## 4. Webhooks (axum design)

Endpoint: `POST /apostille/api/v1/webhooks/stripe` — raw body bytes
preserved (axum `Bytes`), verified against `Stripe-Signature` with
the endpoint secret (`whsec_…`) before any JSON parsing. Use
constant-time HMAC comparison (the crate helper does this).

Events to subscribe (minimal set): `checkout.session.completed`,
`payment_intent.succeeded`, `payment_intent.payment_failed`,
`invoice.paid`, `invoice.payment_failed`, `invoice.voided`,
`charge.refunded`, `charge.dispute.created`/
`charge.dispute.closed`. Full event model: [Set up
events](https://docs.stripe.com/events/set-up-events.md);
failure/retry/signature-error handling: [Manage webhook
endpoints](https://docs.stripe.com/events/manage-webhook-endpoints).

Rules: (a) verify-then-ack — return 2xx fast, do heavy work (PDF
fetch, email) in a background task; (b) **idempotent handlers**
keyed by `event.id` (store processed ids; Stripe retries up to 3
days and delivery order is not guaranteed); (c) never trust amounts
from the client — re-fetch the PaymentIntent/Invoice server-side;
(d) keep test and live endpoint secrets separate
(`STRIPE_WEBHOOK_SECRET_TEST` vs `_LIVE`).

**Test mode + local dev:** Stripe test mode is a key prefix
(`sk_test_…`), and test/live data (customers, invoices) are fully
separate. Local webhook dev uses the Stripe CLI: `stripe listen
--forward-to localhost:$PORT/apostille/api/v1/webhooks/stripe`
prints a CLI signing secret, and `stripe trigger
payment_intent.succeeded` fires a realistic event. Fits `cargo xtask
dev` + `dev-seed`: seed script can create test-mode fixtures via
the API. No card numbers ever touch our logs — log only ids,
amounts, statuses.

## 5. Apple Pay on the web

**Two routes — strongly prefer Stripe-as-processor:**

- **(A) Stripe-as-processor (recommended).** Enable Apple Pay in the
  Dashboard; on **hosted Checkout / Payment Links it works with no
  additional configuration** ([Apple Pay /
  Stripe](https://docs.stripe.com/apple-pay)). On **Payment Element
  / embedded Checkout you must register every domain** showing the
  button (apex + `www` + subdomains, test and prod) under Dashboard
  → Payment Methods → Apple Pay ([same
  page](https://docs.stripe.com/apple-pay)). Stripe hosts the
  `apple-developer-merchantid-domain-association` file path on your
  behalf for registered domains in most setups; where it does not,
  serve it at
  `/.well-known/apple-developer-merchantid-domain-association`
  from axum/`ServeDir` (note: our app lives under `/apostille`, so
  the well-known file must be served at the **domain root** — plan
  the Caddy route now). No Apple Developer merchant ID, no
  certificates, no `ApplePaySession` code on our side — Stripe.js
  handles the payment sheet, and the charge lands as a normal
  card-wallet PaymentIntent. Safari + enrolled card required; the
  Element hides the button elsewhere.
- **(B) Direct Apple Pay on the web (not recommended).** Requires
  Apple Developer Program membership, a Merchant ID, a Payment
  Processing certificate, domain verification per domain (upload
  the association file, click Verify in the Developer portal), and
  owning the full `ApplePaySession` flow: `validatemerchant` →
  your server signs a merchant session with Apple's certificate →
  `paymentauthorized` returns an encrypted token → your server
  decrypts/processes via a gateway. Processor route (A) exists
  precisely to avoid this. Only revisit if we ever leave Stripe.

Device/browser matrix: Apple Pay sheet appears only in Safari
(macOS/iOS) with Wallet cards; Chrome/Edge users get Google Pay /
Link / card fields from the same Element — no extra code.

## 6. Invoice model

**Do not build invoicing from scratch — Stripe Invoices is the
system of record,** mirrored locally. What an invoice contains
(Stripe model,
[lifecycle/statuses](https://docs.stripe.com/invoicing/overview)):
customer + line items (description, quantity, unit amount, tax
behavior) + totals + status + hosted pay URL + PDF URL + number +
footer/memo.

- **Numbering:** Stripe auto-numbers (`INV-…` customizable
  prefix/sequence in Dashboard billing settings). Our local row
  stores `stripe_invoice_id` + `number` as opaque mirrors; never
  mint competing numbers.
- **Statuses:** `draft → open → paid`, with `void` / `uncollectible`
  side states ([same
  page](https://docs.stripe.com/invoicing/overview)). Local mirror
  enum matches 1:1; webhook events (`invoice.paid`,
  `invoice.payment_failed`, `invoice.voided`) drive transitions.
  Draft is editable; finalized (`open`) is close to immutable —
  corrections mean void + reissue, which the Billing tab must
  expose as staff actions.
- **PDF receipts:** every finalized invoice has a Stripe-hosted PDF
  (`invoice_pdf` URL) and a hosted pay page; receipts are emailed
  by Stripe per lifecycle settings. Billing tab "Download" links
  to the Stripe PDF (signed, expiring) or caches a copy as a
  `capture`-style blob per the P2 file-store convention. No
  `printpdf` invoice renderer to build.
- **Local schema sketch (extends P2 §8, lives in the PII-side DB
  per OQ-1):** `invoice(id, submission_id?,
  stripe_customer_id, stripe_invoice_id UNIQUE, number,
  amount_due_cents, amount_paid_cents, currency, status,
  hosted_url, pdf_url, created_at, paid_at)`;
  `payment(id, invoice_id?, stripe_payment_intent_id UNIQUE,
  amount_cents, method_summary /* "visa •••• 4242" / "apple_pay"
  */, status, created_at)`. No PANs, no CVCs, no full card numbers
  — ever.

## 7. Billing tab requirements (SvelteKit SPA + axum)

Customer-facing, per `customer_ref`:

1. **Invoices list** — number, date, description (authority +
   service), amount, status chip (draft hidden from customers;
   open/paid/void shown), pay-now CTA for `open` (link to Stripe
   hosted page or embedded Element).
2. **Invoice detail** — line items, totals, hosted PDF download
   button, payment history for that invoice.
3. **Payment methods** — list saved methods (brand, last4, expiry;
   SetupIntent-based save), set default, detach. Card entry is
   always a Stripe Element; our server sees only `payment_method_id`.
4. **Pay outstanding** — one click per open invoice; result lands
   via webhook → WS event (`billing:{customer_ref} →
   invoice.paid`) reusing the `web-stack.md` §3 bus.
5. **Receipts** — paid invoices show receipt download (Stripe
   receipt URL) + email-receipt toggle state.
6. **Staff extras (same tab, role-gated):** void + reissue, refund
   button (→ `POST /v1/refunds`, reason codes),
   mark-uncollectible, audit trail per invoice (every mutation
   writes an `audit` row per `design.md` §6).

Axum sketch: `GET /billing/invoices`, `GET
/billing/invoices/:id`, `GET /billing/invoices/:id/pdf` (redirect
to Stripe URL), `GET /billing/payment-methods`, `POST
/billing/setup-intent`, `POST /billing/invoices/:id/pay`, `POST
/staff/billing/invoices/:id/{void,refund}`. Auth reuses the Andrii
Co. session story (P2 OQ-6) — no new auth. (Parent note: the
`billing:{customer_ref}` topic extends the frozen topic set in
[README.md](README.md) resolution 5 — freeze update at
scaffolding.)

## 8. PCI scope

Using Stripe.js Elements / Payment Element / hosted Checkout (card
data goes browser→Stripe, our server only handles tokens/ids) keeps
us at **PCI DSS SAQ A** (minimal scope: no cardholder data
environment, no PAN storage/transmission). Rules that keep it that
way: never log request bodies on payment routes; never add `name`
attributes that would POST card fields to axum; CSP must allow
`js.stripe.com`; served over TLS (Caddy). Direct Apple Pay token
decryption (§5-B) would expand scope — another reason to stay on
route (A). Annual SAQ A attestation + Stripe's PCI guide are the
compliance artifacts, not code.

## 9. Refunds / disputes basics

- **Refunds:** `POST /v1/refunds` with `charge`/`payment_intent` +
  amount (full/partial) + reason (`requested_by_customer`,
  `duplicate`, `fraudulent`); money returns to the original method
  (Apple Pay refunds flow back to the wallet automatically).
  Webhook `charge.refunded` updates the mirror; staff action is
  audit-logged. Note: Stripe fees are not returned on refunds; our
  invoice memo should state the refund policy.
- **Disputes (chargebacks):** webhook `charge.dispute.created` →
  surface in staff Billing view with evidence deadline; submit
  evidence via Dashboard/API (`charge.dispute.closed` with
  `won`/`lost`). Lost disputes debit us + a dispute fee. Keep
  submission PDFs, tracking numbers, and delivery proof — P2
  artifacts double as dispute evidence.
- **Voids:** unfinalized intent → cancel; finalized unpaid invoice
  → void + reissue (never edit totals silently).

## 10. What P2 records vs what payments code charges (the seam)

| Concern | P2 today (record-only) | Payments layer (new) |
|---|---|---|
| `shipment.postage_cost_cents` | Staff-typed estimate of label cost | Becomes a Stripe invoice **line item** (postage passthrough) |
| `shipment.paid_by` | Free-text note (`customer`/`absorbed`/…) | Derived from Stripe: invoice `paid` + payment method → `paid_by='customer:stripe:<pi_id>'`; absorbed stays manual + audit row |
| Service fee (our margin) | Not modeled in P2 | Second invoice line item (decided $/%, see OQ) |
| Charging | Explicitly out of scope (P2 §11: "records cost only") | Checkout/PaymentIntent + webhooks flip `labels_pending → ready` only after `invoice.paid` (or staff override for absorbed) |
| Refunds/voids of labels | Staff-only state transitions, no money movement | Label void triggers optional Stripe refund flow (§9) |

Migration is additive: backfill `paid_by` from Stripe ids once
live; keep the manual values as valid enum members for the
absorbed/manual path.

## 11. Cross-border / tax — flagged, not decided

- **Q-T1:** US sales tax / VAT / GST on service fees per customer
  jurisdiction — Stripe Tax vs manual? (Affects invoice line
  `tax_behavior`.)
- **Q-T2:** Canada/Mexico orders: who is importer of record, are
  duties/taxes collected at checkout or on delivery?
- **Q-T3:** Currency: USD-only at launch or multi-currency Prices?
  (Affects Price catalog design.)
- **Q-T4:** Stripe account country + payout bank: Andrii Co. entity
  details determine available payment methods and tax forms.
- **Q-T5:** Receipts for consular/authority filing: do customers
  need itemized paper receipts beyond Stripe's PDF?

## 12. Open questions (decision-gated)

1. **OQ-P1 (pricing):** flat service fee per order vs
   per-authority/per-page? Any postage markup? (Blocks Price
   catalog + invoice line design.)
2. **OQ-P2 (checkout UX):** hosted Checkout redirect (fastest) vs
   embedded Payment Element in-SPA (nicer, needs domain
   registration)? Recommend hosted first, Element second.
3. **OQ-P3 (paid_by semantics):** confirm enum `customer_stripe |
   absorbed | manual_booking` and that labels release only on
   `invoice.paid` or staff override.
4. **OQ-P4 (domains):** production + staging domain list for Apple
   Pay registration; who owns DNS/Caddy for `/.well-known` at
   root?
5. **OQ-P5 (Stripe account):** whose account/keys, test-vs-live
   separation, who approves Dashboard settings (receipt emails,
   invoice numbering, dispute users)?
6. **OQ-P6 (PII DB):** do `invoice`/`payment` mirror rows live in
   `store/p2.sqlite` per P2 OQ-1 (recommended: yes)?
7. **OQ-P7 (saved cards):** is card-on-file + auto-charge for
   reissues needed at launch, or pay-per-invoice only?
8. **OQ-P8 (refunds policy):** who approves refunds, full vs
   partial windows, who eats Stripe fees on refunded orders?

## 13. Fake Stripe (W-3, `stripe.mode = fake`)

Loopback and staging boots only (`docs/specs/config-db.md` §9);
Stripe test keys stay the `sandbox` mode. Under `fake`,
`stripe::keys` returns keys that carry the store instead of a secret
(`Keys::live()` is always false, so every order is a test-mode order),
and the three Stripe calls answer from the p2 `fake_stripe_session`
table (`CREATE TABLE IF NOT EXISTS`) with no network:

- `create_checkout` mints `cs_fake_…` and returns the local DEMO
  checkout page (`/apostille/api/v1/stripe/fake/checkout/{id}`,
  server-rendered, no JS, a red "DEMO — no card is charged" banner);
- its **Pay (demo)** button (same-origin form post) records the
  session paid, then delivers `checkout.session.completed` to the
  real `/stripe/webhook` handler, signed with a per-process random
  secret (never stored, so nobody else can sign one): signature
  check, event dedupe and `orders::settle` run unchanged, and settle
  re-reads the fake session in Stripe's Checkout Session shape
  (`livemode: false`);
- `refund` returns `re_fake_…` and moves nothing.

The order's event reads "Paid $X (DEMO, no card charged)", its
receipt carries `demo: true` and a `receipt_url` to the DEMO receipt
page, and the customer's order page says "(DEMO — no card charged)".
A forged webhook (wrong secret) is still refused with 400.
