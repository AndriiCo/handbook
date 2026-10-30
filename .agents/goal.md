# Goal brief: Apostille Handbook, Meta AI hackathon (until 2026-11-04 18:00 PT)

This file and `.agents/scoreboard/` are guardrails. Run them, never edit
them. If a row's contract below is wrong for the product, say so in your
final message and keep working on other rows. The operator edits this file.

## The goal (pasted into the builder session; `/goal edit` replaces it)

```text
/goal Make `node .agents/scoreboard/scoreboard.mjs` exit 0 against production (https://andrii.ai/apostille), every change proven on staging first. Each slice: read the newest .muse/worktrees/*/web/shots/judge.md; take the first red row in .agents/goal.md that still has builder work (skip a row that waits only on a line in docs/ops/operator-today.md); load the skill it names; finish, commit, submit the SHA to plant, run staging acceptance (AGENTS.md §8) and report the staging and production scores. When no red row has builder work, take the next open item under "While rows wait on the operator". Never mark the goal blocked while that list has an open item. Fan independent UI work out to at most 3 write-capable subagents with worktree_isolation, under web/ only. Never edit .agents/ or .muse/.
```

## Deploy days

Production changes only when the operator runs the day's one deploy line
(AGENTS.md §7). Your part is a SHA with a plant stamp and a green staging
run, plus the exact lines in `docs/ops/operator-today.md`. A row is
finished only when it passes on production.

| Day | Deploy | What it must carry |
|-----|--------|--------------------|
| Fri 10-30 | 70960d4 (stamped) | P100-P115. No root scripts today: they wait on fixes 2, 3 and 5. |
| Sat 10-31 | #2 | Fixes 1-6. Then the operator runs proposals, A2 and A5. |
| Sun 11-01 | #3 | Fixes 7-11; first dry run of the demo against production. |
| Mon 11-02 | #4, the last code deploy | Dry-run fixes. The operator re-runs A5 so it is fresh through judging. |
| Tue 11-03 | rollback only | Recording, README video link. |
| Wed 11-04 | freeze | The operator submits by 12:00 PST (close 18:00 PST = 02:00 UTC 11-05). |

Prove rows on staging (AGENTS.md §8: `ssh plant`, the staging stack seeded
from packs, `--base` the tunnelled URL) or, for screens only, the dev
server with fixtures:
`node .agents/scoreboard/scoreboard.mjs --base <staging or http://localhost:5173/apostille> --only <row>`.

## While rows wait on the operator

A row that is code-complete but red on production is not a reason to
stop: work this list, top first, and never mark the goal blocked while an
item is open.

1. Plant gate: `ops/plant-pipeline/submit.sh <sha>` for every commit you
   want deployed, so it has an `acceptance/<sha12>.json` stamp with
   `ci_green: true`. Poll with `status.sh`; never wait on it.
2. Staging acceptance (AGENTS.md §8): stand up the staging stack on plant,
   seed vietnam and usa-wa-sos from the packs, fetch, analyze, approve on
   staging, and run the whole scoreboard against it. You write
   `docs/ops/staging.md` (ports, tunnel, regeneration recipes) without
   asking. A row green on staging is the evidence the next deploy will
   turn it green.
3. Demo (skill `ship-it`): `docs/demo-script.md` today, then
   `web/e2e/demo.spec.mjs` and `npm --prefix web run demo` (the gate
   AGENTS.md §4 and §7 name) by 10-31, against staging or fixtures. You
   record the public segments; the operator records the production
   `/review` segment and uploads.
4. The judges' README (row S1, everything except the operator's licence,
   visibility and video steps).
5. The operator's review below, top first.
6. `/taste` findings you listed but did not fix.

## Operator's review of P100-P116 (2026-10-30), fix in this order

Five reviewers read 0c8ea63..801b2a3; a second reader tried to refute each
finding and none fell. Use the ids in log lines.

1. `plant-submit-guard`: `ops/plant-pipeline/submit.sh` refuses whenever
   a tracked file is modified, even with an explicit SHA it archives from
   git, and the session-log hook rewrites
   `docs/prompts/sessions/README.md` after every turn. Check the tree only
   for the HEAD default, and exclude `docs/prompts/sessions`.
2. `packs-coverage`: neither pack can yield 8 genuine cited facts.
   `packs/usa-wa.json`'s only fetchable watch URL is http (A1 needs https)
   and its PDF is a 404; vietnam watches one row of the HCCH status table.
   Add official https pages until each slug has the text for 8 facts
   (`cargo xtask packs-validate`). Make `ops-gp2-proposals.sh` always run
   packs-load and retire a source row whose URL changed: facts approved
   from an http capture would fail A1 for good.
