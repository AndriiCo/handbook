# MISSION "WIN": Apostille Handbook as a complete apostille service, with Meta models building each page

## Context

The judges see a 3-minute video and the live URL. The rubric is Impact 30, Meta AI tooling 25,
Demo 25 and Technical depth 20, and the startup track asks whether the product "would hold up in
production". Production `0.1.0+4dcd234` scores 5/10. The newest internal judge gave Impact 7,
Meta 7, Tech 8, Demo 6. Its complaints: empty production dossiers, a hardcoded page structure,
forms and FedEx still on the roadmap, a 1990s admin, and no latency in the trace.

Today the site stops at a list of cited facts. There are no plain-language instructions, no
structured offices or hours, no runtime forms, no customer label or kit purchase, no payments, a
cabinet that lists submissions only, and an admin built from raw tables with no shell. Several
"facts" are hardcoded in code: NATIVE, IN_FORCE, FEATURED names, HagueMap ids, laneOf,
ChainView's bundled Egypt pack, the DS-4194 constant, and the Egypt USPS-only hop.

Outcome: a visitor gets one clear, quote-backed plan for their document: which office, open
now or not, what to bring, which form, total cost, where to mail. They can then fill the
official form, buy a label or a mailing kit, pay, and watch the order move in their cabinet.
Meta models build each page at runtime: discover pages, extract facts, cross-check them,
structure offices and hours, compose the page's render rules and map form fields.
Deterministic validators gate every model output, and a human approves it.

Operator decisions taken in this session (Mon 11-02, about 12:40-14:20 PST):
- The freeze is extended: code deploys are allowed until Wed 11-04 at about 09:00 PST.
- Payments and live carrier postage are ordered: build on test/sandbox keys, and go live once
  the operator installs live keys in admin `/config`. The agent never sees secret values.
- USPS direct: the account, CRID/MID/EPA and Labels API approval are in hand.
- FedEx direct: production Ship keys with label certification are ready.
- Stripe hosted Checkout (PCI SAQ A).
- **This session becomes the builder and orchestrator.** The Muse builder loop is paused, so
  there is exactly one writer on `main`.

Design source: 7 read-only subsystem maps, 3 independent plans, 2 judges (both picked the
demo-first plan as the backbone, with grafts from the other two) and 2 critics on this text.

## Architecture

### A. Meta pipeline (runtime only: propose → deterministic gate → staff accept → WS publish)

| Job | Model | Output schema (strict json_schema) | Deterministic gate |
|---|---|---|---|
| Discover pages | muse-spark-1.3 | `LinkTriage`: ids from a candidate enum on official hosts | Only candidate ids. robots, SSRF guard, 1 req/s per host |
| Extract facts | spark topic-group passes (office, money/time, prepare/mail, routing); long text goes to muse-wonder-1.0 | existing `ProposedRecords` | `analyze::validate_response` (byte span, 8-400 chars, value inside quote) |
| Cross-check (grounding) | muse-wonder-1.0, whole sidecar | `FactChecksV1` {supported, contradicted, not_found} + quote | The quote must be a sidecar substring, otherwise not_found. Shown to reviewers. The public chip appears only if the check ran before the accept |
| Structure offices | spark | `OfficeSetV1` (address, hours, walk-in, appointment, PO box, fact_ids) | `address_ok`, `hours_ok` (am/pm, ranges), PO box forced from cited text, tz only if quoted or staff-confirmed |
| Compose render rules | spark, reasoning high | `RenderPlanV1` (fact_id enum = accepted ids) | `validate_plan`, then a Wonder entailment pass that drops unsupported steps |
| Map form fields | spark over the lopdf text layer + AcroForm field list | `FormMapV1` {acroform name → label, help, prefill_topic, label quote} | The label must be in its cited span; AcroForm names come from an enum |

- Every call follows `.agents/meta-api-findings.md`:
  - temperature unset, `reasoning_effort` set, streaming;
  - a stable developer prefix with `prompt_cache_key`;
  - jittered retries that honour Retry-After;
  - refusal and length checks;
  - only public text, never customer data, and no `-contributor` model.
