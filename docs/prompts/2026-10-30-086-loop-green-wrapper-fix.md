# 086 — Loop unparked: KEK proof green + wrapper trap bug

- Prompt: 2026-10-30 00:18 UTC (approx, operator paste-back)
- Response: 2026-10-30 00:20 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Operator pasted the re-run of `ops-loop-first-run.sh`: all
sections green through `DONE loop first run`, with one §7
`WARNING: staged KEK survived the wrapper trap — deleting now`.

## Response

Step 3 (prod loop first run) is UNPARKED — the exact blocked
chain is proven: `load_kek` → sealed `meta.api_key` →
`GET /models` via the hal→andrii-ai pinned path, exit 0.

- Key validity proof (was runtime-only, unproven): `meta api ok`
  with catalog `muse-wonder-1.0, sam-3.1,
  muse-spark-1.3-contributor, muse-voice-transcribe-1.0,
  muse-spark-1.3, muse-image-1.0, muse-spark-1.2-contributor,
  muse-spark-1.2, muse-spark-1.1`. G3-relevant endpoints
  (Spark, SAM, voice-transcribe) all present — de-risks the
  live-proof sprint's three calls.
- Benign: `net.public_host: empty value; using default` config
  warning (default applied, no failure). Live DB untouched by
  the run (mtime predates it, counts identical, no WAL
  sidecars) — expected: 0 due URLs, unseeded HCCH, read-only
  `--check`.
- The WARNING is a real wrapper bug, correctly caught by the
  §7 fallback (KEK was deleted; `rmdir` succeeded silently, so
  no residue): the heredoc wrapper ended in `exec xtask`,
  which replaces the shell — an `EXIT` trap can never fire
  after `exec`. Fixed: no `exec`; capture the exit code,
  explicit `rm -f`, `exit $CODE` (trap retained for signals).
  §7 fallback hardened: undeletable staged KEK is now ABORT,
  not a second warning.
- The installed `/usr/local/sbin/apostille-loop` on prod is
  still the `exec` version until the script re-runs §1 — hence
  the requested confirmation run below (also doubles as loop
  idempotency evidence).

Operator: one more run of the same command (installs the fixed
wrapper; the WARNING must be gone), then continue:

1. `bash /home/hal/apostille/scripts/ops-loop-first-run.sh`
2. `bash /home/hal/apostille/scripts/ops-units-reinstall.sh`
3. `bash /home/hal/apostille/scripts/ops-backup-enable.sh`

Files changed:

- `docs/ops/scripts/ops-loop-first-run.sh` (wrapper rm without
  exec; §7 undeletable-KEK ABORT).
- `docs/prompts/2026-10-30-086-loop-green-wrapper-fix.md` (this entry).

Verification performed:

- `bash -n` clean (repo + server); staged copy byte-compared.
- Pasted output checked against the 085 evidence checklist:
  all lines present; `meta api ok` + `no root-owned files` +
  `DONE` confirm the unpark; the WARNING traced to the
  `exec`-vs-trap interaction (shell semantics, no alternative
  cause — the trap line itself was correct).
- No Rust touched: CI green at 72ae915 still covers the tree;
  pre-commit `secrets-scan` (hook) clean.

## Commits

- aa48dcb — Ops: loop wrapper deletes staged KEK without exec.
- (This 086 log rides in the commit below.)

## Follow-ups

- Awaiting the confirmation re-run + scripts 2–3 paste-backs.
- A2 HCCH seed still the next prod-data action after unpark.
