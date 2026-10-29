# Prompt 079 — Instruction worker: triggers to reviewed fact updates

- Prompt (UTC): 2026-10-29
- Response (UTC): 2026-10-29
- Author: Andrii (operator)
- Agent: Muse Code (solo inline)

## Prompt

```text
Deal. What about a worker that make specific state / county instructions based on inputs? It should be able to reason about general template requirements, refine it based on special findings even if it applies to specific authorities. Again - it should be a code for worker that calls Models APIs and will be update to update facts in near real-time (as email will arrivae or customer feedback proposed or call recording with new information from Q&A woith a human). Fact will still follow human review process (before / after values and reasons for the change made( - but it should be done on a server side in background, not a Muse from this machine. Apply the same rules for summasization, context compaction or multiple-step processes (orhestrate it in code). Have a test-case for one of authorities like a Washington state to demostrate results.
```

## Response

Built the instruction worker as new crate `instruct` (+ `xtask
instruct-sweep` / `instruct-demo`), spec-first per repo rules.
Event-driven where Atlas is pull: a 5-minute timer sweep turns
new email / comment / transcript captures into instruction
drafts + proposed fact updates; humans still decide via the
existing review queue + `decide()` path.

Pipeline (all in code): triggers past a per-authority cursor →
long evidence condensed via Atlas map-reduce (content-sha file
cache) → budgeted draft (template + live facts + findings,
estimate then live `input_tokens`, fail-don't-truncate) →
refine pass applying special findings as explicit overrides
(body + statements + reason) → diff vs accepted facts →
suppression consult (paths A+B, analysis precedent) →
atomic write: proposed facts (reasoning carries a parseable
`SUPERSEDES/BEFORE/AFTER/REASON` header — the queue renders
reasoning already, zero UI work) + `instruct.proposed` audits
(before_ref/after_ref/reason columns) + one `correction` task
(instruction markdown in detail) + cursor advance.

Deliberately schema-free: cursors live in `watermark` under
`instruct:<slug>` keys (analysis owns bare slugs); no
migration, no topic changes (sections map onto the frozen 25,
validated at pack load).

Files changed: `crates/instruct/**` (new: template, trigger,
draft, change, propose, render, worker, fixture, general
template pack), `xtask/src/instruct.rs` (new), `Cargo.toml`,
`Cargo.lock`, `xtask/Cargo.toml`, `xtask/src/main.rs`,
`xtask/src/atlas.rs` (shared live client), `crates/atlas/src/
model.rs` (scripted StubPort), `docs/specs/instruction-
worker.md` (new), `docs/ops/apostille-instruct.{timer,
service}` (new, PROPOSED — operator installs).

Washington demo (synthetic, shared by test + drill): fee
email ($20→$30), Friday-4pm comment, appointment Q&A →
instructions markdown with refined hours + pending flags, 3
proposals (2 supersede with parsed before/after, 1 new),
1 task, 3 audits; second sweep finds nothing (cursor).

Verification: `cargo test -p instruct` 13 pass (offline);
`cargo xtask instruct-demo` prints the drill on a temp store
(prod untouched); full `cargo xtask ci` exit 0.
Live API never touched in tests (StubPort scripts).

## Commits

- (work commit hash — filled after commit)
- (this log entry, committed alongside)

## Follow-ups

- First live sweeps: `--dry-run` on staging, then `--live`;
  review the first proposals before enabling the timer.
- Template pack authorship (general requirements per
  section) is content work; version bumps go through review.
- Promotion of instruction markdown to a published surface
  is out of scope (task detail carries it for now).