- No Files or Responses path, so the `media.terms_read` gate needs no decision.
- A shared `MetaRestClient::chat_json` goes in `crates/handbook-agent/src/model.rs`. It reuses
  `post_chat` (:218), `retry_delay` (:98) and `parse_stream` (:472).
- `model_call` row per HTTP attempt (model, pipeline, latency, TTFT, input, output, cached and
  reasoning tokens, outcome). Daily caps are counted from these rows. The trace console shows
  p50/p95, and so does the dashboard.
- `RenderPlanV1`:
  - top level: `{headline, paths[] (document types), blocks[] with show_for, hide_topics}`;
  - block kinds: `step | callout | office | fees | mail | forms | timing`.

  `validate_plan` checks:
  - every fact_id is accepted and belongs to this authority;
  - each block cites topics that fit its kind;
  - every number, amount, PO box, street number and time appears in a cited statement (port
    `translate.rs` `protected_spans` / `guard_ok`, :288/:443);
  - the hours guard passes;
  - no URLs, no HTML, no banned words, length caps hold.

  The public read re-runs the filter against the current accepted facts.
- Compile run (admin button): discover → capture → extract → cross-check → offices → compose,
  streamed over `run:{id}`. Ids come from the one `draft.rs` allocator, exposed as
  `next_run_id()`. Capped per run and per day, never on a timer. Composing is on-demand
  (`plan.auto_compose=false`), so batch approvals do not churn plans.
- Bot-blocked official sites (sos.wa.gov returns 403): staff "Upload official document" creates a
  `capture(kind=file, trust=staff_verified)` with a proper text layer.

### B. Public dossier: "Your apostille plan"
- `GET /api/v1/authorities/{slug}/plan` is model-free and rate-limited. It always returns 200,
  with `{plan|null, facts, offices, forms, provenance}` (a 404 here would be a console error
  that fails GP2-page).
- Allowlisted Svelte blocks only:
  - PathPicker (`?path=`);
  - PlanStep with CitationMark chips;
  - OfficeCard: walk-in/appointment badge, "Open now · closes 4:30 PM" via Intl (only with a
    grounded tz), hours table, outbound directions link;
  - FeeSummary total;
  - MailBlock, with CTAs for a free label, a paid label and a kit;
  - FormsBlock: the official link plus "Fill it online";
  - Callout.
- Fallback: with no accepted plan, today's `buildGuide` (guide.js:238) renders.
- The Evidence section with raw statements stays uncollapsed, which GP2-page needs.
- "How this page was made" panel, fed from `/plan` provenance:
  - the models, validator results and cross-check counts;
  - the run's latency and tokens;
  - "Approved by staff reviewer" and the date, with no staff identity.

  This is the only Meta evidence a judge can reach on the live URL.
- Runtime-only cleanup. Delete:
  - IN_FORCE and NATIVE;
  - the DS-4194 `SYNTHETIC_ORDER` constant and route;
  - ChainView's bundled `packs/egypt-consular.json`;
  - HagueMap memberIds;
  - the slug laneOf.

  Instead:
  - FEATURED keeps a slug-only list as routing ids, so GP1 always has ≥3 /authority links;
    names, flags and fact counts come from the API;
  - flags derive from `authority.country`;
  - lanes use `authority.kind`, which the builder adds to `public_authorities` (approval.rs:801)
    together with `verified_at`;
  - Hague membership comes from accepted HCCH `jurisdiction`/`legalization_chain` facts, with an
    empty state when there are none;
  - ChainView renders accepted `legalization_chain` facts only;
  - /order document types come from `plan.paths`, falling back to the topics of accepted facts.

### C. Fillable forms (protected core)
- F1, no model needed for the fill:
  - lopdf reads the official PDF's AcroForm field list;
  - Spark's `FormMapV1` maps each field to a label and a prefill topic, grounded in the text
    layer;
  - staff accept it as `form_template` (an INSERT; status active);
  - the customer fills it in a guided form, with prefill from accepted facts (`compute_prefill`,
    p2.rs:464, which already withholds conflicts);
  - the server writes the values into the official PDF (fill-and-flatten) and stores it.
- F2, cuttable: an auto-layout clean-room redraw (`forms::PdfRenderer`) for flat PDFs with no
  AcroForm.
- The kit prints the filled PDF. If no form is accepted, it prints the official blank form plus
  the checklist.

