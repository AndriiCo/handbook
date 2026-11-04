# Application Feature Specs

- Status: partly built. F1–F13 were written for author review (prompt
  007); most ship today (see `README.md` § Promised → delivered). The
  text below is the feature record, not a to-do list.
- Date: 2026-10-26
- Source: parallel researcher draft (recovered from workflow delivery);
  F7 updated by parent per the verified Egypt-chain research
  ([egypt-consular.md](egypt-consular.md)) and the printpdf decision
  ([p2-forms-labels.md](p2-forms-labels.md) P2-R2, verified against
  docs.rs printpdf 0.12.8, pure Rust, MIT).
- Companion specs: [spa-separation.md](spa-separation.md),
  [analysis-agent.md](analysis-agent.md),
  [p2-forms-labels.md](p2-forms-labels.md),
  [user-journeys.md](user-journeys.md),
  [egypt-consular.md](egypt-consular.md).

Scope: 106 apostille authorities in USA/Canada/Mexico first; every
country later. Repo holds code+schema only, never customer data (design
§0). Stack: Rust/axum+tower-http API, SvelteKit SPA, WS-first realtime,
single Ubuntu 26.04 server under `andrii.ai/apostille`, Mac local dev,
no Docker, pure-Rust deps, `cargo xtask ci` gate (web-stack §1–2).
Evidence store: one SQLite file + `store/` (WARC/mbox/blob/text
sidecars/FTS5), trust tiers, audit+build rows with two commit hashes
(design §1–6). P1: transcripts feed handbook pages + live call console.
P3 (nice-to-have): live translation via SeamlessStreaming. Bulk
raw-data analysis is done by the implemented Model-API agent — see
[analysis-agent.md](analysis-agent.md) — not in chat. Conventions
below: each feature has Goal / Acceptance criteria / Data+API needs /
Dependencies. `xtask` = Rust task runner, no shell scripts.

## F1 — Cited-URL watch (500+ URLs)

Goal: detect quiet changes on every URL the handbook cites, so no page
plans a visit around stale facts.
Acceptance: (1) every cited URL is a
`source(kind=website_official|website_thirdparty)` row with owning
`authority_id`, check cadence, and last verdict; (2) `xtask fetch-url`
re-fetches per cadence, robots-aware for third-party (`source.robots`
re-checked each crawl), stores one `.warc.gz` + TLS evidence in
`warcinfo` (chain PEM, root-bundle version, IP, time) and extracted
`text/<id>.txt` indexed in `capture_fts`; (3) byte-identical re-fetches
create no new work (sha256 dedup); changed fetches create one
`capture(kind=warc)` + diff pointer and enqueue an F4 change record;
(4) fetch failures (DNS/TLS/HTTP error) create/refresh a
`task(kind=validate_fact)` instead of silently skipping; (5) full
500-URL sweep completes on one modest server in a bounded window with
per-domain politeness (rate limit + UA + retry/backoff).
Data/API: `authority, source, domain, capture, capture_fts, task,
audit`. File store `warc/`, `text/`. No Meta API needed for fetch
itself.
Dependencies: domain table seeded with official vs known-third-party vs
unverified (F-story anti-spoofing); design §2–3 implemented first.

## F2 — HCCH feed watch

Goal: catch Apostille Convention membership/effective-date changes (the
Viet Nam 2026-09-11 class of error) within one watch cycle.
Acceptance: (1) HCCH status source(s) tracked as `source` rows with
documented poll cadence; (2) each poll stores a `capture` (WARC or
file) + text sidecar; (3) effective-date/member changes produce F4
change records flagged `priority=high` and auto-create a `task`
against every affected handbook page; (4) a regression test asserts
the Viet Nam case shape: status row with effective date → derived
routing fact (apostille vs consular legalization) flips accordingly;
(5) missed/failed polls alert, never silently pass.
Data/API: same tables as F1 plus `fact(topic=status|routing)`. HCCH URI
list is data (seed), not code.
Dependencies: F1 fetch pipeline; F4 record pipeline; F11 publish path
for the routing flip.

## F3 — Authority-notice inbox

