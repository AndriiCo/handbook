# 088 — Units reinstall ABORT: F2 gate matched a comment

- Prompt: 2026-10-30 00:21 UTC (approx, operator paste-back)
- Response: 2026-10-30 00:22 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Operator pasted `ops-units-reinstall.sh` aborting in §1:
`apostille-instruct.service: --pack outside the live release:
repointed`. Watch + mail services passed first.

## Response

False positive, failed closed correctly (abort precedes
§3 diffs / §4 install — prod untouched): the F2 `--pack`
extraction grepped the whole unit file, matching the prompt-083
header comment ("`--pack` repointed at the shipped packs/
dir") instead of the `ExecStart` line. Fix: extract `--pack`
from `^ExecStart=` lines only. Audited every other `grep` in
the script — all remaining matches are full-line (`-x`) or
`^`-anchored, so comments cannot collide.

Operator re-run (root, same command):

`bash /home/hal/apostille/scripts/ops-units-reinstall.sh`

Expected: 4× `service OK` with `(instruct --pack OK:
/srv/apostille/current/packs/general.toml)`, 4× `timer OK`,
diffs, install, watch + mail enabled, instruct + backup left
disabled, `DONE units reinstall`. Then `ops-backup-enable.sh`.

Files changed:

- `docs/ops/scripts/ops-units-reinstall.sh` (F2 gate anchored).
- `docs/prompts/2026-10-30-088-units-f2-comment.md` (this entry).

Verification performed:

- `bash -n` clean (repo + server); staged copy byte-compared.
- Full `grep` audit of the script (this was the only
  unanchored content match).
- No Rust touched: CI green at 72ae915 still covers the tree;
  pre-commit `secrets-scan` (hook) clean.

## Commits

- 4b61f2c — Ops: units-reinstall F2 gate ignores comments.
- (This 088 log rides in the commit below.)
