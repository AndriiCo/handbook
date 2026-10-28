# AGENTS.md

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- Correct error handling, but with correct scope for  improbable scenarios.
- If you write 200 lines and it could be 100, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.

## 5. Project Instructions (Apostille Handbook)

Stack and tooling:
- Rust workspace. No shell scripts: project tasks live in `xtask/` and run
  via `cargo xtask <task>`, reusing the project's code, libraries, and
  technical decisions.
- `cargo xtask ci` (fmt check, clippy with `-D warnings`, tests) must pass
  before committing.

Commits:
- Commit after each significant change; push when the user asked for it.
- If a change must be rolled back, make a rollback commit (revert) and
  explain why in the commit message — never silently rewrite history.

Prompt/response log:
- Retain a complete log of all prompts and responses in `docs/prompts/`,
  following `docs/prompts/README.md`: one Markdown file per prompt, with
  UTC timestamps, the prompt verbatim, a response summary, files changed,
  commit hashes, and verification performed.
- Write the entry in the same turn as the work and commit it alongside (or
  immediately after) the work it describes so hashes stay accurate.

Living document:
- Keep this file updated when project needs change it (new conventions,
  tooling, or log requirements). An update to this file is itself a
  significant change: commit it separately with a clear message.

## 6. Agent Cohort Rules (approved prompt 012 — swarm execution authorized)

Speed rules for parallel-agent execution.

- **Parallel-first decomposition.** Split independent streams
  across subagents (research, coding, security review, test runs,
  specs); size fan-out to genuinely independent subtasks, never
  one child per file.
- **Single-writer checkout.** Children are read-only
  researchers: they return drafts as text, the parent writes
  files. No parallel writers on the shared tree, no exceptions
  without isolation.
- **Truncation-proof handoffs.** Cap child outputs (~4500 words);
  recover full texts from child session logs when delivery
  truncates; never treat a partial delivery as the complete
  result without checking the log.
- **Decisions stick.** Record author decisions in
  `docs/specs/README.md` the same turn; never re-ask a decided
  item. Open questions carry a number, an owner, and the stream
  they block.
- **Lanes and gates.** CODE merges behind `cargo xtask ci`; SEC
  blocks merges on trust/PII/auth findings; TEST owns red/green
  and the demo drills. Day 4/6/8/10 are demo gates per
  `docs/specs/app-features.md`.
- **Spec-before-surface.** New user-visible surfaces and schema
  changes get a reviewed spec section first; fixtures are
  synthetic only and canonical values never leak into facts.

## 7. Operator Handoffs (root-required actions)

Never hand the operator a sequence of one-liners with
`<placeholders>`. Package every root-required action as a single
committed script under `docs/ops/scripts/`:

- Zero placeholders: derive values (VERSION/SHA from staging,
  build numbers from the DB) or prompt for secrets (hidden entry,
  never on the cmdline or in history).
- Agent stages the script to the server
  (`/home/hal/apostille/scripts/` — always absolute paths, never
  `~`, since the operator may be a different user); scripts run
  as hal (pinned sudo grants, one-time root setup) or root via
  plain `bash <script>`.
- Fail fast with `ABORT: <reason>`; never plow past a failure.
- End by printing the evidence the agent needs pasted back.
- `bash -n` clean before every commit; one script per action.
