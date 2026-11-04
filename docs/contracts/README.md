# MISSION WIN contracts (B0, 2026-11-02)

Shapes the Rust lanes serve and the web lanes render. The JSON seeds in
`docs/contracts/` are the source of truth: web lanes copy them into
`web/e2e/fixtures/` under their prefix (`plan-*` SA-27, `orders-*` SA-28,
`admin-*` SA-26), Rust lanes round-trip them in a serde test. A field
change is a `request:` to the builder, who edits the seed and tells both
sides. Paths below are as the browser sees them (public under
`/apostille`, admin on the admin host). Every POST carries the CSRF
double-submit pair except the Stripe webhook.

## Route table

| Method + path | Router | Role | CSRF | Rate bucket | Owner | Seed |
|---|---|---|---|---|---|---|
| GET `/apostille/api/v1/authorities/{slug}/plan` | public | anyone | n/a | `plan_read` 60/min/IP | SA-22 | `plan.json`, `plan-empty.json` |
| GET `/apostille/api/v1/p2/orders/options?slug=` | public p2 | anyone | n/a | `orders_read` 60/min/IP | SA-24 | `order-options.json` |
| POST `/apostille/api/v1/p2/orders` | public p2 | customer | yes | `orders_write` 10/min/customer | SA-24 | `order.json` (201) |
| GET `/apostille/api/v1/p2/orders` | public p2 | customer (own) | n/a | `orders_read` | SA-24 | `orders-list.json` |
| GET `/apostille/api/v1/p2/orders/{id}` | public p2 | owner or staff | n/a | `orders_read` | SA-24 | `order.json` |
| POST `/apostille/api/v1/p2/orders/{id}/quote` | public p2 | owner | yes | `orders_write` | SA-24 (+SA-25 rates) | `order-quote.json` |
| POST `/apostille/api/v1/p2/orders/{id}/checkout` | public p2 | owner | yes | `orders_write` | SA-24 | `{"checkout_url": "…"}` or 503 `order-paused.json` |
| POST `/apostille/api/v1/p2/orders/{id}/confirm` | public p2 | owner | yes | `orders_write` | SA-24 | `order.json` |
| GET `/apostille/api/v1/p2/orders/{id}/labels/{leg}` | public p2 | owner | n/a | `orders_read` | SA-24 (address label, HTML NOT POSTAGE) / SA-25 (carrier PDFs) | HTML or PDF |
| POST `/apostille/api/v1/stripe/webhook` | public | Stripe (HMAC) | **exempt** | `stripe_webhook` 120/min | SA-24 | 200 `{}` |
| POST `/apostille/api/v1/p2/forms/{form_id}/fill` | public p2 | customer | yes | `forms_fill` 10/min/customer | SA-23 | `form-fill.json` (201) |
| GET `/apostille/api/v1/p2/forms/fills/{fill_id}.pdf` | public p2 | owner or staff | n/a | `orders_read` | SA-23 | PDF |
| GET `/apostille/api/v1/admin/version` | admin | staff | n/a | — | builder | `admin-version.json` |
| GET `/apostille/api/v1/admin/stats` | admin | staff | n/a | — | builder (B3) | `admin-stats.json` |
| GET `/apostille/api/v1/admin/model-calls?pipeline=&since=` | admin | staff | n/a | — | SA-22 | `admin-model-calls.json` |
| POST `/apostille/api/v1/admin/compile` `{slug, seed_urls?}` | admin | staff | yes | `compile.daily_runs` | SA-22 | `{"run_id", "topic"}`; events `compile-run-events.json` |
| GET `/apostille/api/v1/admin/plans?status=` | admin | staff | n/a | — | SA-22 | `admin-plan-proposals.json` |
| POST `/apostille/api/v1/admin/plans/compose` `{slug}` | admin | staff | yes | `compile.daily_runs` | SA-22 | `{"run_id", "topic"}` |
| POST `/apostille/api/v1/admin/plans/{id}/accept` / `reject` | admin | staff | yes | — | SA-22 | the plan row; `plan_accepted` |
| GET `/apostille/api/v1/admin/offices?status=` | admin | staff | n/a | — | SA-23 | `admin-office-proposals.json` |
| POST `/apostille/api/v1/admin/offices/{id}/accept` / `reject` | admin | staff | yes | — | SA-23 | the office row; `offices_accepted` |
| GET `/apostille/api/v1/admin/fact-checks?slug=` | admin | staff | n/a | — | SA-23 | `{"checks": [{fact_id, verdict, quote, model, at}]}` |
| POST `/apostille/api/v1/admin/fact-checks/run` `{capture_id}` | admin | staff | yes | daily cap | SA-23 | `{"run_id", "topic"}` |
| GET `/apostille/api/v1/admin/form-maps?status=` | admin | staff | n/a | — | SA-23 | `admin-form-maps.json` |
| POST `/apostille/api/v1/admin/form-maps/{id}/accept` / `reject` | admin | staff | yes | — | SA-23 | the `form_template` row |
| POST `/apostille/api/v1/admin/captures/upload` (multipart PDF) | admin | staff | yes | — | SA-23 | `{"capture_id"}` |
| GET `/apostille/api/v1/admin/orders?status=` | admin | staff | n/a | — | SA-24 | `admin-orders.json` |
| GET `/apostille/api/v1/admin/orders/{id}` | admin | staff | n/a | — | SA-24 | `order.json` + `customer_ref` |
| POST `/apostille/api/v1/admin/orders/{id}/refund` / `advance` | admin | staff | yes | — | SA-24 | `order.json` |
| GET `/apostille/api/v1/admin/fulfilment` | admin | staff | n/a | — | SA-24 | `admin-fulfilment.json` |
| POST `/apostille/api/v1/admin/fulfilment/{id}/packed` / `shipped` | admin | staff | yes | — | SA-24 | `order.json` |
| GET `/apostille/api/v1/fulfilment/{id}/packet.pdf` | admin | staff | n/a | — | SA-24 | PDF |
| GET `/apostille/api/v1/admin/work-items?status=&limit=&before_id=` | admin | staff | n/a | — | P392 | `admin-work-items.json` `list` |
| POST `/apostille/api/v1/admin/work-items` `{kind, payload, max_attempts?}` | admin | staff | yes | — | P392 | `enqueue_response` (201) |
| GET `/apostille/api/v1/admin/work-items/{id}` | admin | staff | n/a | — | P392 | `detail` |
| GET `/apostille/api/v1/admin/work-items/{id}/result` | admin | staff | n/a | — | P392 | result bytes (attachment) |
| POST `/apostille/api/v1/admin/work-items/{id}/retry` | admin | staff | yes | — | P392 | `retry_response` |
| POST `/apostille/api/v1/admin/work-items/{id}/cancel` | admin | staff | yes | — | P433 | `cancel_response` (pending or leased → failed) |
| POST `/apostille/api/v1/admin/workers/token` `{name}` | admin | staff, fresh step-up | yes | — | P433 | `token_response` (shown once) |
| POST `/apostille/api/v1/admin/work-items/{id}/stage-packs` | admin | staff | yes | — | P392 | `stage_response`; files in `<store>/packs-inbox/{id}/` |
| GET `/apostille/api/v1/worker/ws` (WebSocket) | public | plant worker (`worker.token` bearer) | n/a | 8 sockets/IP | P392 | `worker-frames.json` |
| GET `/apostille/api/v1/admin/sources/{id}/captures` | admin | staff | n/a | — | SA-14 (P394) | `{source, captures: [{id, captured_at, kind, trust, sha256, bytes, previous_id, reread, added, removed}], count}` |
| GET `/apostille/api/v1/admin/captures/{id}/diff` | admin | staff | n/a | — | SA-14 (P394) | `{capture, previous, reread, missing_sidecar, added, removed, lines: [{op: same\|added\|removed, text, old_line, new_line} \| {op: gap, lines}]}` |
| POST `/apostille/api/v1/admin/captures/{id}/draft-changes` | admin | staff | yes | daily cap | SA-14 (P394) | `{outcome: drafted\|skipped, reason?, added_lines, proposed, rejected, suppressed, fact_ids}` |
| POST `/apostille/api/v1/admin/changes/draft-pending` | admin | staff | yes | daily cap | SA-14 (P394) | `{runs: [{capture_id, outcome, …}], count}` |
| GET `/apostille/api/v1/authorities/{slug}/changes` | public | anyone | n/a | — | SA-14 (P394) | `{authority, changes: [{source_url, captured_at, previous_captured_at, added, removed, more_added, more_removed}]}` |
| POST `/apostille/api/v1/admin/facts/{id}/retire` `{reason}` (8-500 chars) | admin | staff | yes | — | R1 (plan §5.5) | `{id, authority_slug, status: "superseded"}`; `fact_retired`; 409 unless accepted |
| POST `/apostille/api/v1/admin/evidence` `{authority_slug, kind, fact_id?, preferred_fact_id?, slot?, stance, occurred_at, channel, summary, capture_id?, supports?}` | admin | staff | yes | — | R1 evidence (§5.4) | the note, staff view (201); `evidence_updated` |
| POST `/apostille/api/v1/admin/evidence/{id}/withdraw` `{reason}` | admin | staff | yes | — | R1 evidence | the note, staff view; `evidence_updated` |
| GET `/apostille/api/v1/admin/evidence?slug=&status=published\|withdrawn\|all` | admin | staff | n/a | — | R1 evidence | `{notes: [staff view], count}`, newest first |