### D. Commerce (p2.sqlite; additive tables; PII stays there)
- Customer options (the user's ask):
  1. `label_fedex` / `label_usps`: a paid print-at-home pair. The outbound label goes to the
     authority's accepted mail or office address, and the return label comes back to the
     customer. The PDFs are kept in the cabinet.
  2. `mail_kit`: shipped to the customer. It contains:
     - the filled form;
     - a checklist printed from the accepted plan;
     - an envelope and stiffener;
     - an enclosed prepaid label to the authority;
     - a prepaid return envelope.

     Staff fulfil it from a queue.
  3. USPS self-print: `label_usps` with postage (paid), or `address_label` without postage
     (free; `labels::printable::render_printable`, printable.rs:18, NOT POSTAGE).
- Identity: sign-in (passkey, OAuth or magic link) is required at the Delivery step. Every
  order creates one backing `form_submission`: the real submission, or a draft shell with the
  order's `customer_ref`. Every leg's `shipment` row references it, which satisfies `shipment.submission_id NOT NULL` plus its
  foreign key (p2_schema_baseline.sql:25,35) and the `P2Owners.shipment_owner` authz. Tests
  insert each option's legs with `foreign_keys=ON`, and check that one customer cannot read
  another's order or shipment.
- Carrier policy (`crates/labels/src/policy.rs`) comes only from accepted facts:
  - a PO Box or a cited "USPS only" in `mailing`/`po_box`/`authority.ship_to_fact_id` makes the
    route USPS-only, and the disabled FedEx card cites the fact;
  - with no accepted address, paid options are disabled with the reason;
  - the hardcoded `MailHop::EgyptConsulate` rule (carrier.rs:43) is removed.

  Destination allowlist: accepted office ZIPs plus the order's return ZIP. `check_zip`
  (crates/labels/src/live.rs:532) is rewritten; `labels.live_ship_to_allow` stays for the staff
  path only.
- Flow:
  1. `POST /p2/orders` (CSRF double-submit, RateBuckets, an unguessable `ord_` id). The response
     includes the server-authored WS `topic`.
  2. Quote: FedEx Rate (`rate_quote_body`, crates/labels/src/live.rs:366) or USPS Prices v3,
     plus operator fee config. This lifts specs decision 10 for commerce.
  3. Stripe hosted Checkout over ureq, with an Idempotency-Key and a pinned version.
  4. The webhook sits on the PUBLIC router and is the only CSRF-exempt POST: raw body, HMAC via
     `aws_lc_rs` (precedent `crates/auth/src/meta.rs:182`), `stripe_event` dedupe, then a
     server-side session re-fetch (amount, paid, livemode).
  5. A reconciler worker (`follow::spawn_worker`, follow.rs:407) plus `POST /orders/{id}/confirm`
     on the success redirect. This covers staging, which Stripe cannot reach, and missed
     webhooks.
  6. `label_purchase` ledger:
     - UNIQUE `ord:<id>:<leg>`;
     - the in_flight row is committed before the carrier call;
     - a timeout or 5xx after send becomes `failed_unknown`, goes to needs_staff and is never
       auto-retried;
     - a carrier 4xx voids the sibling leg and runs an automatic idempotent Stripe refund.
  7. A tracking poller (USPS Tracking v3, FedEx Track), plus customer-entered tracking for the
     free option. It writes `tracking_event` and `order_event`.
- Mode coherence:
  - Stripe test mode buys only FedEx sandbox or USPS TEM postage;
  - a paid quote returns 503 `paid_tiers_paused` on a mode mismatch, when `orders.enabled` is
    false or when caps are hit;
  - demo-session customers can never pay in live mode.

  Switches are read through a new 30 s cached config reader (builder; `config::get_*_for_job`
  has no cache today), and LiveConfig is built per request. Installing keys therefore needs no
  restart.
- Carriers:
  - FedEx:
    - `labels.fedex_base_url` replaces the 4 hardcoded `FEDEX_BASE_PROD` sites
      (fedex_live.rs:272/320/385/474);
    - ship-body fixes: top-level accountNumber, labelResponseOptions, and a return leg that
      swaps the parties;
    - `buy_pair` keeps the PDF (live.rs:548);
    - `require_staff(true)` (live.rs:531) becomes an actor policy: staff, or customer with
      customer caps.
  - USPS:
    - new `crates/labels/src/usps.rs`: OAuth client-credentials, the Payments v3 authorization
      token sent in the `X-Payment-Authorization-Token` header, a Labels v3 `imageInfo` body and
      multipart parsing;
    - fix the fixture asserts at client.rs:756/876;
    - `SandboxGuard` becomes `UspsLiveGuard` (usps.live, caps, livemode).
- Cabinet:
  - `/orders` shows order cards read from `customer_order`;
  - `/orders/[id]` shows the event and tracking timeline, label PDFs and the receipt;
  - both subscribe to `billing:{customer_ref}` and `shipment:{id}`, with no polling;
  - `/billing` shows real receipts;
  - `p2::caller_customer_ref` (p2.rs:120) becomes `pub(crate)` and replaces `p2_labels::caller`;
  - `/shipping` drops the hardcoded `authorityId='1'` and the staff controls.

### E. Admin console
- The root `+layout.svelte` skips the public chrome on `/(admin)` routes.
- The `(admin)/+layout.svelte` shell:
  - a shadcn Sidebar with groups Overview, Content, Commerce, Comms and System;
  - breadcrumbs, a ⌘K palette and a staff menu;
  - an env badge fed by a new admin `/version` route;
  - a live dot;
  - the literal "Staff only".
- Root redirects to `/dashboard`: admin.rs root_to_review and smoke.mjs change together. `/triage`
  stays as a route because admin-denial.spec uses it.
- Dashboard:
  - KPIs and Meta tiles (calls per model, p50/p95, cache share, A5);
  - zero-dependency SVG charts;
  - an activity feed from audit rows plus the new staff-only `ops` topic.
- Review console:
  - a data table with a detail pane and j/k/a/r keys;
  - "accept all verified from this capture";
  - a Wonder chip;
  - tabs: Page plans (validator report plus side-by-side preview and diff), Offices, Forms.
  - It keeps "Review queue", "Accept", "Trace console" and "Draft from this capture" exactly.
- New pages: Orders (table plus detail Sheet), Fulfilment (To pack / Awaiting label / Shipped),
  Compile (run timeline), Labels with the `csrfHeaders()` fix.
- Workers, Config and Calls are restyled; Calls moves from polling to `call:{id}`.

### F. Data, WS, config, routes (no migration, no ALTER, no CHECK change)
- handbook.sqlite: `model_call`, `fact_check`, `page_plan`, `office_record`, `form_map`,
  `compile_run`. Forms are inserted into the existing `form_template` table.
- p2.sqlite: `customer_order`, `order_shipment(order_id, shipment_id, role)`, `order_event`,
  `label_purchase`, `stripe_event`, `tracking_event`. Their DDL runs on `p2conn` before main.rs:965
  drops it.
- An `order_event` is written inside the p2 transaction. The handbook audit row is written after
  commit, keyed idempotently.
- WS:
  - `authority:{slug}`: the "fact_accepted only" guard (admin.rs:686, tests main.rs:3905/4315,
    docs ws_bus.rs:6-8 and ws.rs:5-6) becomes the allowlist {fact_accepted, plan_accepted,
    offices_accepted}, ids only;
  - `billing:{customer_ref}` carries order.*, label.ready and tracking.update (no `orders:`
    noun; ws.rs:3 retired it);
  - `shipment:{id}` gains tracking.update;
  - new staff-only `WsTopic::Ops`.
- B0 publishes a route table (router, role, CSRF, rate bucket) for every new endpoint, and an
  admin-denial spec covers each new admin route.
- Config keys:
  - `stripe.secret_key` and `stripe.webhook_secret` (secret);
  - `orders.{enabled, fee_label_cents, kit_price_cents, currency, ship_from, daily_cap, max_order_cents}`;
  - `labels.{fedex_base_url, customer_live, customer_daily_cap, customer_max_usd}`;
  - `usps.{client_id*, client_secret*, crid, mid, manifest_mid, epa_account, base_url, live, live_daily_cap, live_max_usd}`;
  - `plan.auto_compose`, `compile.daily_runs`, `tracking.poll_enabled`.

## Lanes: 7 worktree writers plus the builder

Each lane runs its own full gates and live evals at the branch tip, so the builder re-checks only
the delta (the builder is the critical path, about 38 of the 44 h). Lanes own only their module
files. Shared hooks are stubbed by the builder in B0. Each lane marks finished steps `ready:` in
`docs/lanes/<name>.md`. Commit ids are builder `P314+` and lane `<lane>-<n>`. Web fixtures are
owned by filename prefix: `plan-*` (SA-27), `orders-*` (SA-28), `admin-*` (SA-26). The B0 seeds
are checked by each Rust lane's serde round-trip test.

| Lane | Owns | Slices in order (estimate) |
|---|---|---|
| **builder** (this session) | Cargo.toml; `crates/web/src/{main,admin,approval,ws_bus}.rs`; `crates/auth/src/ws.rs`; `crates/config/src/lib.rs`; both `+layout.svelte`; app.css; `components/ui/*`; package.json; shots, smoke and demo specs; README; demo-script; LOG; operator pages | **B0** (2h): merge the `meta` lane's unmerged `meta-1` (prompt v2; eval-gated per builder.md); contracts, the route table, the fixture seeds and lane briefs; stubs (DDL hooks on conn and p2conn, `routes()` merges, `publish_authority_event`, all config keys, the cached config reader, `next_run_id`). **B1** (2.5h): chrome split, tokens, shadcn sidebar/sheet/table/breadcrumb/select/switch/progress/alert. **B2** (2h): `(admin)/+layout` shell, `/version`, root redirect + smoke. **B3** (2.5h): `/admin/stats`, the `ops` topic, the WS allowlist, authority kind and verified_at. Then merges, 4 trains, and **B5** (4h): gates, demo and README |
| **SA-22 plan** | `handbook-agent/src/model.rs`; `crates/web/src/{telemetry,plan,compile}.rs`; `crates/web/prompts/`; `xtask/src/eval.rs` (plans and offices modes, on request from SA-23) | M1 telemetry + `chat_json` (3h); M2 RenderPlanV1 + `validate_plan` + read filter + entailment + plan routes + `eval --plans` with planted faults (10h); M6 compile + link triage (6h) |
| **SA-23 ground** | `crates/ingest/src/{html,pdf}.rs` + extract hook; `handbook-agent/src/runner.rs` + prompts; `crates/web/src/{factcheck,offices,forms_map}.rs`; `crates/forms/**`; `evals/dossier/` | E1 readable HTML + lopdf text layer + AcroForm list + staff document upload (6h); **O1** OfficeSetV1 + guards + accept (5h); **M3** Wonder cross-check (4h); **F1** FormMapV1 + fill-and-flatten (6h); M4 topic passes, eval-gated (3h); F2 redraw (3h) |
| **SA-24 commerce** | `crates/web/src/{orders,stripe,reconcile,fulfilment,admin_orders}.rs`; p2.rs `caller_customer_ref` made pub(crate) | C1 tables, state machine, backing submission, API (dark) (5h); C2 Checkout + webhook + reconciler + confirm + refunds (6h); C5 kit queue + packet (calls `render_packet_cover` as-is) + admin order API (5h) |
| **SA-25 carriers** | `crates/labels/**`; `crates/web/src/{p2_labels,fedex_live,tracking}.rs`; `xtask/src/{fedex,usps}.rs` | CR1 FedEx base URL, body fixes, keep PDF, rate quote, USPS client, all starting at hour 0 (6h); C3 ledger + actor policy + `policy.rs` + allowlist + caller swap (5h); C4 UspsLiveGuard + Prices + TEM proof (5h); C6 tracking (4h) |
| **SA-26 web-admin** | `routes/(admin)/**` except +layout; `lib/admin/**` | W1a dashboard stub → dashboard + charts + labels CSRF fix (6h); W1b review console + Page plans/Offices/Forms tabs + trace latency (6h); W1c Orders/Fulfilment/Compile (5h); W1d restyle Workers, Config, Calls (4h) |
| **SA-27 web-public** | `routes/(public)/**`; `lib/public-ui/**`; `components/brand/{HagueMap,FeeReceipt}`; `(customer)/forms/**` | W2b runtime-only cleanup (2h); W2a plan renderer + OfficeCard + provenance (7h); W2c forms: dossier → guided fill → PDF (4h) |
| **SA-28 web-customer** | `(customer)/{order,orders,billing,shipping}/**`; `lib/customer/**` | W3a /order: Destination → Document → Form → Delivery (4 cards, live quotes, cited disabled reasons; sign-in gate) → Pay, **keeping the stepper's `aria-current="step"`** (GP4) (6h); W3b cabinet list + timeline + billing + shipping cleanup (6h) |

If the operator keeps the cap at 6 writers, SA-28's work moves to SA-27 after W2a.

## Timeline (Pacific) and deploy trains

| Window | Lands | Train |
|---|---|---|
| Mon 14:30-16:30 | Operator step 0. B0. Rust lanes start as soon as their stubs land (M1, E1, C1, CR1); web lanes start on the fixture seeds | none |
| Mon 16:30-21:30 | B1, B2, M1, C1 (dark), W2b + B3 authority fields, CR1, E1 | **T1 about 21:30-22:30**: shell, telemetry, trace latency, runtime-only cleanup, meta-1 prompt v2 |
| Mon 22:00-Tue 09:00 (operator sleeps) | M2 + W2a (one train), O1, M3, B3, W1a, W1b, C2, C3, C4, W3a, W3b | **T2 about 09:00-10:00**: plans, offices, cross-check, commerce in test mode, cabinet, dashboard |
| Tue 10:00-13:00 operator block | Keys (sandbox first). Seed URLs and uploads. Compile for the hero slugs and 2-4 more. Approve facts, then offices, then plans | none |
| Tue 10:00-17:00 | F1 + W2c, C5 + W1c, C6, M4, M6, W1d, F2, B5 | **T3 about 17:00-18:00** |
| Tue 18:00-20:00 | Operator: live keys if chosen, one real capped purchase per option, publish forms, production dry run. A5 re-run before 20:29 | none |
| Tue 20:00-24:00 | Video on production (fixture and staging takes as fallback) | none |
| Wed 00:00-08:00 | Fixes only | **T4 by 08:30, prod-after by 09:00** |
| Wed 10:00 / 12:00 | Tag `submission-2026-11-04` after RO-2; the operator submits | freeze |

Every train follows the existing release path: plant stamp, then staging acceptance (scoreboard
plus live demo take, staging-green), then `ship-release.sh` and `ops-deploy.sh --skip-accept
--allow-today-overlap`, then prod-before and prod-after under the §7 rollback rules. Each deploy
is reported with its version, the previous version and both scoreboards.

## Operator steps (exact commands kept current in `docs/ops/operator-today.md`)
0. Now:
   - pause the Muse builder `/loop`;
   - commit edits to AGENTS.md §7 (deploys until Wed 09:00, lane merges until Wed 07:30) and
     AGENTS.md §1 (this order covers payments, live USPS and FedEx postage and kits, and lifts
     specs decisions 10 and 37 and the p2 door-kit non-goal);
   - update `.agents/goal.md` (Calendar, Lanes, priorities), `.agents/roles/builder.md:71,76` and
     the lane role files (B0 drafts them in `docs/lanes/sa-22..28.md`);
   - raise the write-lane cap to 7.
1. Sandbox credentials for lane proofs:
   - create FedEx developer-portal **test** project keys and account, and confirm the USPS TEM
     credentials;
   - put the Stripe test, FedEx sandbox and USPS TEM env files on plant (0600, never printed).

   If FedEx sandbox keys are missing, FedEx stays on recorded synthetic fixtures until the Tue
   18:00 capped live buy.
2. Mon evening:
   - approve usa-wa-sos's 17 existing proposals (≥8 accepted, keep 2 proposed);
   - add official Viet Nam seed URLs in admin Workers → fetch, because production gave vietnam 0;
   - commit `google.svg` and `apple.svg`;
   - re-run `ops-a5-eval.sh` and `ops-a2-record.sh`.
3. Tue 10:00-13:00:
   - install keys in `/config` (sandbox first) and register the Stripe webhook;
   - set fees, kit price and ship-from;
   - run Compile;
   - approve facts, then offices, then plans (keep 1 plan proposed for the video), then forms.
4. Before live money: publish refund, terms and contact pages, update the privacy note for stored
   shipping addresses, and decide sales tax.
5. Tue 18:00: decide live or test mode for judging; make one real capped purchase per option
   (void any unused label).
6. Record the video. Crop or blur any real label. Add the video link to the README.

## Video (180 s; captions name the model doing the work)
| Time | Beat |
|---|---|
| 0:00-0:10 | Hook: search "Washington". |
| 0:10-0:40 | "Birth certificate": the plan reflows with an open-now office, what to bring, a cost total and where to mail, every line with a citation chip. |
| 0:40-0:55 | Citation popover, then the "How this page was made" panel (Spark composed, Wonder cross-checked, validator passed, staff approved). |
| 0:55-1:30 | Admin dashboard, then a Compile run streaming tokens and latency, then the Page plan validator. Accept, and the public page flashes live. |
| 1:30-1:45 | The official form arrives prefilled from facts, and the filled PDF is ready live. |
| 1:45-2:15 | Delivery: FedEx is disabled by the cited PO Box fact. Choose the kit, pay on Stripe, and the cabinet flips to "Paid" live. |
| 2:15-2:48 | Fulfilment: buy labels and print the packet; the customer's timeline gains "Shipped" and tracking. |
| 2:48-3:00 | Close on the URL and the model list. |

## Verification
- Web: AGENTS.md §4 in full (shots, check, imports, tailwind, both builds, demo gate). New shots
  routes:
  - dossier with plan and `?path`;
  - /order Delivery, /orders, /orders/[id], /billing;
  - admin from build-admin: /dashboard, /review, /orders.

  GP1 (≥3 links with a slow index) and GP4 (aria-current) are part of the done criteria.
- Rust: crate tests plus clippy. Required cases:
  - `validate_plan` rejects a foreign id, an invented fee, an invented PO box, a URL and a bad
    hour;
  - office guards;
  - the order state machine;
  - Stripe signature, staleness, replay, amount and livemode;
  - the ledger never double-buys;
  - backing-submission FK inserts with `foreign_keys=ON`;
  - cross-customer authz;
  - USPS and FedEx shape tests against recordings;
  - the WS allowlist, `ops` authz, and the webhook as the only CSRF-exempt POST.
- AI: a live staging run on plant with the staging key (model, p50/p95, input/output/cached/
  reasoning tokens) and replay fixtures. Eval deltas: A5 drops by at most one gold case; meta-1's
  topic_accuracy is compared before and after; `eval --plans/--offices` report planted-fault catch
  rates. The model-gate stays green.
- Synthetic data: sandbox and TEM calls use only synthetic addresses. A recording scrubber removes
  account numbers, CRID/MID, tokens, tracking numbers and addresses before anything lands in
  fixtures, and secrets-scan has patterns for each.
- Commerce end-to-end on staging: Stripe test (via the reconciler) → FedEx sandbox plus USPS TEM
  labels → PDFs in the cabinet → WS timeline. On production: one capped real purchase per option.
- Each train is staging-green and no production row regresses. A non-gating
  `e2e/commerce.spec.mjs` take provides fallback footage.
- Proposed new scoreboard rows for the operator: a plan with a validator; the cabinet; trace
  latency.

## Cut lines

Protected core: B0-B3, M1, M2, M3, O1, F1, W2a, C1-C3, W3a/b, W1a/b.

Drop in this order:
1. Stretch items (public Ask, SAM 3.1 photo masking, es/fr plan translation).
2. W1d.
3. M6 Compile (manual fetch plus per-capture draft instead).
4. C6 poller (customer tracking plus carrier links instead).
5. F2 redraw.
6. M4 topic passes.
7. Paid USPS (free no-postage label plus FedEx remain).
8. Kit fulfilment UI (staff advance it via the order API).

## Top risks
- Builder throughput. Mitigations: self-gating lanes, delta merges, 4 trains.
- The shared CARGO_TARGET_DIR lock on a nearly full disk. Mitigation: per-crate tests only, and
  prune between trains.
- First live FedEx and USPS calls will need fixes. Mitigation: sandbox and TEM first, and the
  manual fallback queue.
- Viet Nam and Washington data depend on seeds, uploads and operator approval time (about 5 h
  across the Tue 10:00 and 18:00 blocks).
