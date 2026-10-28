# Prompt 054 — run to 100: Phase C implementation (§11 review queue)

- Prompt timestamp (UTC): 2026-10-28T22:18:51Z (same run directive —
  one phase per log entry)
- Response timestamp (UTC): 2026-10-28T23:07:10Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

Continued run-to-100 directive from prompt 051. Author approvals
received mid-run: §11 spec approved as-is (D40), both timers
enabled (D41), Gmail publish + verify (D42).

## Response

Phase C (implementation) done and green: the §11 review queue +
rejection-feedback suppression loop is built end to end — schema,
runner consultation, queue/rules API, and the `/review` admin UI.
Single work commit (`a123071`).

C1 — migration 011 + store helpers:

- `crates/handbook-store/migrations/011_review_queue.sql`:
  `suppress_rule`, `suppression`, fact citation offsets +
  `reasoning`.
- `crates/handbook-store/src/db.rs`: statement normalization
  (trim/collapse/lower), suppression check (path A live
  duplicate, path B rule match), insert paths; staff-quoted
  facts store uncited with empty reasoning
  (`crates/web/src/admin_voice.rs` call-site update).

C2 — summary field + runner consultation:

- `crates/contracts/src/model.rs`: per-response `summary` on
  `DraftResponse` (quarantined: served only as
  `reasoning_untrusted`, never a decision input).
- `crates/handbook-agent/src/model.rs` + `analyze.rs`: Meta
  client + stub return the summary; runner consults
  suppression before insert, stores citations/reasoning, and
  audits skips as `analyze.suppressed`.
- `xtask/src/main.rs`: `analyze` prints the suppressed count.

C3 — queue/rules API + tests:

- `crates/web/src/queue.rs` (new, wired in `main.rs` +
  `admin.rs`): `GET /admin/queue[/{id}]` (enriched: diff,
  citation quote sliced server-side, quarantined reasoning,
  blast radius, provenance), `GET/POST /admin/suppress-rules`,
  `POST .../revoke`, `GET /admin/suppressions`. Mutations run
  through `approval::decide()` (accept-with-edit text,
  reject-with-suppress rules); error vocab mirrors `approval`
  exactly (404/409/422 table).
- 9 router-level tests (oneshot through the real router).

C4 — `/review` admin UI (`bundled:taste` read first):

- `web/src/routes/(admin)/review/+page.svelte` (new, nav
  "Review" link in `(admin)/+layout.svelte`): fixed field order
  per §11.4 (header → diff → citations → quarantined reasoning
  → blast radius → provenance), word-level diff marks,
  expandable quotes, public conflict-banner reuse, trust chips
  (official/success, staff_verified/accent, community/grey),
  one-click Accept, Accept-with-edit (live diff preview, key
  omitted when unchanged), Reject (required reason + counter,
  collapsed suppress scope/lock_topic), 409 →
  "Already decided — refreshing" + refetch, decided one-liners,
  bookmarkable filter params, limit ≤200, load-more, "Queue
  clear" empty state, reversible rule create + revoke, and a
  read-only suppression-log table.
- `web/src/lib/admin/review-client.js` (new, calls-client
  conventions: JSDoc types, pure builders, CSRF double-submit
  on POST, 409/422 server-detail errors) + 7 node tests
  (`web/test/review-client.test.mjs`).

Decisions recorded this turn: §11 APPROVED as-is (D40) in
`docs/specs/analysis-agent.md`; both timers APPROVED for prod
(D41) in `docs/ops/apostille-watch/mail.timer` (+ services);
Gmail publish + verify once (D42) in `docs/gmail-shapes.md`.

Deviations from §11.4 (no endpoint exists; page stays honest):

- Citations show the sidecar path as text (no sidecar-fetch
  endpoint to link to).
- `open_tasks` links to the triage page (it takes no URL
  params to scope by authority).

## Verification

- `RUSTC_WRAPPER=sccache cargo xtask ci` → exit 0: fmt, clippy
  `-D warnings`, 578 passed / 0 failed (34 Rust suites), node
  85/85, `svelte-check` 0 errors, check:imports, both web
  builds, smoke OK, tailwind 266 tokens resolve.
- Fixes to reach green: fmt drift; clippy `then_some`,
  7× collapsible-if → let-chains, `needless_question_mark`,
  `manual_next_back`, revoke 8-tuple → scan into `RuleView`;
  svelte `}` + `Array.isArray` narrowing consts; blank-scope
  `ruleBody` handling.
- Bundle proof: `web/build-admin/_app/immutable/nodes/13.*.js`
  contains the Review queue + quarantine caption (ships in the
  admin build); API covered by 9 oneshot router tests.
- Staged-hook secrets-scan clean (commit accepted).

## Commits

- `a123071` Phase C: implement §11 review queue + suppression
  (C1–C4)
- (this log entry commits separately, per convention)

## Follow-ups

- Deploy train with migrations 009/010/011 + release notes
  (011 adds tables/columns — flag rollback hardness); enable
  both timers on prod; author enters `meta.api_key` + Gmail +
  Telnyx secrets via admin config UI.
- §11 OQ-1..5 still open (owner/SLO, exact-match scope,
  suppression retention, topic edits, summary field) though
  the spec is approved as-is.
- OPEN-32 (`docs/specs/p2-forms-labels.md` §9) staff label
  printing still awaiting author approval — not implemented.