Every admin route gets an `admin-denial` case (no session → 401/403 on
the admin host, and the public host never serves it).

`GET /apostille/api/v1/authorities` rows now also carry `kind`
(`country | state | province | office`) and `verified_at` (unix seconds
of the newest staff accept, or null) — P320. Since R1 only facts still
`accepted` count toward `verified_at`, so a retired fact's accept drops
out, and `/plan` `offices[]` serves an accepted office only while every
fact it quotes (`fact_ids` and each piece's `fact_id`) is accepted.

`GET /apostille/api/v1/authorities/{slug}/facts` rows (and `/plan`
`facts[]`) now also carry `accepted_at` (unix seconds of the fact's
newest staff accept, or null); each citation carries `kind` (`web |
pdf | upload`) and `capture_id`. A source-less capture cites the
official website only when it is a page (`warc`) or an upload
(`file`); a dangling source cites nothing — R1 (plan §5.2).

`POST /orders` body (P325): `{slug, kind, path_id|null, form_fill_id|null,
return_to: {name, line1, line2|null, city, region (2 letters), postal_code,
country: "US"}}`; `address_label` answers 201 with status `labels_ready`;
paid kinds answer 503 `paid_tiers_paused` while paused, 409
`option_unavailable` (with `reason`) when the card is disabled for any
other reason: a fact (a PO box, a disputed PO box) or no fact (no courier
street for FedEx, no readable mailing address for USPS).

`GET /plan` also carries the `authority` row (R1, §5.3): `{slug, name,
region, country, kind, path, official_url}`, where `path` is the readable
address (`usa/washington`) and `official_url` the https
`website_official` source with the shortest URL path (or null), so the
page heads itself without the `/authorities` index; null for an unknown
or hidden slug (`plan-empty.json`).

`GET /plan` also carries `evidence[]` (R1, §5.4): the published staff
notes, oldest first, `[]` when none — `{id, kind, stance, fact_id,
preferred_fact_id, slot, date (YYYY-MM-DD, America/Los_Angeles), channel,
summary, recorded: false, supports, by: "Staff reviewer"}`. A call, email
or visit note names the accepted fact it confirms, contradicts or adds
context to; an absence names only a slot; a decision names a slot, the
preferred fact of a conflict, and in `supports` the ids of the notes in
the same array it rests on. Notes never enter a fact's `citations`, and
`created_by` is never public. The read re-runs the contact guard: a note
leaves `evidence[]` once its anchor fact, or the accepted fact that made
one of its numbers or emails public, is retired. Every guard refusal is a 422 `{"error":
"<plain reason>"}` (an unknown body field such as `call_id` is a 422
parse error); withdraw answers 404/409 for a missing or already-withdrawn
note, or one a published decision rests on.

## WS events (ids only; the client re-reads the API)
Seeds: `ws-events.json`.
- `authority:{slug}` (anyone): `fact_accepted {fact_id}`,
  `fact_retired {fact_id}`, `plan_accepted {plan_id, version}`,
  `offices_accepted {office_ids}`, `evidence_updated {note_id}` (a note
  published or withdrawn).
  Publish with `ws_bus::publish_authority_event`; any other kind is
  refused.
- `billing:{customer_ref}` (owner or staff): `order.created`,
  `order.paid`, `order.refunded`, `label.ready`, `kit.packed`,
  `order.shipped`, `tracking.update` — each `{order_id, leg?}`.
- `shipment:{id}` (owner or staff): `tracking.update {shipment_id,
  status}`.
- `run:{id}` (staff): `compile.step`, `compile.done` (and the existing
  draft-run steps).
- `ops` (staff): `fact_accepted {fact_id, slug}` (live since P320),
  `order.paid`, `order.needs_staff`, `compile.done`, `model_call.error`,
  `plan_accepted`, `kit.packed`.

## Enumerations
- Order kind: `label_fedex | label_usps | mail_kit | address_label`.
- Order status: `draft → quoted → awaiting_payment → paid →
  label_purchasing → labels_ready → (packing → shipped) → in_transit →
  delivered`; side states `needs_staff`, `refunded`, `canceled`. Free
  `address_label` goes `draft → labels_ready`.
- Leg role: `outbound` (to the authority), `return` (to the customer),
  `kit_to_customer` (the kit itself).
- Plan block kind: `step | callout | office | fees | mail | forms |
  timing` (`callout.tone`: `info | warning`). `show_for: []` means every
  path.
- Office visit kind: `walk_in | appointment | mail_only`;
  `timezone_source`: `quoted | staff_confirmed` (no tz otherwise, so no
  "Open now").
- `model_call.outcome`: `ok | refusal | length | http_4xx | http_5xx |
  timeout | transport | parse_error`; `pipeline`: `draft | factcheck | offices | plan
  | forms | triage | precheck | voice`.
- Cross-check verdict: `supported | contradicted | not_found`.
- Evidence note kind: `call | email | visit | decision | absence`;
  `stance`: `confirms | contradicts | context`; status `published |
  withdrawn`.

## Shared rules
- `conflict-cases.json` (plan §5.1): golden cases for the field-level
  conflict rule, `handbook_store::db::facts_disagree` and its JS twin
  `web/src/lib/public-ui/compare.js`. A case is `{name, topic,
  topic_b?, a, b, disagree, tokens_a?, tokens_b?}` (`topic_b` for a
  cross-topic pair; `tokens_*` pin `value_tokens` of each value).
  `crates/web/src/conflicts.rs` and `web/test/compare.test.mjs` run
  every case both ways round, so a rule change edits the cases first.
