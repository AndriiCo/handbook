# Prompt 047 — OPEN citation rule + numbering question

- Prompt timestamp (UTC): 2026-10-28T20:22:05Z
- Response timestamp (UTC): 2026-10-28T20:24:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Then asking for operator outstanding questions like OPEN-32 (why not 42?) - please also cite the file there it's located. Save this instructions to AGENTS.md
```

## Response

Saved as a §6 "Decisions stick" rule: outstanding-question prompts
must cite file + section, e.g. "OPEN-32
(`docs/specs/p2-forms-labels.md` §9)". Open questions still carry
number + owner + blocked stream; the citation is now a fourth
requirement.

Numbering answer (verified by sweep): OPEN numbers are sequential by
creation — 25/26/27/28 in `customer-auth.md`, 29/30/31 in
`ui-framework-proposal.md`, 32 in `p2-forms-labels.md` §9. There is
no 42; 32 is simply the latest opened. (Unrelated sequences: D38
decisions, 046 prompt log.)

## Commits

- `24db211` AGENTS.md: cite file+section when asking about OPEN items
- (this log entry commits separately, per convention)

## Verification

- Docs-only change; code tree identical to the CI-green `88562bb`
  state (intervening commits: prompt logs only). Staged-hook
  secrets-scan clean.
