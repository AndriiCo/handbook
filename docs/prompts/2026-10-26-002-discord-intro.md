# 002 — Discord intro message

- Prompt timestamp (UTC): 2026-10-26 ~17:10 (approximate, same session)
- Response timestamp (UTC): 2026-10-26 17:13:04
- Author: Andrii Tereshchenko
- Agent/model: Muse Code, powered by Meta Muse Spark

## Prompt

Make an intro message to Discord on behalf of Andrii Co. and our project. You can include link to andrii.co website and details from my https://www.linkedin.com/in/tagua/

## Response

Drafted a Discord intro message (delivered in chat, copy-paste ready; not
stored elsewhere in the repo since only the message was asked for).
Sources used:

- `https://andrii.co` — fetched successfully (200 OK): mobile notary &
  apostille practice in Bothell, WA, serving Greater Seattle and all of
  Washington State.
- `https://www.linkedin.com/in/tagua/` — behind a sign-in wall; only
  public headline facts visible ("Andrii Tereshchenko — Andrii Co.",
  Greater Seattle Area, 2K followers, 500+ connections). No employment
  history or other details were claimable from it, so background details
  (ex-Microsoft LUIS.AI, Build 2022) were taken from the user's own
  hackathon application text instead, plus the GitHub/X links from there.

Message text (verbatim as delivered):

```text
👋 Hi everyone, I'm Andrii Tereshchenko — founder of Andrii Co. (https://andrii.co), a mobile notary & apostille practice in Bothell, WA, and a Startup Track hacker here.

📜 My project: the Apostille Handbook — one source for every country's apostille details and forms, kept current, with prepaid labels and envelopes delivered to your door. Starting with 106 apostille authorities across the US, Canada, and Mexico.

💡 Why: government sites change quietly and real people pay the price. Washington's apostille counter moved from Olympia to Tumwater in June 2025, yet searches still surface superseded forms; when the Apostille Convention took effect in Viet Nam (Sept 2026), even our own site briefly sent customers down the wrong path.

🛠️ The 10-day plan: automated upkeep — watching 500+ cited URLs plus HCCH feeds, with Muse Spark turning changes into cited records; one consular legalization chain (Egypt in USA); walk-in reports with SAM 3.1 masking; everything served through the handbook and an MCP server. Rust stack, built with Muse Code.

🙋 About me: previously Microsoft (LUIS.AI, featured at Build 2022); recently shipped a Rust booking/MCP platform for my notary practice (~5,250 tests, self-hosted OAuth 2.1, Gemini voice interpreter in Spanish). More: https://www.linkedin.com/in/tagua/ | https://github.com/tagua | https://x.com/andrii_us

Happy to trade notes on MCP servers, Rust, or keeping AI-generated content accurate — DMs open!
```

Files changed: `docs/prompts/2026-10-26-002-discord-intro.md` only.
No code changes; no test/build verification applicable. No push (not asked
for this turn, per AGENTS.md §5).

## Commits

- This entry's own commit: the commit adding this file (see
  `git log -- docs/prompts/2026-10-26-002-discord-intro.md`).

## Follow-ups

- None. Design still arrives as a later prompt.
