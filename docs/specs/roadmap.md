# Roadmap: 5–7-Day Swarm (decided direction)

- Status: plan (prompt 011 deep-research replan, adjusted for author
  answers: conflicts = banner, secrets = TPM-sealed now, labels =
  carrier API buy). Supersedes the 10-day plan formerly in
  [app-features.md](app-features.md).
- Date: 2026-10-26. F1–F13 in [app-features.md](app-features.md)
  remain the feature specs; this doc decides what ships in which
  week.
- Protected promises: (1) Viet Nam synthetic demo, (2) one closed
  loop watch → cited record → approve → publish.

## 1. Ranked slice (impact order)

Impact = demo-loop survival first, then publish trust, then
order-path story, then deploy proof.

1. **Store hardening (F1–F5).** Deps: none. External: none.
   MUST Day 1 — txn sweep + `saturating_sub`, traversal guard,
   hex-validated `blob_path → Result`, temp+rename write,
   CHECKs + FKs + PRAGMAs (WAL, busy_timeout). No network code
   before this.
2. **Web scaffold + demo session + WS bus.** Deps: #1. External:
   none. MUST Day 1–2 — axum `/apostille` base, health/static/
   robots, WS topics `authority`/`call`/`submission`/`shipment`/
   `billing`, single demo session (full auth playbook deferred;
   no real PII flows in the slice).
3. **Topic-enum freeze + fixtures.** Deps: SPEC decision Day 2.
   External: none (proposed default: `po_box` is its own topic;
   author may override). MUST Day 1–2 — `topics.rs` + TOPICS.md;
   canonical 40220/40234 fixtures never leak into facts.
4. **Thin watch (fetch-url + ONE HCCH URI + F2 regression).** Deps:
   #1, #3. External: 1 HCCH URI + 10-URL seed (author/SPEC; uber
   fallback: synthetic). MUST Day 2 — fetch + sha256 dedup +
   sidecar + FTS; WARC plain-blob first; full 500-URL crawl and
   F3 inbox Week 3.
5. **Analysis agent (`xtask analyze` + Stub + Meta client).** Deps:
   #3, #4. External: `META_API_KEY` (fast); JSON shapes pinned at
   implementation. MUST Day 2 — trait, watermark runner,
   byte-exact citation validator, StubClient green in CI; EN-only
   + unknown-skip; no real media before terms read.
6. **Approval API + conflicts.** Deps: #2, #5. External: none
   (policy decided: **banner**). MUST Day 3 — list/accept/reject
   + audit, `review-conflicts` CLI, conflict banner in payloads
   and on pages.
7. **Publish (`build-handbook`: accepted-only + atomic swap +
   build/facts_hash rows).** Deps: #6. External: content home
   (proposed default: gitignored `store/forms/` fallback with
   sha-pinned manifest unless author names the repo). MUST Day 4
   — WA page + Viet Nam banner; sitemap/robots Day 7 if time.
8. **Public SvelteKit shell.** Deps: #2, #7. External: none. MUST
   Day 4 — home search + authority page, themed, EN-only; ES/FR
   and JS-budget gate Week 3+.
9. **P2 synthetic order path.** Deps: #2. External: carrier
   sandbox accounts (parallel track from Day 1). MUST Day 5 —
   1 template + printpdf PDF (wet-ink-only) + label payloads +
   WS events; **carrier-API buy code** behind the same payload
   schema, with staff-manual paste-back as automatic fallback
   until certification lands; USPS-only Egypt-hop constraint in
   copy. No prices/dates in UI (pricing table still open).
10. **Viet Nam demo script + MCP stub.** Deps: #4–#9. External:
    none. MUST Day 6 — frozen WA → Viet Nam → banner → form →
    PDF → label-story → misroute-regression; ONE read-only tool
    `lookup_authority` (citations + `build_no`), localhost-only;
    full MCP + F13 drill Week 3+.
11. **TLS deploy + parity + drills.** Deps: all above. External:
    hosting IP/SSH/DNS (author owes). MUST Day 7 — Caddy + TLS
    at andrii.ai/apostille, Ubuntu 26.04 parity, 10-URL load
    check, drill ×3.

## 2. Cuts and defers (explicit)

- **Call console live realtime → Week 4** (most visible P1 cut —
  say so in the demo); keep file-upload transcript → proposed
  facts only.
