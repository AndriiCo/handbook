# 084 — Steps 3–4 advance: deploy train 72ae915 + operator staging

- Prompt: 2026-10-29 23:41 UTC (approx, continuation directive)
- Response: 2026-10-29 23:46 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Continuation directive: continue the resumed prompt-081 delivery
plan from step 3, preserving commits 4d979d6, acb678d, 72ae915 —
stage the operator scripts + unit files for root execution and
coordinate loop unpark / unit reinstall, then proceed through
deploy and gate closure.

## Response

Advanced steps 3–4 as far as agent-reachable hands allow. Live
prod deploy executed as hal; root steps staged and handed to the
operator as committed scripts.

- SSH as hal verified; live prod was `b577c10` (as recon
  predicted); no timer units installed; runbook §0 preflight
  green (WRITE/FLIP/SUDO/SQL OK, stop correctly refused).
- Prod `/var/lib/apostille/mail/` is an empty stale dir
  (FileMailer is dev/test-only, `magic.rs:112-116`); no backup
  change needed. Store is ~450KB — weekly plain-copy is fine.
- Staged to the server: `ops-loop-first-run.sh` +
  `ops-units-reinstall.sh` in `/home/hal/apostille/scripts/`
  (`bash -n` clean on both ends) and the 8 unit files in
  `/home/hal/apostille/units-staging/`.
- Deploy train at HEAD `72ae915` (step 4, first, per the forward
  recon VERSION call): zigbuild ELF x86-64 pair with
  BUILD_SHA=72ae915, both web bundles, full stage set incl.
  `apostille.service`, 40-hex SHA, and `packs/general.toml`
  (F2). `ops-deploy.sh --skip-accept` (announced; no prod build
  row — loop never ran): unit installed (Burst 12 +
  TimeoutStopSec 20s), deployed (previous b577c10, pruned
  49775a6), stamp `0.1.0+72ae915`. Verified from outside:
  healthz ok, version build 72ae915, shell 200, new `backup` /
  `backup-verify` tasks live, `current/packs/general.toml`
  present (units-reinstall F2 gate will pass), service
  active/running. Notable: the old binary's stop used the NEW
  unit's 20s TimeoutStopSec, so no 90s hang on this train.
- One staging hiccup: runbook §2's mkdir only creates `spa/`;
  created the remaining staging subdirs explicitly (runbook
  left as-is — one-word fix territory, not worth the churn;
  noted here).
- New `ops-backup-enable.sh` (AGENTS §7 packaging for the
  backup-timer enable decision): starts the service once
  (skipped if today's daily exists), verifies the daily (exact
  5-file set + `sha256sum -c` as andrii-ai), runs
  `backup-verify` into scratch as the drill-3 restore proof
  (scratch removed), enables the timer, prints evidence. Staged
  to the server; README entry added.
- Drill log: drill-1 4th leg recorded green at 72ae915.
- Operator handoff (root, in order; paste all outputs back):
  1. `bash /home/hal/apostille/scripts/ops-loop-first-run.sh`
  2. `bash /home/hal/apostille/scripts/ops-units-reinstall.sh`
  3. `bash /home/hal/apostille/scripts/ops-backup-enable.sh`
- Still blocked on the author (unchanged): cert-present browser
  check before the cert-gate flip; three rulings (gates / SHP-0
  / staging ETA); terms-read attestation. Cert flip, live-proof
  sprint, and gate closeouts follow those + the root runs above.

Files changed:

- `docs/ops/scripts/ops-backup-enable.sh` (new, +x) +
  `README.md` entry.
- `docs/ops/drill.md` (drill-1 4th leg row).
- `docs/prompts/2026-10-29-084-deploy-train-handoff.md` (this entry).

Verification performed:

- Server-side `bash -n` on all three staged scripts; staged
  file listings confirmed (3 scripts + 8 units).
- Deploy output captured whole (unit diff, announced skip,
  deploy line, stamp line); external curl triple
  (healthz/version/shell) + service state + shipped-files
  checks all green.
- No Rust touched this turn: `cargo xtask ci` green at 72ae915
  (prompt 083) still covers the tree; pre-commit `secrets-scan`
  (hook) clean.

## Commits

- 397d8e8 — Ops: backup-enable script + drill-1 4th leg.
- (This 084 log rides in the commit below.)

## Follow-ups

- Operator: run the 3 root scripts in order; paste outputs back
  for agent verification (hal cannot read timer states — the
  pasted §6 outputs are the evidence of record).
- Then: drill-3 ×3 (first leg rides backup-enable), cert flip
  after the author's browser check, live-proof sprint (step 5),
  Day-4 gate closeout drill (step 6).
