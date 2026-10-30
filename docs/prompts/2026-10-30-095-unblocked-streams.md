# 095 — Unblocked streams: gate evidence + drill-4 legs + packaging

- Prompt: 2026-10-30 00:48 UTC (approx, "any tasks not blocked on
  a operator?" + all-three-streams pick)
- Response: 2026-10-30 00:55 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Any taks that not bloced on a operator? (Verbatim; then picked
all three offered streams: gate-evidence bundle, drill-4
scratch legs, timer units + seed script.)

## Response

Ran all three unblocked streams. No operator steps executed;
one script staged for a future author call.

Stream 1 — Day-4 gate evidence bundle (all local, temp
stores, prod untouched):

- `demo-vietnam`: 5/5 PASS (incl. [3/5] form PDF render,
  1794 bytes — stamped-PDF-path evidence).
- `instruct-demo`: triggers=3 proposed=3 (transcript→facts
  path evidence).
- `voice-replay` (default happy): PASS (6 webhooks, 2
  transcripts).
- `cargo test -p forms`: 17/17 (5 render tests green).
- Sweep unit tests: 3/3, PLUS a CLI end-to-end on a seeded
  temp store (authority via real CLI, SQL-seeded old/fresh
  audio + sessions): `deleted: 1`, old row/file gone,
  session link nulled, fresh kept, 1 audit row. (Seed
  `captures=2` post-sweep includes the migration-005
  sentinel — accounted, not a leak.)
- `packs-validate`: clean (9).
- Per the plan, this log IS the gate-evidence record; the
  gate verdict itself still needs the author's operative
  gate-list ruling.

Stream 2 — drill-4 scratch legs (local; envelope step stays
author): enroll → mint (`setup_`+64hex, 1 audit row) →
seeded staff passkey → re-mint refused (exact message, exit
1) → `--force` re-mint (new key, 2nd audit row). Old-key
invalidation proven structurally: `config.key` is PRIMARY
KEY, so the `ON CONFLICT DO UPDATE` overwrite admits no
second hash. (One self-caught slip: queried
`setup_key_hash` instead of `setup.key_hash` after cleanup —
recovered via schema, no re-run needed.)

Stream 3 — packaging (read-only child drafts, parent
vetted/committed): `apostille-retention.*` (PROPOSED,
F1-clean, UMask 0027 deliberately — job writes only the
group-readable live store; network-online dep deliberately
omitted as residue) + `ops-packs-seed.sh` (load-only full
seed, counts-guarded, staged to the server, awaiting the
author's full-seed call). Child's flag/table claims
verified against main.rs/packs.rs/schema; full 9-pack load
rehearsed locally (15 authorities, 59 tasks, all clean).

Files changed:

- `docs/ops/apostille-retention.service` / `.timer` (new).
- `docs/ops/scripts/ops-packs-seed.sh` (new, +x) + README.
- `docs/prompts/2026-10-30-095-unblocked-streams.md` (this entry).

Verification performed:

- Every stream-1/2 command run directly; outputs quoted
  above; temp stores removed afterwards.
- `bash -n` on the seed script (repo + server); rehearsal
  before commit.
- Pre-commit `secrets-scan` (hook) clean.

## Commits

- f106bfd — Ops: retention units + full packs-seed script.
- (This 095 log rides in the commit below.)

## Follow-ups

- Retention units need install support at enable time
  (`ops-units-reinstall.sh` covers 4 fixed units — v2 or
  manual install then).
- Full seed awaits the author's call (script staged).
- Drill-4 envelope step + remaining author queue unchanged.