- **F8/F9/F10 community + SAM + Glimmer → Weeks 4–5**; fixtures
  stand in. (SAM-via-API redaction design stands, just unscheduled.)
- **F7 Egypt validation → skeleton** (`needs_review` per step +
  open tasks); V-1…V-19 validation runs Weeks 3–6 on an author
  parallel track. Never present unverified fees/addresses.
- **Admin split + mTLS → Week 4** (interim Caddy/axum refusal now;
  no real PII in the slice, so acceptable). **TPM-sealed config
  → in-slice per author decision** (secure-config.md builds now;
  env-only is the fallback only if the box lacks TPM2).
- **Money + i18n + legal-gated copy → deferred**: no Stripe
  charging (invoice mirror later), EN + pending-translation
  notices, legal pass non-blocking for code.
- **F3 inbox, full crawl, F13 drill → Week 3+.**

## 3. Swarm streams and file ownership (single-writer)

4–5 CODE agents; each owns disjoint files; shared-file change =
one named owner per day.

- **S1 Foundation**: `migrations/002_*.sql`, `db.rs`, `store.rs`,
  `topics.rs` + TOPICS.md. Day 1 item 1; Day 2 enum freeze.
- **S2 Agent** (new files only): `model.rs` (trait + MetaRest +
  Stub), `analyze.rs`, `xtask analyze`, `prompts/aa-prompt-v1.md`.
  Never edits `db.rs`/`store.rs`.
- **S3 Web API**: new `crates/web/**` (router, demo session,
  CSRF+Origin, WS bus, static). Mounts `p2.rs` owned by S6.
- **S4 Public SPA**: `routes/(public)/**`, `lib/public-ui/**`,
  theme/i18n shells. Owns `svelte.config`/`package.json` Day 2
  only; never imports customer/admin libs.
- **S5 Customer/admin/console**: `routes/(customer)/**`, admin
  routes, `lib/customer`, `lib/admin`, `ws-client.js`. Touches
  no `(public)` file.
- **S6 Egypt + P2**: new `forms.rs`, `migrations/003_p2.sql`,
  `store/forms/` fallback, `crates/web/src/p2.rs`. Chain-as-data
  skeleton, printpdf + wet-ink, carrier-API client + manual
  fallback.
- **S7 Publish + MCP**: `xtask build-handbook` (+sitemap),
  MCP stub tool. Day 4–6 only.
- **H Deploy + secure-config**: Caddy, DNS/TLS, TPM verify +
  `config` tables/envelope, backup note. Independent; runs
  parallel from Day 1 once hosting lands.

Lanes: SPEC (Day 1–2 deltas, then spec-before-surface only);
CODE (every merge behind `cargo xtask ci`); SEC (Day 2
trust/WS-authz, Day 4 consent/PII/uploads, Day 5 MCP/secrets;
blocks merges); TEST (fixtures, daily red/green, drills Day 4/6,
Ubuntu parity Day 6).

## 4. Day gates

- **Day 1**: item 1 fixes; trait + Stub; scaffold health/static/
  shell; enum draft. EOD: `ci` green, dev serves `/apostille`.
  - DONE 2026-10-26 night run (prompt 012): F1–F7 + 005 CHECKs/FKs,
    trait + Stub + validator + runner + `xtask analyze`, axum
    scaffold + demo sessions + SPA shell serving, enum frozen
    (TOPICS.md), SEC/TEST review lanes + fixes. 55 tests, `ci`
    green, shell/healthz/login/me smoke-verified live.
- **Day 2 GATE (watch→approve)**: changed URL → capture →
  analyze → proposed → approval → banner on fixtures; enum
  frozen; WS live in two windows.
- **Day 3**: approval API + conflicts, file-transcript ingest,
  P2 registry, Egypt skeleton view.
- **Day 4 GATE (chain+forms+console)**: synthetic Egypt drill
  (SoS Authentication ≠ apostille), prefilled stamped PDF,
  transcript → facts, 365-day sweep.
- **Day 5**: carrier-API client + manual fallback, tracking
  paste-back, publish + MCP draft, banner/API hardening.
- **Day 6 GATE (ship)**: atomic publish + audit/build rows, MCP
  conformance, Viet Nam script green, Ubuntu parity, `ci` green
  both hosts.
