# 087 — Wrapper fix verified live (WARNING gone)

- Prompt: 2026-10-30 00:23 UTC (approx, operator paste-back)
- Response: 2026-10-30 00:25 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Operator pasted two `ops-loop-first-run.sh` runs: the first
still shows the old `survived the wrapper trap` WARNING (ran
from the pre-fix staged script), the second is fully clean —
no WARNING.

## Response

The `aa48dcb` wrapper fix is verified end to end on prod: the
clean run's §7 found no staged KEK (the wrapper deleted it
itself), and the installed `/usr/local/sbin/apostille-loop`
read back over ssh is the fixed version (no `exec`, explicit
`rm -f`, exit-code passthrough). Loop idempotency now proven
×3 (086 run + these two), all `meta api ok`, counts stable,
no residue in any run.

Accepted nit (not worth a root cycle): the installed
wrapper's header comment still says "deleted on exit (trap)
either way" — behavior is correct (trap for signals,
explicit rm for the normal path); reword only if the wrapper
is ever touched again.

No repo changes this turn besides this log. Operator
continues with scripts 2–3 (`ops-units-reinstall.sh`,
`ops-backup-enable.sh`).

Files changed:

- `docs/prompts/2026-10-30-087-wrapper-verified.md` (this entry).

Verification performed:

- Pasted outputs checked: run 1 = old script text (pre-fix),
  run 2 = new script, zero WARNING/ABORT lines, `DONE` present.
- Installed wrapper content read via hal ssh; matches the
  fixed heredoc from `aa48dcb`.
- Pre-commit `secrets-scan` (hook) clean.

## Commits

- (This 087 log rides in the commit below.)

## Follow-ups

- Awaiting scripts 2–3 paste-backs (units reinstall, backup
  enable + drill-3 leg 1).
