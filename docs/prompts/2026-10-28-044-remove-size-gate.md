# Prompt 044 — Remove the web size gate (advisory-only)

- Prompt timestamp (UTC): 2026-10-28T19:04:45Z
- Response timestamp (UTC): 2026-10-28T19:06:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Remove this limit for now. We can optimize initial page size later. Size of any JS/HTML code and images is not limited
```

## Response

Removed the 223KB enforcement. `cargo xtask web-budget` still exists
but only reports (top 8 + total, always exit 0 on size); it no longer
runs in `cargo xtask ci`, the `BUDGET` const and the over-budget exit
are gone, and the help text calls it advisory. The bump history
(150→200→220→221→223KB) is preserved as a comment for the future
optimization pass. The measurement stays so the "later" work has a
baseline; nothing about images changes (they were never counted).

Live docs updated to match: `runbook-release.md` + `deploy.md` (release
step now advisory), `spa-separation.md`, `form-framework.md` (§6.2 + A5),
`staging-plan.md`, `staff-enrolment.md`, `web/README.md`, and the four
`config-values.md` staging checklists (dropped the `web-budget green`
clauses but kept the live substance: no bundled Google/Apple/Meta/mailer
JS). Prompt logs untouched (history).

`docs/ui-framework-proposal.md` got a dated amendment note instead of a
rewrite: the analysis stands as measured, but the recommendation's
premise changed — with no gate, Tailwind v4's main objection (forcing a
bump) is gone, so OPEN-29 should be re-read as ecosystem/convention vs
consolidation churn. Rollout steps updated to advisory wording.
`docs/specs/roadmap.md`'s Week-3 gate line left as a superseded plan
artifact (that roadmap's Day-N horizon is long past).

## Commits

- `b52a8d1` Remove web size gate; web-budget advisory-only
- (this log entry commits separately, per convention)

## Verification

- `cargo xtask web-budget`: prints `total JS+CSS: 226136 bytes
  (advisory, no gate)`, exit 0.
- `cargo xtask ci`: exit 0, ends at `smoke OK` (no budget step).
- Leftover sweep for live `223KB`/`budget gate`/`Day-1 gate` references:
  only intentional history notes remain (code comment, proposal
  amendment + ground truth, superseded roadmap line).

## Follow-ups

- OPEN-29 re-read under no-gate premise (author).
- Initial-page optimization pass ("later") has the reporter as baseline;
  re-adding a gate then is a small reversal of this commit.
