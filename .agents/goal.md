# Goal brief: Apostille Handbook, Meta AI hackathon (until 2026-11-04 18:00 PT)

This file, `.agents/roles/`, `.agents/scoreboard/` and `.agents/skills/`
are guardrails. Run them, never edit them; propose changes in your final
message. The operator edits them. History before 2026-11-01 (the 10-30
review list, the old deploy recipe) is in `.agents/archive/`.

## Objective

Make `node .agents/scoreboard/scoreboard.mjs` exit 0 against production
(https://andrii.ai/apostille), every change proven on staging first, and
make the 3-minute demo show Meta models doing visible, measured work on
real data.

How each session is started (one file reference each; the SessionStart
hook repeats it on every resume and compaction):

| Session | Start | Scheduled |
|---|---|---|
| builder (main checkout) | `/goal Follow .agents/roles/builder.md` | `/loop "*/30 * * * *" Continue: follow .agents/roles/builder.md` |
| lane (`muse -w`) | `/goal Follow .agents/roles/lane-<name>.md` | none |
| judge (`muse -w`) | `Run /judge-eyes now.` | `/loop "0 */2 * * *" /judge-eyes` |

## Lanes

| Lane | Role file | Session / worktree / branch | State (2026-11-01 08:53) |
|---|---|---|---|
| `meta` | `.agents/roles/lane-meta.md` | `01a19798-66eb-72b0-8ce7-3e80f3958724`, `.muse/worktrees/20261101-7e99`, `muse/session-01a19798-66eb-72b0-8ce7-3e80f3958724` | F1-F14 fixed on the branch (P184-P195), not merged; follow-ups listed in its role file |

Any other `muse/session-*` branch (the judges') is not a lane: never merge it.

## State (2026-11-01 08:00 PST; the SessionStart hook prints the last full runs)

Production `0.1.0+69ca7fa`: **5/10** (P0, GP1, GP4, A2, A5). Staging:
**9/10** (only S1 red). Main is ahead of production by docs, ops and
README commits (P180-P183).

| # | Row | Prod | Skill | Who moves it now | Contract ("pass" needs) |
|---|-----|------|-------|------------------|-------------------------|
| 1 | P0 | pass | (none) | builder: never regress | `/healthz` 200. |
| 2 | GP2-data | FAIL | `ai-slice` | operator approves in `/review` (the 11-01 01:41Z proposals run left usa-wa-sos 17 proposed, vietnam 0); builder: if the operator reports vietnam still empty, list the `ops-gp2-proposals.sh` line (as hal) | `GET /api/v1/authorities` lists `vietnam` and `usa-wa-sos`, each with `fact_count >= 8`. Facts reach "accepted" only through the operator's `/review` approvals. |
| 3 | A1 | FAIL | `ai-slice` | follows GP2-data | `GET /api/v1/authorities/{slug}/facts`: every fact has `citations: [{quote, source_url, captured_at}]` (quote 8-400 chars, found on the captured page; `https` URL; ISO date) and `drafted_by` (e.g. `"muse-spark-1.3"`). |
| 4 | GP1 | pass | `ship-screen` | builder: never regress | Home: real typeface, a search affordance (`role=combobox` or `[aria-keyshortcuts]`), >= 3 links to `/authority/…`, zero console errors, no overflow at 390 px, no banned strings. |
| 5 | GP2-page | FAIL | `ship-screen` | follows GP2-data | `/authority/vietnam`: h1 is the authority's name; accepted fact statements on the page (all topics); opens `/ws/bus?topic=authority:vietnam`; each fact's quote, source and date one tap or hover away. |
| 6 | GP3 | FAIL | `ship-screen` | operator adds the official `google.svg` + `apple.svg` to `web/src/lib/assets/brand/` and commits them (they ship in the public bundle; the folder is not git-ignored); then the next deploy | `/login` at 390x844: `input[autocomplete="username webauthn"]` with conditional mediation, a "Sign in with passkey" button above the fold, `GET /api/v1/auth/providers` → `{"providers":[…]}` (production: google, apple), one button with the official logo per provider. |
| 7 | GP4 | pass | `ship-screen` | builder: never regress | `/order`: a stepper marked with `aria-current="step"`, clean at 1440 and 390. |
| 8 | A2 | pass | `ai-slice` | operator re-runs `ops-a2-record.sh` after the `meta` lane's SAM fix (F4) is deployed; builder stages the recorder and lists the line | `GET /api/v1/demo/precheck/samples`; `POST /api/v1/demo/precheck {"sample":id}` → `{"verdict","issues","models":["sam-3.1","muse-spark-1.3"]}`, rate-limited; `/precheck` says when the replay was recorded. |
| 9 | A5 | pass | `ai-slice` | operator re-runs `ops-a5-eval.sh` Monday evening whether or not Monday's deploy happens, and again Wednesday morning: the current run (11-01 04:29Z) leaves the 72 h window Tue 11-03 20:29 PST | `GET /api/v1/quality` → `{run_at, gold_count >= 25, quote_found_rate > 0, …, model}` from a live `muse-spark` eval in the last 72 h; home shows the percent with model and date. Current: quote_found 1.0, value_in_quote 0.969, topic_accuracy 0.641. |
| 10 | S1 | FAIL | `ship-it` | operator: MIT licence, public repo, video link (11-03); builder: README (no claim of anything not live), scrub list, full-history secret scan | MIT or Apache-2.0 LICENSE, a judges' README with a YouTube/Vimeo/Loom link and "built with Muse Code", public repo. |

Not on the scoreboard because no public URL shows them: the GP5 review
queue (approve → GP2 updates live) and the A4 trace console. Both are in
the demo video.

## Calendar

| Day (Pacific) | Deploy | Notes |
|---|---|---|
| Sun 11-01 | **spent.** `69ca7fa` went out Sat 23:33 PDT on the operator's order; the operator counts it as Sunday's deploy. The day gate would allow one today: do not use it unless the operator orders a deploy in the session. | Staging proof, lane merges, demo dry run on staging. |
| Mon 11-02 | **the last code deploy**: the newest stamped, staging-green SHA, after the 15:00 PST merge cut | Lane work not merged by 15:00 PST waits until after judging. The operator re-runs A2 (if F4 shipped) and A5. |
| Tue 11-03 | freeze: no deploy | Recording, README video link. Rollback only on the operator's word or a failing `/healthz` (AGENTS.md §7). |
| Wed 11-04 | freeze until the operator lifts it after judging | The operator re-runs A5 in the morning and submits by 12:00 PST (close 18:00 PST). Same rollback rule. |

## Builder priorities, top first

1. Production rows. Keep `docs/ops/operator-today.md` exact and current
   (approvals, logos, A2/A5 re-runs, the terms decision, S1). S1 prep:
   the README claims nothing that is not live (today it calls the order
   timeline live), the scrub list, the full-history secret scan
   (`ship-it`).
2. Merge the `meta` lane under the protocol in `.agents/roles/builder.md`.
   Then the follow-ups outside its paths: `meta.base_url` honoured in
   `crates/web/src/draft.rs` and the env-key path in `xtask/src/main.rs`;
   `cached_tokens` through `contracts::Usage`, the runner and the trace
   console. No shared client crate before judging.
3. Release path: `ship-release.sh` gains `--allow-today-overlap` (passed
   only on an operator order) and checks the plant stamp and the full
   root-process probe itself; `README.md` points at `xtask/ops/`, not
   `docs/ops/scripts/`.
4. Demo: once approvals land, a production dry run
   (`DEMO_BASE_URL=https://andrii.ai/apostille npm --prefix web run demo`),
   then the `DEMO_CAPTIONS=1` take; `docs/demo-script.md` stays within 175 s.
5. Breadth from the operator's 10-31 list, each staging-proven before
   Monday's cut, none needing a migration, top first:
   - live updates where they are missing: a staff topic for new proposals
     so `/review` updates live; `call:` accepting the 32-hex call id and
     carrying status and transcript events; `label.*` and shipment events
     carrying their `topic` field;
   - a readable step-by-step "how to get your apostille" guide on the
     dossier, built from accepted facts, with a friendly empty state;
   - the Washington SOS apostille request form template, seeded from the
     official form (cite its URL), prefilled from accepted facts;
   - `/order` saving a submission, with a live order timeline;
   - Meta transcription of call recordings with the `meta` lane's fixed
     client, once merged (Telnyx WAV recording in `crates/voice`).
6. `/taste` findings you listed but did not fix.

Not before judging unless the operator orders it in the session: live
carrier postage (FedEx/USPS/DHL), payments, self-hosted inference
(Glimmer, `docs/ops/plant-inference.md`), new auth providers, schema
migrations.

## Operator-only steps

- Approve proposals in `/review`: at least 8 per slug, keeping 2 per slug
  proposed for the video's approve scene.
- Add and commit the official `google.svg` and `apple.svg` in
  `web/src/lib/assets/brand/`.
- Decide the Meta data-handling terms opt-in for atlas transcription and
  uploads (the `meta` lane left them fail-closed); tell the builder.
- Re-run A2 after F4 ships; re-run A5 Monday evening and Wednesday
  morning.
- S1: switch LICENSE to MIT, make the repo public after the builder's
  scrub list and secret scan, add the video link on 11-03.
- Vogun on production only if its licence covers web embedding: then set
  `BRAND_FONT_DIR` for the builder's release builds (README "Brand
  font"); otherwise builds keep the OFL fallback face.
- Record the production `/review` segment; upload the video; copy the
  official Cerebral Valley submission rules into row S1; submit by
  11-04 12:00 PST.
- Any root step, any production config or secret.
