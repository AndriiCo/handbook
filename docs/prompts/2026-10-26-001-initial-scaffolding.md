# 001 — Initial scaffolding

- Prompt timestamp (UTC): 2026-10-26 ~16:52 (session start, approximate)
- Response timestamp (UTC): 2026-10-26 17:05:35
- Author: Andrii Tereshchenko
- Agent/model: Muse Code, powered by Meta Muse Spark

## Prompt

Add a readme file for Apostille handbook project for https://cerebralvalley.ai/e/meta-ai-developer-hackathon.md based on this application text "Your Application
What's your LinkedIn?
https://www.linkedin.com/in/tagua/
What's your Twitter/X?
https://x.com/andrii_us
What's your Github?
https://github.com/tagua
Where do you live (City, State, Country)?
Bothell, WA, USA
Are you applying for the startup track or indie dev track? (you can change your track anytime)
Startup Track
What country is your company incorporated in?
United States
What round of funding are you?
Bootstrapped
What do you want to build for this hackathon?
People send documents overseas once every few years, so they shouldn't have to study government sites that change quietly. Washington's apostille counter left Olympia for Tumwater in June 2025, yet searches still surface superseded forms. On 13 October 2026 even our own site sent Viet Nam customers to consular legalization, though the Apostille Convention took effect there on 11 September.

The Apostille Handbook by Andrii Co. aims to be one source for every country: every detail and form, then prepaid labels and envelopes delivered to your door, kept current so nobody plans a visit around a two-year-old address or mails the wrong form. It plan to covers all authorities on Earth, but we will start 106 apostille authorities in the U.S., Canada and Mexico.

In 10 days, with Muse Code, we'll automate that upkeep for these three countries first, then add others (Build for Your World):
- days 1-4: watch the handbook's 500+ cited URLs, HCCH's feed and an inbox for authority notices; Muse Spark turns only changes into cited records I approve, reading English, Spanish or French pages and forms to flag contradictions like Washington's two PO Boxes
- 5-6: one consular legalization chain (Egypt in USA) with pre-filled authority forms
- 7-8: walk-in reports (route photos, parking tips, rejections) update the handbook, SAM 3.1 masking faces and plates; self-hosted Muse Glimmer pre-checks document photos
- 9-10: approved changes reach the handbook and MCP server; a new country means new sources, not new code,

Success criteria: synthetic-document demo: a customer can request apostille for Viet Nam online in very convenient way and envelopes with completed form will be delivered to his house via Fedex tomorrow.
Tell us about an AI project you recently built that showcases your technical skills. (hint: Include what you built, the techniques you used)
After my mother, Victoria Tereshchenko (1956-2026), passed away this summer, for a while I could not talk to people due to grief. My customers still needed a notary, so I vibe-coded a way for them to get one without a phone call, directing AI coding agents under production guardrails.

Within 90 days of the first commit, a customer could: pick a live 15-minute slot with a running price estimate; sign in with a passkey or Apple; pay online; upload documents and track the order; mail in a certificate for an apostille with no meeting; or ask an AI assistant like Claude or ChatGPT to request a booking.
In her memory, estate and inheritance documents now cost $1.99 per order, notarized, apostilled and shipped anywhere, until June 2027 or until $50,000 has been given.

Those 90 days held 1,100 commits, 60% with Claude co-author trailers. Under the hood: a Rust backend (~549k lines of Rust in the tree today, ~5,250 tests).

AI booking: an MCP server (15 tools, official registry), stateless Streamable HTTP. AI-made bookings are requests I confirm, never payments. Its self-hosted OAuth 2.1 uses PKCE, Client ID Metadata Documents and refresh-token families with reuse revocation.

Voice: a Gemini phone interpreter for Spanish language at +1(425)800-8688

Earlier: Microsoft LUIS.AI, a service on customized large language models featured at Build 2022.

Links:
https://andrii.co/en/notary/booking/
https://andrii.net/mcp
https://andrii.co/en/notary/in-loving-memory/
Have you used Meta Model API before?
Yes
How likely are you to build with Meta Model API?
Very likely
How would you describe your overall opinion of Meta Model API?
Somewhat favorable
How did you hear about this hackathon?
Muse Code
By selecting Yes, you agree to receive marketing related electronic communications from Meta, including news, events, updates, and promotional emails. You may withdraw your consent and unsubscribe from these at any time, for example, by clicking the unsubscribe link included on our emails.

For more information about how Meta handles your data please read our Meta Data Policy
Yes". We will retain the copyright to the source code and collective work (handbook) produced - make correct license file for this. As part of nice features - you will also retain the complete log of all prompts and responses generated with timestamps and others details. You will make code commits after each significant change. In case there will need to rollback - make a rollback commits and explain why.  You will use following AGENTS.md # AGENTS.md

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

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes. as your guideline and will keep it updated (for example include instructions on keeping prompts and responses in some lod in MD / nicely formatted output)  if needed for succesfull project completition. The project stack will be Rust based. Do not write shell scripts, but small utilities using xtask create to keep reuse of the code/libaries/techinical decigions. Make a first commit now and push to github. We will do design as next prompt.

## Response

Scaffolded the project and committed it. Decisions and assumptions:

- Workspace layout: root `Cargo.toml` workspace with `crates/handbook`
  (library, smoke test only — design arrives in a later prompt) and `xtask/`
  (task runner with `help` and `ci`; `cargo xtask ci` runs fmt check,
  clippy `-D warnings`, and tests). No external dependencies, edition 2024.
- `README.md` covers the hackathon link, problem, vision, 10-day plan, and
  success criteria, all drawn from the application text; no details invented.
- `LICENSE` is a short proprietary license: Copyright (c) 2026 Andrii Co.,
  all rights reserved, covering both the Software and the Collective Work
  (handbook content), with viewing allowed for evaluation (including
  hackathon judging) and all other uses requiring written permission.
- `AGENTS.md` keeps the given guidelines verbatim and adds §5 project
  instructions: Rust/xtask-only tooling, `cargo xtask ci` gate before
  commits, commit-after-significant-change with explained rollback commits,
  the `docs/prompts/` log requirement, and keeping `AGENTS.md` itself
  updated.
- Prompt log: one Markdown file per prompt under `docs/prompts/`, format
  specified in `docs/prompts/README.md`. This entry is committed separately
  so it can record the real hash of the work commit it describes.
- `Cargo.lock` is committed (workspace contains the `xtask` binary).

Files changed: `Cargo.toml`, `.cargo/config.toml`,
`crates/handbook/Cargo.toml`, `crates/handbook/src/lib.rs`,
`xtask/Cargo.toml`, `xtask/src/main.rs`, `README.md`, `LICENSE`,
`AGENTS.md`, `docs/prompts/README.md`, `Cargo.lock`,
`docs/prompts/2026-10-26-001-initial-scaffolding.md`.

Verification (all observed this turn):

- `cargo build --workspace` — success.
- `cargo test --workspace` — 1 passed, 0 failed.
- `cargo fmt --all --check` — clean after `cargo fmt --all`.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo xtask ci` — passes end to end.
- `git status` — clean after commits; `git push` to `origin/main` — success.

## Commits

- `10c4e57aabf88e6fb209b433d3cd658f25cfd043` — Scaffold project: README,
  LICENSE, AGENTS.md, Rust workspace with xtask (work commit).
- This entry's own commit: the commit adding this file (see
  `git log -- docs/prompts/2026-10-26-001-initial-scaffolding.md`).

## Follow-ups

- Design arrives as the next prompt.
