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