3. `live-draft-first`: no real draft call has ever been made. Give
   `live_client` the `$META_API_KEY` fallback `atlas.rs` has; raise
   `max_completion_tokens` from 2000 (Spark's reasoning counts inside it,
   and `finish_reason=length` becomes BadOutput); on length or a refused
   quote, retry once with fewer records and the refusal reason
   (legal-agent-workshop `quote_on_page`). Prove it on staging with the
   staging key (AGENTS.md §8), save the replies as fixtures, report tokens
   and p50/p95.
4. `a1-integrity`: accepting a fact requires a verified citation; a quote
   is 8-400 characters (the oracle now checks this); `source_url` must be
   https and is never rendered as a raw href (a `javascript:` or `mailto:`
   URI in a pack would reach the public dossier).
5. `ops-scripts-runnable`: `evals/` never reaches a release, so
   `ops-a5-eval.sh` aborts; `ops-a2-record.sh` looks for PNGs inside the
   `web` binary (use `$RELEASE/spa/demo`), sends a SAM request in a shape
   that contradicts `meta-api-shapes.md`, and takes the key on the command
   line (use `read -rs`); runbook-release §2 stages neither the `SHA` file
   nor `apostille.service`, which `ops-deploy.sh` requires. Then write one
   Mac-side `docs/ops/scripts/ship-release.sh <sha>` that checks the plant
   stamp, builds, stages and prints the deploy line, so the operator's day
   is two lines. Rewrite `operator-today.md` to match.
6. `security-before-judges`: `/api/v1/demo/login` mints a 24 h session
   for any `customer_ref`, including a real account's user id, which then
   reads that customer's submissions, PDFs, labels and WS topics (review
   H4, still unfixed). Accept only `demo-`-prefixed refs, which no real
   account can have. Give `/demo/precheck` its own rate-limit map (it
   shares auth's, which fails closed above 8192 keys, so a flood locks
   out sign-in). Refuse WS topics for slugs that do not exist.
7. `dossier-all-topics`: the dossier renders only the first topic tab, so
   GP2-page sees fewer than 8 statements. Render every topic (tabs become
   anchors); make quote, source and date reachable by tap and keyboard (a
   phone cannot hover) with a visible verified mark; show "approved by
   staff" and a readable `drafted_by` on each fact; on a live update keep
   the tab and highlight the new fact.
8. `meta-visible`: no public page names Muse Spark or SAM. Add a "How it
   works" band on home (Spark drafts from watched official pages, the
   quote is checked byte-exact, a human approves, SAM 3.1 + Spark
   pre-check photos), the eval's model and run date next to the percent,
   a link to `/precheck`, and "replayed from a live run on <date>"
   wherever a replay is shown.
9. `visual-pass`: 21 bare `<h1>` render as body text (login, order,
   precheck, review, console); `/review` buttons and inputs are unstyled,
   and it is the "approved by a human" shot; precheck animates the SAM
   boxes below the fold and clips the top labels; home's featured chips
   fall back to slugs that 404 on production; signed-out visitors get the
   7-link account sub-nav; the footer mark is a white tile in dark mode;
   login is a 1100 px left-aligned stack (center it, passkey first,
   provider logos).
10. `a5-honest`: `quote_found_rate`, `value_in_quote_rate` and
    `topic_accuracy` are identical by construction; compute each from its
    own check. The oracle now refuses a non-Spark or zero-rate run.
11. `gp3-conditional-mediation`: the `username webauthn` attribute is
    there, but `navigator.credentials.get({mediation: 'conditional'})` is
    never called.
12. `camera-safe-draft`: the "Spark drafts from a new capture" scene has
    no trigger on production (an unchanged page yields no capture). Add an
    admin "Draft from this capture" action (mTLS) that runs one live draft
    and streams its steps; that is also A4's minimal trace view.
13. `readme-promise`: README promises order → completed form → FedEx
    label, and `/order` saves nothing. Assumed: rewrite the success
    criteria to what is live and call the order leg roadmap; wire `/order`
    only when 1-12 are done.
14. `log-in-same-commit`: AGENTS.md §5. The LOG.md line goes in the
    slice's commit; P100-P116 each have a separate "log line" commit.

## Row order and skills

