# 085 — Loop-first-run ABORT: unseeded HCCH gated too hard

- Prompt: 2026-10-30 00:10 UTC (approx, operator paste-back)
- Response: 2026-10-30 00:15 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Operator pasted the root run of `ops-loop-first-run.sh`: §1–3
green (wrapper installed, grant appended, packs clean, sealed
rows present), then §4 ABORT: `hcch-poll: no website source
under the hcch authority — seed the ONE status-table URI first`,
exit 2.

## Response

Root cause is script-side, not prod-side: the script made
`hcch-poll` a hard gate, but prod has no HCCH seed (the known A2
gap — seed URIs were never decided/loaded). The poll's own
design treats this as a setup state: `run_hcch`
(`xtask/src/watch.rs:138-178`) exits 2 **only** for `NoSource`
with fixed flags (0 = polled, 1 = failed), and `poll_hcch`
documents "staff must seed first". Patched §4: exit 2 becomes an
announced skip with the packs-load + fetch-url follow-up; any
other nonzero exit still ABORTs.

- Verified the aborted run left no residue: no
  `.loop-creds` staged (abort preceded §5), no root-owned files
  under `/var/lib/apostille`.
- The `authority=1 fact=0 capture=1` counts are the migration-005
  sentinel rows (id 0, UNATTRIBUTED), not real data — the zero
  baseline stands. (Left the script's raw counts as-is; filtering
  sentinels would add a schema assumption for cosmetics.)
- Re-staged script is byte-identical to the repo (`diff` over
  ssh); script stays idempotent for the re-run (grant takes the
  already-present path).

Operator re-run (root, same command):

`bash /home/hal/apostille/scripts/ops-loop-first-run.sh`

Expected new evidence: the `(note: hcch unseeded …)` line, the
sweep outputs, `meta api ok: …` (KEK proof), `no root-owned
files`, `DONE loop first run`. Then continue with
`ops-units-reinstall.sh` + `ops-backup-enable.sh` per 084.

Files changed:

- `docs/ops/scripts/ops-loop-first-run.sh` (§4 NoSource skip).
- `docs/prompts/2026-10-30-085-loop-hcch-unseeded.md` (this entry).

Verification performed:

- `bash -n` clean (repo + server side); staged copy
  byte-compared against the repo.
- Exit-code claim checked against `xtask/src/watch.rs:138-178`
  (all five `poll_hcch` arms) — no other exit-2 path with fixed
  flags.
- Read-only prod probes: sentinel rows identified via
  `apostille-sqlite`; residue scan clean.
- No Rust touched: CI green at 72ae915 still covers the tree;
  pre-commit `secrets-scan` (hook) clean.

## Commits

- 2904db0 — Ops: loop-first-run tolerates unseeded HCCH (A2).
- (This 085 log rides in the commit below.)

## Follow-ups

- A2 HCCH seed (packs-load + fetch-url of the ONE status-table
  URI) is now the next prod-data action after the loop unpark;
  needs the seed URI decision.
- `hcch-poll` will exit 2 (skipped, noted) on every loop-script
  re-run until seeded — expected, not a regression.