Goal: ingest official notices (email/portal announcements, e.g. office
moves, fee changes, superseded forms) with authenticity evidence.
Acceptance: (1) inbound mail appended to per-source per-month mbox
(`mbox/<source_id>/YYYY-MM`, Brotli once sealed), one
`capture(kind=email)` per message with `authenticity` JSON (SPF, DKIM
result+selectors, DMARC verdict) verified at ingest via Rust mail-auth;
failures keep mail but cap trust at `community`; (2) sender domain
resolved through `domain` table; `official` requires staff evidence
capture; new senders default `unverified`; confirmed spoofs → `fake`,
captures capped at `rejected`; (3) each actionable notice yields an F4
change record citing the mbox capture; (4) portal-only notices (no
email) entered via F1 URL watch, not ad-hoc paste, so provenance stays
uniform.
Data/API: `source(kind=email_official), domain, capture,
fact(proposed), task, audit`. Readers handle plain and `.br` mbox
transparently.
Dependencies: F1/F4; domain allowlist workflow; retention policy for
mail (open question).

## F4 — Cited change records + author approval (Muse Spark)

Goal: the author approves small cited records, never raw diffs; only
approved records reach the handbook.
Acceptance: (1) every detected change becomes ≥1
`fact(status=proposed)` row citing its `capture_id`, with `topic`
(address|hours|fee|form|status|routing|…), one short `statement`,
`created_by=model:<version>`; (2) approval UI (SvelteKit + axum
JSON/WS) lists proposed records grouped by authority with side-by-side
cited evidence, accept/reject with reason; accept → `status=accepted`,
reject → `rejected`, replaced → `superseded`; every transition writes
an append-only `audit` row (actor, before/after refs, both commit
hashes, reason); (3) no auto-publish path exists: community/model
input reaches usable state only via staff approval or
two-independent-capture corroboration + audit row (design §3); (4) WS
`authority:{slug}` topic pushes record updates; concurrent-edit banner
(last-write-wins + presence, no CRDT); (5) the
[analysis agent](analysis-agent.md) does bulk EN/ES/FR diff→record
drafting via Model APIs; chat carries at most one tiny worked example.
Data/API: axum routes for proposed/accept/reject; `fact, capture,
audit, capture_fts`; WS broadcast per web-stack §3. Model API
endpoint/auth from spec (open).
Dependencies: F1–F3 capture pipelines; F6 conflict view (approver must
see contradictions); F11 build consumes only `accepted`.

## F5 — EN/ES/FR reading (pages and forms)

Goal: read authority pages and forms in English, Spanish, French well
enough to draft accurate cited records.
Acceptance: (1) text extraction covers HTML + PDF/forms for all three
languages into `text/<id>.txt` sidecars; (2) record drafting
prompts/templates are language-aware and cite exact source spans (see
[analysis-agent.md](analysis-agent.md) §5); (3) acceptance sampled per
language: ≥1 English, ≥1 Spanish, ≥1 French authority page/form
end-to-end (fetch → extract → proposed record → approval) green in
CI-seeded synthetic fixtures; (4) unsupported-language sources are
marked, not misread (task opened, no fabricated record).
Data/API: extraction helpers in `crates/handbook`; fixtures under
synthetic `cargo xtask up` seed (never committed real data).
Dependencies: F1/F4; Meta Models API text/models availability.

## F6 — Contradiction flagging (e.g. Washington's two PO Boxes)

Goal: finding contradictions is one query + one command, surfaced
where the author approves.
Acceptance: (1) conflict = same `authority_id` + same `topic`, two
`accepted` facts with different `statement` (design §7 query) plus FTS
keyword search across `capture_fts`; (2) `xtask review-conflicts`
prints each conflict with cited captures; (3) approval UI shows a
conflict banner on affected records/authorities with links to both
citations; (4) resolving a conflict = accept one / supersede or reject
the other + audit row; conflicts never auto-resolve; (5) pilot proof:
Washington-state fixture with two PO Box statements flags exactly one
conflict and clears after scripted resolution.
Data/API: `fact, capture, capture_fts, audit`; read-only queries +
xtask.
Dependencies: F4 (statuses must exist); FTS sidecars populated.

