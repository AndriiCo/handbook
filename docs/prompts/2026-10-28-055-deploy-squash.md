# Prompt 055 — deploy dc400a9 (009–011) + squash migrations to baselines

- Prompt timestamp (UTC): 2026-10-28T23:08:00Z
- Response timestamp (UTC): 2026-10-28T23:30:39Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

Run the deploy and migrations. After migrations - update the schema
to latest versions and delete outdated migrations code

## Response

Deployed `dc400a9` to prod (schema v8→v11, healthz
`0.1.0+dc400a9`), verified all three migrations on the live DB,
then squashed the incrementals into `schema_baseline.sql` (v11) +
`p2_schema_baseline.sql` (P2 v2) and deleted the 13 outdated files
(work commit `6061d18`).

Deploy (runbook-release.md §1–3 + `ops-deploy.sh`):

- Pre-flight green as hal (WRITE/FLIP/SUDO/SQL-OK, stop
  refused); live was `88562bb`, schema v8, P2 v2.
- `cargo zigbuild --release -p web -p xtask` (4m05s, sccache) +
  CI-fresh web bundles staged to
  `~/apostille/staging/dc400a9/` (ELF verified); staged `SHA`
  (full), `apostille.service` (byte-identical to prod unit —
  install was a no-op diff), one-line `MIGRATION_NOTES`.
- 009–011 scanned additive (no DROP/RENAME/STRICT) →
  rollback-safe, no `--rollback-unsafe`.
- Accept gate: prod `build` table is EMPTY (`count(*)=0`,
  verified twice) — no handbook build was ever stamped, so the
  gate is vacuous (it would fail even as root with "run
  build-handbook first"). Deployed with the script's announced
  `--skip-accept` escape hatch per its "confirm with agent"
  clause; the skip is in the output + manifest trail. (Mid-run
  self-correction: an early "8" was misread as build_no — it
  was `user_version` from the second query of a two-query
  pipe; re-verified cleanly before deciding.)
- `ops-deploy.sh` output: `deployed dc400a9 (previous:
  88562bb, pruned: 9c28dd9)`, stamp
  `{"status":"ok","version":"0.1.0+dc400a9"}`.

Migration verification on prod (service auto-migrates at boot):

- `user_version=11`; `suppress_rule` + `suppression` tables
  present; `fact` gained `citation_start/citation_end/
  reasoning`; `source` gained the six 009 columns
  (poll_cadence/etag/simhash sampled); `mail_poll` present,
  0 rows (timers not yet enabled); manifest records schema
  11 + notes + `rollback_safe=true`; `/healthz` ok, admin
  200.

Squash (`6061d18`, 18 files, +262/−810):

- New `migrations/schema_baseline.sql` (36 CREATEs + 005 id-0
  sentinel seeds) and `migrations/p2_schema_baseline.sql`;
  deleted 001–011 + p2_001–002 (in git history).
- `db.rs`: fresh DBs apply the baseline once and stamp
  current in one transaction; v1–v10 (P2: v1) refused loudly
  via `version_gap()` naming the remedy (upgrade through ≤
  dc400a9 or restore); newer-than-binary refused too.
  `SCHEMA_VERSION` stays 11 / P2 2, so the next deploy opens
  prod's DB untouched. `HARD_MIGRATIONS` (005) kept: it
  guards rollback across shipped history, not code.
- Tests: 5 incremental-path tests replaced (v1/mid-version
  refusal + rows-intact, baseline-rollback atomicity, P2
  refusal, P2 label-purchase shape on fresh DB); 4 schema
  tests renamed `migration_*` → `baseline_*`. Doc pointers
  updated (`db.rs` header, `store.rs`, `runner.rs`,
  threat-model, p2-forms-labels); roadmap/SECURITY historical
  refs left frozen.
- Correctness proof: baseline-vs-incremental schema dumps
  `diff`-identical (both DBs); FTS MATCH + both audit
  triggers smoke-tested on a baseline DB; the missing-seed
  failure (FK violations on id-0 inserts) was caught by the
  suite before CI and fixed by appending the 005 seeds.

## Verification

- Prod: healthz `0.1.0+dc400a9`, `user_version=11`, all
  009/010/011 objects present (queries above), admin 200.
- `RUSTC_WRAPPER=sccache cargo xtask ci` → exit 0 after the
  squash: 578 passed / 0 failed Rust, 85/85 node,
  svelte-check 0, both builds, smoke OK, tailwind 266.
  (One clippy fix: `Error::new(Other)` → `Error::other`.)
- Staged-hook secrets-scan clean (both commits accepted).

## Commits

- `6061d18` Squash migrations 001-011 + p2_001-002 into
  v11/v2 baselines (message amended once, unpushed, to remove
  a false 004 note — 004 was tracked and deleted normally)
- (deploy itself ships `dc400a9`, committed in prompt 054)
- (this log entry commits separately, per convention)

## Follow-ups

- prod `build` table still empty: first `build-handbook` stamp
  (needs andrii-ai DB write — no hal grant) would re-arm the
  accept gate for future deploys; until then `--skip-accept`
  stays per-release-confirmed.
- Watch + mail timers (D41) still need root install/enable on
  prod — hal cannot; author action.
- §11 OQ-1..5 open; OPEN-32 (`docs/specs/p2-forms-labels.md`
  §9) awaiting approval; `meta.api_key`/Gmail/Telnyx secrets
  via admin config UI.
