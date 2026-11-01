# Role: lane `meta` (Meta Model API correctness)

You are a lane: a separate Muse session in your own worktree
(`.muse/worktrees/20261101-7e99`), on branch
`muse/session-01a19798-66eb-72b0-8ce7-3e80f3958724`. The builder merges
your branch into `main` under the protocol in `.agents/roles/builder.md`.
Rules: AGENTS.md. Never edit `.agents/`, `.muse/`, `AGENTS.md` or
`docs/ops/operator-today.md`; never push to `main`; never deploy.

## Goal

Every Meta Model API call in the repo matches Meta's documented usage,
proven with live calls on plant, without lowering any eval number. The
checklist is `.agents/meta-api-findings.md` (F1-F14, each with file:line
and its official page). Official pages: `https://dev.meta.ai/docs/<page>.md`
(the operator's Mac also keeps copies under
`~/Code/meta-models/handbook-review/meta-api/raw/`). Never commit Meta's doc
pages into this repo.

## State (2026-11-01 08:53)

Your branch holds P184-P195: fixes for F1-F14 in the checklist's order,
an eval reported at 0.966-1.000 against the 0.7 baseline, and `ca58221`,
which put six follow-ups into `docs/ops/operator-today.md`. Nothing is
merged into `main` yet.

## Next, in order

1. `git merge main` (never a rebase), so you have this file and the
   current rules.
2. Move your follow-ups off the operator's page: revert your
   `docs/ops/operator-today.md` hunk on your branch and put the items in
   `docs/lanes/meta.md` (your status file, which the builder reads).
3. The follow-ups now inside your paths:
   - `crates/atlas/src/task.rs`: the `temperature: 0.1` sites (:350, :406,
     :654), `reasoning_effort: None`, the 2048 cap (:653), `cache_key:
     None` (:408), and the raw `debug/extract.txt` dump (:382: remove it,
     or write it only outside the repo with no customer data);
   - `crates/handbook-agent/src/runner.rs:120-157`: one retry budget per
     draft, not runner + model + HTTP stacked;
   - `xtask/src/atlas.rs` `live_client`: wire `.with_terms_read(true)`
     only after the operator decides the terms opt-in (goal.md operator
     steps); until then it stays fail-closed.
4. Re-prove on plant (below) and report.

Outside your paths, for the builder after the merge (write them in
`docs/lanes/meta.md`, don't touch them): `meta.base_url` in
`crates/web/src/draft.rs` and the env-key path in `xtask/src/main.rs`;
`cached_tokens` through `contracts::Usage`, the runner and the trace
console; Telnyx WAV recording in `crates/voice`. A shared client crate is
not before judging.

## You own (touch nothing else)

- `crates/handbook-agent/src/model.rs`, `crates/handbook-agent/src/runner.rs`,
  `crates/handbook-agent/prompts/`, `crates/handbook-agent/tests/`
- `crates/atlas/src/model.rs`, `crates/atlas/src/task.rs`,
  `crates/atlas/packs/`, `crates/atlas/tests/`
- `crates/instruct/src/draft.rs`, `crates/instruct/src/trigger.rs`,
  `crates/instruct/packs/`, `crates/instruct/tests/`
- `crates/media/src/redact.rs`, `crates/media/src/transcribe.rs`,
  `crates/media/src/transport.rs`, `crates/media/tests/`
- `xtask/ops/a2-record.py`, `xtask/src/atlas.rs`
- `docs/meta-api.md`, `docs/meta-api-shapes.md`, `docs/lanes/meta.md`
- your own lines in `docs/prompts/LOG.md` (AGENTS.md §5)

If a fix needs a change outside this list, stop at the boundary, commit
what is green, and write the request in `docs/lanes/meta.md`.

## Proof (each slice)

- `cargo fmt --check`, `cargo clippy -p <crate> -- -D warnings`,
  `cargo test -p <crate>` for every crate you touched, with the main
  checkout's target directory
  (`export CARGO_TARGET_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/../target"`;
  the Mac's disk is nearly full).
- Live calls on plant with the staging key
  (`~/.config/apostille/meta-staging.env`; `set -a; . <file>` inside the
  remote shell only; never print, copy or commit the value), in your own
  directory and scratch store, e.g. `~/staging/lane-meta`: `packs-load`,
  `fetch-slug` for vietnam and usa-wa-sos, then `analyze --live` and
  `eval --live`. Never restart or bind the shared staging stack (ports
  18318/18319).
- Keep a change only if `eval --live` (26-case gold set) loses no more
  than one gold case (≈0.04) on any of `quote_found_rate`,
  `value_in_quote_rate` and `topic_accuracy` against your previous slice
  (production's 11-01 04:29Z run: 1.0, 0.969, 0.641), and the analyzer
  proposes no fewer records; report before → after, latency, and
  input/output/reasoning/cached tokens.
- Save recorded replies as replay fixtures; no key or personal data in
  them.

## Commit and report

Commit on your branch, numbered `meta-<n>` from now on (`meta-13: …`;
your first twelve used P184-P195), each with its LOG.md line. Keep
`docs/lanes/meta.md` current: the F1-F14 table (fixed / open, with the
commit), the eval numbers, and the requests for the builder. Final
message, every turn: the same three things. Do not end a turn while an
item above is open; if blocked, say on what.

## Cut-off

The builder merges nothing after Monday 11-02 15:00 PST. Commit only
green slices.
