# 091 — A2 seed + drill-3 scripts staged; mail verdict; F1 live

- Prompt: 2026-10-30 00:28 UTC (approx, continuation directive)
- Response: 2026-10-30 00:40 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Continuation directive: verify the mail-timer first fire via
hal-side probes, then advance A2 HCCH seed, drill-3 full ×3,
cert-gate flip, and Day-4/Day-6 gate evidence to the Nov 4
ship gate, preserving commits 4d979d6–a84cf6f.

## Response

Mail verdict + two new staged operator scripts; no root steps
executed this turn (all packaged for the operator).

- Mail first fire (00:30:09 UTC): exit 2, exactly predicted.
  Stronger than predicted: exit 2 in `run_mail` is reachable
  ONLY for missing Gmail creds (store/KEK/config failures all
  exit 1; flag-validation exit-2s are unreachable with the
  unit's fixed flags). So the run opened the store via the
  F1 `--store` flag, loaded the KEK through `LoadCredential`,
  and decrypted config — **F1 proven live on prod**, and the
  only remaining mail precondition is the documented Gmail
  sealing (author action). Status line confirms the shipped
  command line ran verbatim.
- `ops-hcch-seed.sh` (new, staged): dir-scoped packs-load of
  hcch.json only (deliberately NOT all 9 packs — full seed is
  a separate author call), URI derived from the loaded source
  row, staff fetch-url from prod IPs, hcch-poll unblock proof,
  root-residue check. Fully rehearsed on a Mac temp store
  against live hcch.net: +1 authority/source, fetch 200 →
  capture 1, `hcch-poll: unmodified` (exit 0). The ONE URI
  (`.../status-table/?cid=41`) was already decided in the
  pack — no author decision needed for the scoped seed.
- `ops-drill3-run.sh` (new, staged; draft by read-only child,
  vetted + fixed by parent): one full drill-3 run — newest
  daily → scratch restore → loopback re-port (ports probed,
  TLS/ACME/QUIC neutralized, binds pinned) → scratch boot as
  andrii-ai → healthz + stamp → packs + config-verify →
  envelope attestation WITHOUT disclosure (tty-gated,
  type-anything-aborts) → kill + silence check → scratch
  removed only on green → prints the drill-log row. Parent
  verified all load-bearing claims against code (key names +
  types, serve-mode fallthrough with empty≡unset TLS pair,
  config-set/verify flags) and caught one draft bug before
  commit: the `config-set`/`config-verify` help greps would
  have false-aborted (pipe-joined help line) — proven
  against real `help` output both ways.
- Gate status: steps 0–4 + F1-live + nightlies-live done.
  Cert flip still needs the author's browser check; rulings /
  terms / full seed / Gmail sealing still author-side;
  live-proof sprint (step 5) and Day-4 closeout (step 6) are
  next after the operator runs below.

Operator (root, in order; paste all outputs back):

1. `bash /home/hal/apostille/scripts/ops-hcch-seed.sh`
2. `bash /home/hal/apostille/scripts/ops-drill3-run.sh` (×3,
   fresh scratch each time; a failed run restarts drill 3's
   count per `drill.md`)

Files changed:

- `docs/ops/scripts/ops-hcch-seed.sh` (new, +x).
- `docs/ops/scripts/ops-drill3-run.sh` (new, +x).
- `docs/ops/scripts/README.md` (both entries).
- `docs/prompts/2026-10-30-091-seed-drill3-mail.md` (this entry).

Verification performed:

- `bash -n` clean on both scripts (repo + server); drill-3
  staged copy byte-compared (seed script staged pre-rehearsal,
  server `bash -n` clean).
- Seed rehearsed end to end on a temp store with a live
  hcch.net fetch (then removed); unblock proven by exit-0
  `hcch-poll`.
- Draft-bug proof: fixed help-grep matches real output;
  original proven to miss.
- Mail verdict from hal `systemctl show`/`status` +
  exhaustive exit-code audit of `run_mail`.
- No Rust touched: CI green at 72ae915 still covers the
  tree; pre-commit `secrets-scan` (hook) clean.

## Commits

- ba4228e — Ops: A2 HCCH seed script.
- 45e6b5c — Ops: drill-3 full-run script.
- (This 091 log rides in the commit below.)

## Follow-ups

- Awaiting seed + drill-3 ×3 paste-backs (drill-3 rows go to
  `drill.md` as they land).
- Watch timer first fire Sat 00:12 UTC (no action needed).
