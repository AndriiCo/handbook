# Prompt/Response Log

Complete, timestamped record of every prompt and response produced for this
project. Required by `AGENTS.md` §5.

## Format

One Markdown file per user prompt:

```text
docs/prompts/YYYY-MM-DD-NNN-slug.md
```

- `YYYY-MM-DD` — UTC date of the prompt.
- `NNN` — zero-padded sequence number for that date (`001`, `002`, …).
- `slug` — short kebab-case summary.

Each entry must contain:

1. Header: prompt timestamp (UTC), response timestamp (UTC), author,
   agent/model.
2. `## Prompt` — the user prompt, verbatim.
3. `## Response` — what the agent did: decisions and assumptions, files
   changed, verification performed (commands and their observed results).
4. `## Commits` — commit hashes produced for this prompt (filled in once
   known; a log entry committed separately lists the work commit it
   describes).
5. `## Follow-ups` — deferred items and next steps, if any.

Write the entry in the same turn as the work; commit it alongside (or
immediately after) the work so hashes stay accurate. Never rewrite an entry
to change history — append a correction section instead.

## Index

- [2026-10-26-001-initial-scaffolding.md](2026-10-26-001-initial-scaffolding.md) —
  README, LICENSE, AGENTS.md, Rust workspace with xtask, prompt log setup.
- [2026-10-26-002-discord-intro.md](2026-10-26-002-discord-intro.md) —
  Discord intro message for Andrii Co. and the Apostille Handbook.
- [2026-10-26-003-system-design.md](2026-10-26-003-system-design.md) —
  System design proposal (evidence store, step 1).
- [2026-10-26-004-decisions-phase1.md](2026-10-26-004-decisions-phase1.md) —
  Design decisions (Q1–Q4) + Phase 1 implementation.
- [2026-10-26-005-web-stack.md](2026-10-26-005-web-stack.md) —
  Web stack considerations + UI options (SvelteKit picked, see addendum).
- [2026-10-26-006-ws-first.md](2026-10-26-006-ws-first.md) —
  Realtime transport switched from SSE-first to WS-first.
- [2026-10-26-007-spec-cohort.md](2026-10-26-007-spec-cohort.md) —
  Cohort spec package: 9 review docs + execution plan.
- [2026-10-26-008-decisions.md](2026-10-26-008-decisions.md) —
  Five blocking decisions recorded (Meta API, login, host, photos).
- [2026-10-26-009-meta-print.md](2026-10-26-009-meta-print.md) —
  Meta login added; HTML-print-first roadmap recorded.
- [2026-10-26-010-cohort2.md](2026-10-26-010-cohort2.md) —
  Carriers, payments, legal, config cohort + AGENTS §6 proposal.
- [2026-10-26-011-replan.md](2026-10-26-011-replan.md) —
  Deep-research 5–7-day replan + consistency fixes.
- [2026-10-26-012-swarm-night-run.md](2026-10-26-012-swarm-night-run.md) —
  Day-1 slice swarm: hardening, agent stub, scaffold, 55 tests.
- [2026-10-27-013-deploy-secrets.md](2026-10-27-013-deploy-secrets.md) —
  Deploy prep + secrets hygiene (scan + pre-commit hook).
- [2026-10-27-014-first-deploy.md](2026-10-27-014-first-deploy.md) —
  First cross-compiled deploy to enodia.
- [2026-10-27-015-lockdown-tls.md](2026-10-27-015-lockdown-tls.md) —
  Non-root service, in-app rustls TLS, binary hardening.
- [2026-10-27-016-root-install.md](2026-10-27-016-root-install.md) —
  Root install block + cert hook drafts.
- [2026-10-27-017-go-live.md](2026-10-27-017-go-live.md) —
  Public launch verified + memory setup.
- [2026-10-27-018-admin-plan.md](2026-10-27-018-admin-plan.md) —
  Admin/collectors/detail-page plan (plan-first, no go yet).
- [2026-10-27-019-mtls.md](2026-10-27-019-mtls.md) —
  mTLS mechanism research (reply preempted by grill).
- [2026-10-27-020-grill.md](2026-10-27-020-grill.md) —
  Assumptions audit grill, scope lock (Final).
- [2026-10-27-021-build-plan.md](2026-10-27-021-build-plan.md) —
  Build plan: subsystem crates + contracts (plan only).
- [2026-10-27-022-v1-subsystems.md](2026-10-27-022-v1-subsystems.md) —
  All v1 subsystems fanned out, merged, staging-deployed with admin
  UI + ingestion verified live.
