# 030 — Scripted operator handoffs + AGENTS.md §7

- Prompt timestamp (UTC): 2026-10-28
- Response timestamp (UTC): 2026-10-28
- Author: Andrii Tereshchenko
- Agent/model: Muse Code, powered by Meta Muse Spark

## Prompt

I find it's time-consumming that you ask mw to do a lot of stuff with elevated permissions and nope of that is packeged in easy to run scripts. First - get setup once for all root-required actions and then give easy to use and without placeholders (can propmpt for inputs) scripts. Save this guideline to agents.md.

## Response

Durable mechanism (setup once): root-required actions ship as
committed, reviewable scripts under `docs/ops/scripts/`, staged by
the agent to `~/apostille/scripts/` on the server; operator reviews
then runs `sudo bash <script>`.

- `ops-deploy.sh <staging-dir>`: installs the staged unit file
  (diff shown first), reads build N from the live DB, runs
  `xtask deploy`, verifies the health stamp. VERSION from dir
  name, SHA from staging/SHA — zero inputs.
- `ops-acme-enable.sh`: guards that the live binary contains the
  :80-first fix (refuses on the old binary), flips
  `acme.enabled`, unpins the file cert, restarts, polls health to
  180s, prints issuance evidence. Zero inputs (EAB already sealed
  in DB).
- `README.md`: per-script rules (no placeholders, ABORT-fast,
  evidence-back, `bash -n` clean).
- `AGENTS.md` §7 "Operator Handoffs" records the guideline
  (committed separately per the file's own rule).

Verification: `bash -n` clean on both (no shellcheck on this box);
scripts staged to `~/apostille/scripts/` + `SHA` file written into
`~/apostille/staging/69480d3/` (verified present server-side).

## Commits

- c330a3a scripts (+ this log entry's companion AGENTS.md commit; see git log)

## Follow-ups

- Operator: review + `sudo bash` the two scripts in order (deploy,
  then ACME flip); paste outputs back; agent verifies dual-SAN
  cert and closes goal-028.
- hal sudo remains fully denied — hal-only deploys still need a
  root decision on the intended end state.