## F7 — Egypt-in-USA consular legalization chain + pre-filled authority forms

Goal: one complete non-apostille chain (Egypt in USA) proving the
model handles multi-step legalization, ending in correct pre-filled
forms. Chain research: [egypt-consular.md](egypt-consular.md) —
verified steps below supersede the earlier sketch.

Acceptance: (1) chain steps modeled as data (authorities + order +
per-step fee/address/form/hours facts), not hardcoded flow. Verified
skeleton: notary (on-document seal; county-clerk pre-step where
required — NY confirmed, other states surveyed) → Secretary of State
**Certificate of Authentication, never an apostille** ("Apostilles are
not accepted" — embassy's own rule) → US Dept of State Office of
Authentications, **conditional** (mandatory for commercial, court, and
marriage/divorce instruments; personal-doc need is validation task
V-6) → Egypt embassy/consulate legalization routed by **issuing
state** (DC, NY, Chicago, LA, Houston jurisdictions as researched).
Every step value stays a validation task until its citation is a
staff-verified capture; (2) each step page cites its captures; chain
view shows order, cost sum, and addresses; (3) pre-fill: customer
details entered once generate per-authority completed forms rendered
server-side with **printpdf** (pure Rust; clean-room rect layout per
[p2-forms-labels.md](p2-forms-labels.md) P2-R2) with field mapping
stored as data; unmapped fields left blank and flagged, never guessed;
(4) synthetic-document demo path: request → forms + prepaid-label
story validated with fake data (no real PII) — with the researched
constraint that the **consulate mail hop is USPS-only (FedEx/UPS
refused)**, so prepaid labels for that hop must be USPS; (5) chain
breakage (a step's source changes) raises an F4 record + task on that
step only.
Data/API: `authority(kind=office|country), source,
capture(file|scan), fact(topic=fee|address|form|hours|step), task`;
form templates as versioned files + field-map data (see P2 spec).
Dependencies: F1–F6 pipelines; content-repo home for form templates
(open); validation tasks V-1…V-19 resolved per chain step before demo.

## F8 — Walk-in reports (route photos, parking tips, rejections)

Goal: visitor reports (photos, route/parking tips, rejection stories)
improve visit instructions after moderation.
Acceptance: (1) submission path (SvelteKit form + axum upload) creates
`capture(kind=photo|video|comment)` with `trust=community`,
quarantined by default, EXIF preserved in original; upload requires
accepting the rights grant (instruction photos become Andrii Co.
property — decided prompt 008; grant version + timestamp recorded on
the capture, refusal blocks upload); (2) report fields:
authority/office, visited-at, route/parking text, rejection reason if
any; each report links to its captures; (3) moderation = F4 approval:
promote only via staff approval or two-independent-capture
corroboration + audit row; promoted claims become `proposed→accepted`
facts (topic `visit|parking|hours|…`); (4) derivatives (thumbnails,
F9-redacted copies) generated on demand by xtask and cached; originals
immutable; (5) abuse path: reject + audit, reporter never auto-trusted.
Data/API: `capture(blob store), fact, task(kind=correction|
validate_fact), audit`; upload size/type limits; WS update on
moderation decision.
Dependencies: F4 approval UI; F9 masking before any public render;
photo retention decision (open — only voice 365-day decided).

## F9 — SAM 3.1 face/plate masking

Goal: no visitor photo/video is shown publicly with identifiable faces
or plates.
Acceptance: (1) every public render of a community photo/video uses a
redacted derivative; originals never served to anonymous clients; (2)
redaction runs through **SAM 3.1 on the Model API** with text prompts
("face", "license plate"), usable by **both the uploading customer
(pre-publish self-service) and admin moderation**, producing cached
derivatives with EXIF stripped (decided prompt 008); recall verified
on fixtures, self-hosted detector as fallback; (3) QA: synthetic
fixtures with faces/plates assert redaction applied; unmaskable media
stays private with a task, never published raw; (4) pipeline is
reproducible: derivative records input sha + model version in
notes/audit.
Data/API: blob store + derivative cache; Meta Model API is an
approved processor for redaction media (read its data-handling terms
before real media flows); no other third-party upload of customer
media.
Dependencies: F8 upload path (with rights grant); API latency/cost
budget for SAM calls.

## F10 — Self-hosted Muse Glimmer document-photo pre-check

Goal: catch bad document photos (blur, glare, crop, unreadable) before
the customer mails anything.
Acceptance: (1) upload returns a pre-check verdict (pass /
retake-with-reasons) plus per-issue hints (e.g. glare top-right,
cut-off bottom edge); (2) verdicts are advisory only: they never
create handbook facts and never auto-reject an order; (3) all
inference self-hosted (no customer pixels to third parties); model
version recorded with each verdict; (4) synthetic fixtures
(good/blurry/glare/cropped) assert expected verdicts in CI; (5)
latency budget documented and met on the single server (fallback:
async job + WS notify).
Data/API: axum upload + job endpoint; WS per-upload notify topic;
verdict log table or capture notes (decide at scaffolding; must not
leak PII into repo).
Dependencies: F8 upload plumbing; Glimmer self-host packaging +
hardware budget; privacy review (security lane).

## F11 — Approved-changes publish (handbook pages + build manifest)

Goal: only approved records reach customers, with full traceability.
Acceptance: (1) `xtask build-handbook` regenerates pages from
`accepted` facts only; each page links citations to captures; the same
command also emits `sitemap.xml` + per-country shards (see
[site-theme.md](../site-theme.md) §7.2 — no separate sitemap tool);
(2) every build writes a `build` row (timestamp, `code_git_commit` +
`content_git_commit`, monotonic `build_no`, `facts_hash` over accepted
inputs); every content-affecting change writes an `audit` row; (3)
conflicts (F6) visibly banner the affected page — never
publish two contradictory `accepted` statements silently (decided
prompt 011: banner, not block); (4) publish is atomic per
build (no half-updated site); rollback = new build from earlier
accepted set + audit reason; (5) sub-path serving `/apostille` works
locally and in prod; static caching rules per web-stack §2 (hashed
immutable assets, no-cache HTML, 304s, Brotli).
Data/API: `fact, build, audit`; content repo (location TBD) or interim
content dir with hash discipline.
Dependencies: F4/F6; content-repo decision; SvelteKit build + axum
static serving.

### F11 Serving — public pages read live accepted facts (Slice A, D36)

Decided (O1, author 2026-10-28): the Svelte public SPA + live
API serves handbook pages, not the static publish HTML. Static
builds stay audit/accept artifacts (`build` rows + `facts_hash`
+ `xtask accept` gate unchanged); what customers see is the
currently accepted set. Build-pinned reads (serve facts as of
`build_no`) are a named follow-up, not v1.

- `GET /api/v1/authorities` (new, public router, anonymous):
  `{"authorities": [{"slug", "name", "country", "fact_count"}]}`
  from `authority WHERE id != 0 ORDER BY slug` (the id-0
  sentinel backs unattributed facts and is never listed);
  `fact_count` = accepted facts per authority, one query
  (LEFT JOIN + GROUP BY). No pagination in v1 (dozens of
  rows). Store failure → the standard `store unavailable`
  shape. No auth, no rate limit beyond the shared public
  posture (same as the per-slug route).
- `GET /api/v1/authorities/{slug}/facts` (existing): unchanged
  wire shape (`{"authority", "facts": [FactView...]}`, 404
  `unknown authority`); the SPA is its first UI caller.
- SPA contract: search filters the live index client-side
  (name/slug/country substring); dossier groups accepted facts
  by topic; `conflict: true` facts render the Notice banner
  with sibling statements (F11(3) on the live site at last);
  honest loading/empty/404 states throughout; the placeholder
  authority array is deleted. Public code stays inside
  `lib/public-ui` (import gate); no prices/dates rendered
  (unchanged rule).
- Empty prod is a valid state: zero authorities → search empty
  state; zero facts → dossier empty tabs (not errors).
- O3 (audit checkpoints, staff on demand — no daemon, no served
  `out_dir`): `cargo xtask build-handbook --out <dir>`, recorded by the deploy's
  accept gate (`xtask deploy --expect-build`); builds stay under the given dir for
  audit/hash verification only.
- Out of slice: FTS search, pagination, build-pinned reads,
  per-fact permalinks, ES body content.

## F12 — MCP server (approved content only)

Goal: AI assistants (Claude/ChatGPT-class) can answer apostille
questions from the same approved facts via MCP.
Acceptance: (1) read-only tools over accepted facts/builds (e.g.
lookup authority, get visit steps, get form/fee/hours, cite sources);
every answer carries citations + `build_no`; (2) no
proposed/community/rejected content exposed; no PII, no uploads, no
payments through MCP; bookings-style side effects explicitly out of
scope (requests-not-payments pattern from prior art if ever extended);
(3) versioned API; stale-client behavior documented; (4) auth story
decided (public read vs keyed) before exposure; rate limits; (5)
conformance: scripted MCP client queries against seeded store assert
citation+build fields present.
Data/API: axum-hosted MCP endpoint (Streamable HTTP if following prior
MCP server) or separate binary reusing `crates/handbook`; same SQLite
read path.
Dependencies: F11 build discipline; security review of tool surface;
spec for tool list.

## F13 — New-country onboarding = new sources, not new code

Goal: adding a country is a data task (seed sources, domains, chain
steps), provable without a code deploy.
Acceptance: (1) documented onboarding checklist: add `authority` rows,
`source` rows (URLs, inbox senders, HCCH mapping), `domain` verdicts
with evidence captures, chain-step facts, language tag; (2) proof
drill: onboard one out-of-scope country in a staging store with zero
code changes and show fetch → record → approve → build → MCP answer
working; (3) schema guards: `authority.country` free text; no
country-code enums in code paths; (4) onboarding validation xtask
(missing source/domain/chain detection) green on USA/Canada/Mexico
seeds.
Data/API: seed files + validation xtask; same tables, new rows.
Dependencies: F1–F6, F11–F12 complete; content-repo workflow supports
country packs.

## Cross-cutting: synthetic Viet Nam demo (success criteria)

A customer can request a Viet Nam apostille online conveniently;
envelopes with the completed form ship via FedEx next day — all with
synthetic documents. Acceptance: scripted demo uses seeded Viet Nam
routing fact (apostille, effective 2026-09-11, not consular
legalization), generates the form + label story, and asserts the
2026-10-13 misroute class cannot recur (routing derives from watched
status fact, covered by F2 regression test). This section owns the
single Viet Nam demo script; no duplicate demo scripts elsewhere.

## Tiny worked example (the one allowed)

Washington fixtures: `fact#1 (authority=wa-sos, topic=po_box,
statement="PO Box 40220", accepted, capture#11)` and `fact#2 (same
authority+topic, statement="PO Box 40234", accepted, capture#12)` → F6
query returns 1 conflict; `review-conflicts` prints both citations;
approver rejects #1 with reason → audit row; page rebuild shows only
#2. All bulk analysis follows this shape via the Model-API agent.
Canonical fixture values are `40220`/`40234` everywhere — never invent
other PO Box values in fixtures or tests.

## Open questions (from this draft)

- ~~Meta Models API spec~~ — decided prompt 008 (see
  [../meta-api.md](../meta-api.md)); Seamless absent from the API,
  P3 stays conditional. Residual: exact JSON shapes, data-handling
  terms.
- Content repo location + form-template home; photo/mail retention
  beyond voice 365-day.
- HCCH feed exact URIs + poll cadence; 500-URL seed list + politeness
  budget.
- Egypt chain exact steps/fees/forms (see validation tasks V-1…V-19 in
  [egypt-consular.md](egypt-consular.md)); FedEx label provider vs
  demo stub.
- SAM 3.1 + Glimmer self-host packaging, weights licensing,
  single-server latency budget.
- MCP tool list + auth (public vs keyed) + rate limits; conflict
  block-vs-banner policy.
