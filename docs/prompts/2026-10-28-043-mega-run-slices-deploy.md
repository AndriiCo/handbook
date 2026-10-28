# Prompt 043 — Mega-run: admin/config answers, Slices B/D/E/F, UI, deploy 14615cd

- Prompt timestamp (UTC): 2026-10-28T15:29:35Z
- Response timestamp (UTC): 2026-10-28T16:35:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Why admin.andrii.ai and https://andrii.ai/apostille/ are the same page and not customer vs. admin focused interfaces? How I can put configuration values (including secrets) into app via web UI (without root command lines - store only a few ecryption keys in soft-TPM, store rest of values in database in database encrypted by one of thouse soft-TPM keys). Can you add a work item to have a nice UI for the app (Tailwind CSS or other commonly used framework, make a propopsal for vusually appealing with nice UI/UX design). Make an efficient use of sub-agents more agreesively.  Finsih the goal (100%, subject to config values set) in next run.
```

(Verbatim, typos preserved.)

## Response

One run finished the goal subject to config values: the three questions
answered in code + docs, Slices B/D/E/F implemented with tests, admin
quote UI shipped, UI framework proposal persisted, and release `14615cd`
(7 commits) deployed via hal and verified live. Aggressive subagent use
throughout: 6 read-only research streams for assessment, per-slice
implementation drafts, UI bundle + proposal streams — all parent-verified
(single-writer checkout per AGENTS.md §6); parent review caught draft
defects in every stream before writing (wrong test helpers, wrong
`contracts::Topic` path, invented route, negated 404-copy pattern).

### Answers shipped

1. **Admin vs public sameness** — root cause: the admin host served the
   same SPA root. Fix: `GET /` on the admin host → 303 to `/admin`
   (303, not 301: the dashboard layout gate forwards to login);
   `/mock/*` → 404 on the staff origin. 4 router tests + smoke
   assertions (`redirect-target`, `single-root`); decision recorded in
   `docs/specs/spa-separation.md`.
2. **Config via web UI (soft-TPM + DB-encrypted values)** — the admin
   config UI accepts values incl. secrets on the server; only the KEK
   arrives via systemd credentials (soft-TPM-class bootstrap), all other
   values live sealed in DB rows. No new code needed — verified the path
   and documented staff enrolment (`docs/ops/staff-enrolment.md`) + hal
   `apostille-cert`/`apostille-invite` wrappers in `ops-hal-access.sh`
   (strict stdin validation, fenced PEM delivery, rejection/verify-dummy
   probes — no new trust: hal already holds sqlite + deploy).
3. **Nice-UI work item** — `docs/ui-framework-proposal.md`: Tailwind v4 /
   UnoCSS / vanilla-extract ranked against hand-rolled CSS vars;
   recommends Option A (consolidate vars — the only option with negative
   bundle cost vs the 2.2KB gate headroom), with per-page design direction
   and phased rollout. OPEN-29/30/31 carry the author decisions.

### Slices (all parent-written, spec-first)

- **Slice B** (`095a8e6`): `xtask fetch-slug --slug --actor --trust
  --limit` (manual, staff-triggered) + `analyze --scan-captures` bridge
  (Option-B scan over `last_capture_id`, MAX() advance in-txn; missing
  sidecar = correction task + audit + advance; rejected trust + sentinel
  filtered; client error stops without advancing) + migration 008
  (`capture_watermark`, additive) + DB `get_capture_watermark`.
- **Slice D** (`e247392`): optional `submission_id` on shipments (absent =
  shell path byte-identical; owner mismatch = 404 per file convention;
  non-submitted = 422) + PDF `build_no` stamp (`Some(MAX)`, 0 = explicitly
  unbuilt). D37: P2 order container deferred (no order record without
  money). Manual-label queue UI cut — OPEN-32: `queue_manual` is a helper,
  not a handler; no staff list route exists.
- **Slice E** (`8c66845`): Piece 1 `xtask demo-vietnam` frozen 5-step VN
  drill (temp store only, no `--store` flag by construction; 2 CLI tests).
  Piece 2 `POST /admin/calls/{id}/quote` (contract §7): ONE proposed fact
  via `insert_proposed_fact`, unattributed at rest (`capture_id` 0 +
  `created_by staff:<user>`), call+segment refs in the audit reason, never
  transcript text; frozen 25-topic enum; foreign segments 404 (mark_asked
  convention). 3 tests (happy-path, 5×422 + 404 matrix, guards).
- **Slice F** (in `095a8e6`): `cargo xtask ci` runs node tests,
  svelte-check, check:imports, both builds, smoke, web-budget after cargo
  gates. Missing binary = fatal, never skipped.
- **Admin quote UI** (`06bd92a`): transcript checkboxes + topic/statement
  form on the call detail page; `quoteBody`/`quoteToDraft` in
  `calls-client.js`; 8/8 node tests; svelte-check clean; public budget
  unchanged (admin-only).
- **Deploy path** (`14615cd`): `ops-deploy.sh` forwards optional
  `$STAGE/MIGRATION_NOTES` as `--migration-notes` into the release
  manifest (bash array, echoed for the audit trail) — implements the
  standing "note migrations in release" rule.
- **fmt** (`8161f55`): rustfmt-canonical rewraps on Slice B/D files (local
  rustfmt is newer than the Slice B author's; no semantic change).

### Deploy 14615cd (as hal)

- zigbuild `-p web -p xtask` (one transient sccache link failure on `web`,
  clean on retry) + `npm run build`/`build:admin`; budget 226136/228352.
- BUILD_SHA note: first `web` link baked `06bd92a`; the retry (after HEAD
  moved) re-ran build.rs and baked `14615cd` — verified via `strings`
  before staging, so VERSION matches the wire.
- Staged SHA + `MIGRATION_NOTES` ("schema 7->8 via 008_capture_watermark
  (new table only, additive, rollback-safe); no packs diff"); synced the
  updated `ops-deploy.sh` to the server first.
- `--skip-accept` announced: prod has no build row (1 authority / 0 facts /
  1 capture — same as the `6008dcc` deploy), so the accept gate has nothing
  to check against.
- Deployed over `6008dcc`; `current` → `14615cd`; 8 releases on disk
  (≤10 cap); manifest carries notes verbatim + `rollback_safe = true`;
  prod `user_version = 8`.

### Verification (observed, this run)

- `cargo xtask ci` exit 0 (fmt, clippy `-D warnings`, 513 cargo tests /
  0 failed across 32 suites, node 78/78 incl. new quote test,
  svelte-check 0/0, check:imports, both
  builds, smoke incl. new redirect/single-root assertions, budget
  226136/228352); `secrets-scan` clean (334 files; staged-hook clean on
  every commit).
- `cargo test -p web admin_voice`: 10/10 (7 existing + 3 quote).
- `node --test web/test/calls-client.test.mjs`: 8/8.
- Live probes: healthz `{"status":"ok","version":"0.1.0+14615cd"}`; index
  `{"authorities":[]}`; admin `/` → 303 `https://admin.andrii.ai/admin`;
  `/mock/anything` → 404; quote POST unauthenticated → 403 (route mounted);
  P2 shipments garbage body → 422 (handler live, no write).
- Server: `current` symlink → `releases/14615cd`; `release.toml` notes +
  `rollback_safe`; `user_version = 8` via `apostille-sqlite`.

### Files changed (this prompt's commits)

- `crates/web/src/admin.rs` (303 + mock-404 + 4 tests), `admin_voice.rs`
  (quote handler + 3 tests), `p2.rs` (fmt), `p2_labels.rs` (submission_id),
  `p2.rs` (build_no stamp), `Cargo.toml` (+contracts).
- `xtask`: `fetch_slug.rs` (new), `demo_vietnam.rs` (new), `main.rs`
  (dispatch + scan + CI gates), `Cargo.toml`, `tests/` (fetch/analyze/demo).
- `crates/handbook-store`: `008_capture_watermark.sql` (new), `db.rs`
  (SCHEMA 8 + watermark); `handbook-agent/runner.rs` (capture plumbing);
  `contracts/model.rs` (`DraftRequest.capture_id`).
- `web/`: `calls-client.js` (+quote), `[id]/+page.svelte` (+form),
  `calls-client.test.mjs` (+tests), `smoke.mjs` (+assertions).
- `docs/`: `ui-framework-proposal.md` (new), `specs/voice-telnyx.md` (§7),
  `specs/p2-forms-labels.md` (OPEN-32), `specs/spa-separation.md` (root
  decision), `specs/README.md` (D36/D37), `specs/app-features.md` (F11),
  `ops/staff-enrolment.md`, `ops/scripts/ops-hal-access.sh`,
  `ops/scripts/ops-deploy.sh` (notes passthrough).

## Commits

- `fe83289` Infra: admin root redirect, hal enrol wrappers, spec decisions
- `095a8e6` Slice B: per-jurisdiction collection loop + Slice F CI gates
- `e247392` Slice D: optional submission join on shipments + PDF build stamp
- `8161f55` fmt: rustfmt-canonical rewraps on Slice B/D files
- `8c66845` Slice E: frozen VN demo drill + voice quote-to-draft
- `06bd92a` Admin UI: quote-to-draft surface + UI framework proposal
- `14615cd` ops-deploy: forward optional MIGRATION_NOTES into manifest
- (this log entry commits separately, per convention)

## Follow-ups

- OPEN-29/30/31 (UI proposal: confirm Option A, Phase-0 drive-bys,
  pilot scope) — owner: author; blocks UI Phases 0–3.
- OPEN-32 (staff label-queue list route spec) — owner: author; blocks
  Slice D manual-queue UI.
- Slice C operator unlocks (Telnyx keys + 8am proof call, terms read,
  META_API_KEY + shapes, Gmail `mailer.*` + relay-sender, portal work,
  `apostille-ship` grant removal, www SAN) — unchanged, parallel track.
- Slices B/D/E staging drills need staging host + test inbox (~Oct 31).
- OPEN-26/27/28, OPEN-9..24 unchanged.
- Prune-after-cutover wart stands: `xtask deploy` cuts over before pruning
  (prune failure ABORTs post-live) — future xtask slice should warn-not-ABORT.
