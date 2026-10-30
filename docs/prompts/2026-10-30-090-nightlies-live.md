# 090 — Nightlies live: first backup proven + timer enabled

- Prompt: 2026-10-30 00:24 UTC (approx, operator paste-back)
- Response: 2026-10-30 00:27 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Operator pasted `ops-backup-enable.sh`: fully green —
preconditions OK, first backup ran (id 2026-10-30, per-file
hashes in journal), exact 5-file set, `sha256sums` OK,
`backup-verify` green, timer enabled (next 02:31 UTC), zero
WARNING/ABORT.

## Response

Step 2's "nightly backups live" criterion is now TRUE on
prod (was repo-side only): installed, first nightly proven
restorable, timer live. Drill-3 leg 1 recorded in `drill.md`;
full-drill ×3 (scratch boot on scratch ports, healthz,
packs-validate, envelope locate) still 0/3 — needs a
KEK-holding operator session.

- Journaled per-file hashes (A2 second channel) observed
  working in the pasted §1 output.
- Independently corroborated as hal: backup timer
  `enabled`; `/var/lib/apostille-backups/` is
  `Permission denied` for hal — B1 owner-only perms holding
  live (hal was never meant to read PII backups).
- Mail timer's first fire (00:30 UTC) had not happened at
  check time (00:24, `ExecMainStartTimestamp` empty) — the
  exit-2-if-Gmail-unsealed expectation from 089 is still
  pending; checkable later via hal service state.
- All three operator scripts are now executed; the §7
  handoff chain for steps 2–4 is complete.

Files changed:

- `docs/ops/drill.md` (drill-3 leg-1 row).
- `docs/prompts/2026-10-30-090-nightlies-live.md` (this entry).

Verification performed:

- Pasted output checked against the backup-enable evidence
  bar (daily id, file set, sums, verify line, timer state,
  DONE) — all present, no WARNING/ABORT.
- Hal-side probes: timer enabled, backup tree unreadable
  (perms positive control), mail/backup service states sane.
- Pre-commit `secrets-scan` (hook) clean.

## Commits

- (Drill row + this 090 log ride together in the commit below.)

## Follow-ups

- Mail first-fire result (post-00:30): confirm exit 2 with
  the Gmail-sealed message, or green if creds are sealed.
- Drill-3 full runs ×3 (operator, KEK-holding): scratch
  restore + re-port + boot + healthz + packs + envelope.
- Offsite pull decision (author) + A3 prune-marker follow-up.
- Unchanged author items: cert browser check → flip; three
  rulings; terms attestation; A2 HCCH seed URI.
