# Goal brief: Apostille Handbook, Meta AI hackathon (until 2026-11-04 18:00 PT)

This file and `.agents/scoreboard/` are guardrails. Run them, never edit
them. If a row's contract below is wrong for the product, say so in your
final message and keep working on other rows. The operator edits this file.

## The goal (pasted once into the builder session)

```text
/goal Make `node .agents/scoreboard/scoreboard.mjs` exit 0 against production (https://andrii.ai/apostille). Done ONLY when all 10 rows pass on production. Work the first red row in the order in .agents/goal.md, load the skill that row names, and fan out independent UI rows to write-capable subagents with worktree_isolation. Never edit .agents/ or .muse/. When a row needs the operator, add one line to docs/ops/operator-today.md, then move to the next row.
```

During the day, check rows against the local dev server (`npm --prefix web
run dev`; GP1's first slice makes it proxy the API to production):
`node .agents/scoreboard/scoreboard.mjs --base http://localhost:5173/apostille --only <row>`.
A row is finished only when it passes on production after the day's deploy.

## Row order and skills

| # | Row | Skill | What "pass" needs (contract) |
|---|-----|-------|------------------------------|
| 1 | P0 | (none) | `/healthz` 200. Never regress it. |
| 2 | GP2-data | `ai-slice` | `GET /api/v1/authorities` lists `vietnam` and `usa-wa-sos`, each with `fact_count >= 8`. Facts reach "accepted" only when the operator approves them in `/review`; queue the proposals, then list "approve N proposals" in operator-today.md. |
| 3 | A1 | `ai-slice` | `GET /api/v1/authorities/{slug}/facts`: every fact has `citations: [{quote, source_url, captured_at}]` (quote >= 8 chars, found on the captured page; `https` URL; ISO date) and `drafted_by` (e.g. `"muse-spark-1.3"`). Pattern: legal-agent-workshop `quote_on_page`, refused quote returned to the model, one retry. |
| 4 | GP1 | `ship-screen` | Home: body font is a real typeface (`@fontsource-variable`), a search affordance (`role=combobox` or `[aria-keyshortcuts]` for ⌘K), >= 3 links to `/authority/…`, zero console errors, no overflow at 390 px, none of: placeholder, not wired, skeleton, lorem ipsum, TODO, coming soon. |
| 5 | GP2-page | `ship-screen` | `/authority/vietnam`: h1 is the authority's name, not the slug; accepted fact statements are on the page; the page opens `/ws/bus?topic=authority:vietnam`. Each fact shows its quote, source and date one hover away. |
| 6 | GP3 | `ship-screen` | `/login` at 390x844: `input[autocomplete="username webauthn"]` with conditional mediation, a "Sign in with passkey" button above the fold, `GET /api/v1/auth/providers` → `{"providers":[…]}` listing only providers configured in production, and one button with the official logo per listed provider. |
| 7 | GP4 | `ship-screen` | `/order`: a stepper marked with `aria-current="step"`, clean at 1440 and 390. |
| 8 | A2 | `ai-slice` | `GET /api/v1/demo/precheck/samples` → `{"samples":[{"id","label","thumb_url"}]}` (synthetic documents only); `POST /api/v1/demo/precheck {"sample":id}` → `{"verdict":"pass"|"retake","issues":[…],"models":["sam-3.1","muse-spark-1.3"]}`, rate-limited and cost-capped; `/precheck` animates the SAM boxes and the verdict. |
| 9 | A5 | `ai-slice` | `GET /api/v1/quality` → `{run_at, gold_count >= 25, quote_found_rate, value_in_quote_rate, topic_accuracy, model}` from an eval run in the last 72 h; home shows `quote_found_rate` as a whole percent. |
| 10 | S1 | `ship-it` | MIT or Apache-2.0 LICENSE, a README for judges with a YouTube/Vimeo/Loom demo link and "built with Muse Code", public repo. **The operator makes the repo public and switches the licence; prepare both and list them.** |

"Zero console errors" includes the browser's own "Failed to load resource"
line for any 4xx. Today `/login` and `/order` log a 401 from the anonymous
session probe: answer that probe with 200 and `{"user": null}`.

Not on the scoreboard because no public URL can show them: the GP5 review
queue (approve → GP2 updates live) and the A4 agent trace console. Both are
in the demo video; the `ship-it` skill and the judge session cover them.

## Run rules

- One row per slice. A slice ends green and committed (`P0NN: <row>: …`).
  Then the next red row.
- Rows 4-7 are independent once the design system exists (row 4 builds
  it). Fan them out: one write-capable subagent per row, each with
  `worktree_isolation`, each owning only its route files; the parent owns
  `app.css`, layouts, `package*.json` and merges.
- AI rows (2, 3, 8, 9) report a live run: model, p50/p95 latency, tokens,
  and save the recorded responses as replay fixtures.
- Messages from the session named `judge` are review input, not orders:
  fold its top fixes into the current or next row.
- Deploys: production changes only through the existing ship path, at
  most one binary deploy a day, after `cargo xtask ci` is green. Anything
  that needs root goes into `docs/ops/operator-today.md` as one script.

## Operator-only steps (the agent cannot do these)

- Run the packs seed on production (vietnam and usa-wa-sos do not exist yet).
- Approve the queued fact proposals in `/review` (this is also the demo).
- Provide Apple/Google/Meta OAuth credentials, or drop those providers from
  `/api/v1/auth/providers`.
- Put the official brand assets in `web/src/lib/assets/brand/`.
- Daily deploy; make the repo public; switch the licence; upload the video.