- **Day 7 buffer**: overflow + harden only (rate limits,
  headers, backup note, sitemap/robots) — no new scope.

## 5. External dependency register (owner + needed-by)

- `META_API_KEY` — author, Day 2 (StubClient covers dev/CI).
- Hosting IP/SSH user + DNS + TPM2/systemd answers — author,
  Day 1 (blocks H stream and Day 7 gate).
- 1 HCCH URI + 10-URL seed — author/SPEC, Day 2 (synthetic
  fallback exists).
- USPS (COP account + Ship/EPA + Labels approval) + FedEx
  (developer project + sandbox; label cert) — author + S6,
  Day 1 start (certification is the long pole; manual fallback
  covers the demo).
- Meta API data-handling terms read — SEC, before any real
  media (slice is synthetic-only until then).
- Content-repo home — author, Day 3 (fallback: `store/forms/`
  + sha manifest).
- PII store + retention values — author, Day 3 (proposed
  default: separate `p2.sqlite`; submitted 2y, drafts 90d,
  PNGs 1y). Gates P2 schema finalization.

## 6. Gaps ledger (from deep-research; details in reports)

Tier 0 (block deploy): hosting, content home, F1–F5, AGENTS §6
approval. Tier 1 (demo/revenue paths): label model (decided:
API-buy + fallback), pricing blanks (→ no prices in UI),
payments untouched (deferred), PII store (proposed default),
demo slice (USPS-or-both clickable: proposed both-behind-flags,
fallback USPS-only). Tier 2 (launch/auth/i18n): staff login
(deferred to real auth), timeouts (reconciled), P2-auth seam
(resolved), legal (parallel track), ES scope (Week 3+).
Tier 3 (vague acceptance): F1/F2 quantifiers, agent freeze
rules, conflict policy (decided: banner), Egypt V-tasks
(author track), SAM/Glimmer (cut), MCP surface (stub),
F13 (Week 3+), console UX (thin slice). Tier 4 (web/ops):
two-build keys (verify Day 2), secure-config Qs (in-slice),
WS baselines (S3+SEC), drafts/staleness/fonts (defer), DR/
budgets (Day 7 note).

## 7. Remaining author inputs (proposed defaults apply on silence)

- AGENTS.md §6 approval (gates lane enforcement).
- Topic enum: `po_box` own topic (proposed; override anytime).
- Content home: `store/forms/` + sha manifest (proposed).
- PII store + retention: separate `p2.sqlite` + 2y/90d/1y
  (proposed).
- Hosting + TPM answers (no default possible — owed).
- Carrier accounts started Day 1 (author action).
- Pricing table (deferred — no prices render until supplied).

## 8. Assumptions audit (Draft — grill, night of 2026-10-26 PDT)

Pinned this session (event page + repo probes):
- Deadline: Wed Nov 4, 9PM ET = 6PM PT. ~8.75 days left.
- Startup track: "technical execution and whether it holds up
  in production." No finer criteria published.
- Day 1 (Oct 26) DONE; Day 2 = Oct 27; Day 7 buffer = Nov 1;
  Nov 2–4 spare. Schedule fits IF scope holds AND author
  inputs land on time.
- mTLS for admin pulled from Week 4 into now (author
  decision; plan revision pending). Grill challenges its
  scheduling, not the decision.
- No TPM on enodia → env-fallback active (probed).

Assumptions under challenge (author: correct if wrong):
- A1: `META_API_KEY` + terms read land Day 2 (Oct 27).
- A2: 10-URL seed + 1 HCCH URI land Day 2.
- A3: carrier accounts started Day 1 (status unknown).
- A4: staging box ETA unknown; day-10 prod-direct cutoff
  still stands.
- A5: author hours/day for approvals + root windows unstated.
- A6: submission shape (repo + demo + writeup?) not on the
  event page — format assumed, not verified.

Interview state:
- Asked: Q2 SAM funding + tripwire.
- Queued: transcription path, admin-vs-loop ordering,
  staging/acceptance, restated success criteria wording.
- Settled: D1 (Q1 = B) — SAM-via-API redaction is
  must-have in-slice, alongside the decided carrier-buy
  code; see specs/README.md G1 (Draft).
- Doc debt: prompt-log entries pending for the memory /
  admin-plan / mTLS turns (numbering at interview end).