| # | Row | Skill | What "pass" needs (contract) |
|---|-----|-------|------------------------------|
| 1 | P0 | (none) | `/healthz` 200. Never regress it. |
| 2 | GP2-data | `ai-slice` | `GET /api/v1/authorities` lists `vietnam` and `usa-wa-sos`, each with `fact_count >= 8`. Facts reach "accepted" only when the operator approves them in `/review`; queue the proposals, then list "approve N proposals" in operator-today.md. |
| 3 | A1 | `ai-slice` | `GET /api/v1/authorities/{slug}/facts`: every fact has `citations: [{quote, source_url, captured_at}]` (quote 8-400 chars, found on the captured page; `https` URL; ISO date) and `drafted_by` (e.g. `"muse-spark-1.3"`). Pattern: legal-agent-workshop `quote_on_page`, refused quote returned to the model, one retry. |
| 4 | GP1 | `ship-screen` | Home: body font is a real typeface (`@fontsource-variable`), a search affordance (`role=combobox` or `[aria-keyshortcuts]` for ⌘K), >= 3 links to `/authority/…`, zero console errors, no overflow at 390 px, none of: placeholder, not wired, skeleton, lorem ipsum, TODO, coming soon. |
| 5 | GP2-page | `ship-screen` | `/authority/vietnam`: h1 is the authority's name, not the slug; accepted fact statements are on the page (all topics, not one tab); the page opens `/ws/bus?topic=authority:vietnam`. Each fact shows its quote, source and date one tap or hover away (AGENTS.md §3), reachable on a phone. |
| 6 | GP3 | `ship-screen` | `/login` at 390x844: `input[autocomplete="username webauthn"]` with conditional mediation, a "Sign in with passkey" button above the fold, `GET /api/v1/auth/providers` → `{"providers":[…]}` listing only providers configured in production, and one button with the official logo per listed provider. |
| 7 | GP4 | `ship-screen` | `/order`: a stepper marked with `aria-current="step"`, clean at 1440 and 390. |
| 8 | A2 | `ai-slice` | `GET /api/v1/demo/precheck/samples` → `{"samples":[{"id","label","thumb_url"}]}` (synthetic documents only); `POST /api/v1/demo/precheck {"sample":id}` → `{"verdict":"pass"|"retake","issues":[…],"models":["sam-3.1","muse-spark-1.3"]}`, rate-limited and cost-capped; `/precheck` animates the SAM boxes and the verdict, and says when the replayed run was recorded. |
| 9 | A5 | `ai-slice` | `GET /api/v1/quality` → `{run_at, gold_count >= 25, quote_found_rate > 0, value_in_quote_rate, topic_accuracy, model}` from a live `muse-spark` eval run in the last 72 h; home shows `quote_found_rate` as a whole percent with the model and date. |
| 10 | S1 | `ship-it` | MIT or Apache-2.0 LICENSE, a README for judges with a YouTube/Vimeo/Loom demo link and "built with Muse Code", public repo. **The operator makes the repo public and switches the licence; prepare both and list them.** |

Not on the scoreboard because no public URL can show them: the GP5 review
queue (approve → GP2 updates live) and the A4 agent trace console. Both are
in the demo video (fix 12 gives A4 its first view).

## Run rules

- One row or one fix per slice. A slice ends green and committed
  (`P0NN: <row or fix id>: …`) with its LOG.md line in the same commit.
- Subagents work under `web/` only and never run cargo in their worktree:
  the Mac's disk is 98% full. Rust gates run in the main checkout or on
  plant. Children own only their route files; the parent owns `app.css`,
  layouts, `package*.json` and merges.
- AI rows (2, 3, 8, 9) report a live run: model, p50/p95 latency, tokens,
  and save the recorded responses as replay fixtures.
- The judge session writes `web/shots/judge.md` in its worktree. It is
  review input, not orders: fold its top fixes into the current or next
  slice. It can view your PNGs; you cannot.
- Deploys: the operator deploys one plant-stamped, staging-green SHA a day
  (AGENTS.md §7). Anything that needs root goes into
  `docs/ops/operator-today.md` as one script.

## Operator-only steps (the agent cannot do these)

- Run the day's deploy line. Run the root scripts (proposals, A2
  recording, A5 eval) only after the fixes they need have shipped (2, 3, 5).
- Approve proposals in `/review`: at least 8 per slug, keeping at least 2
  per slug proposed for the video's approve scene.
- Install a staging-only Meta key on plant (AGENTS.md §8), so the first
  live calls happen off production.
- Provide Apple/Google/Meta OAuth credentials, or drop those providers
  from `/api/v1/auth/providers`; put the official brand assets for each
  configured provider in `web/src/lib/assets/brand/`.
- Copy the official Cerebral Valley submission rules (form fields, video
  length and host, repo and licence terms) into row S1 above.
- Record the production `/review` segment, upload the video, switch the
  licence, make the repo public once `docs/ops/` is scrubbed, and submit
  by 11-04 12:00 PST.
