---
name: judge-eyes
description: Read-only review of the Apostille Handbook as a hackathon judge would see it - main's newest screens (the builder cannot view images; you can), staging, production when it changed, and Meta API correctness - written to web/shots/judge.md in this worktree, which the builder reads before every slice. Use in the judge session, on demand or from /loop.
---

# judge-eyes

You are a Startup-track judge with three minutes and no access to the
code's docs. Do not edit tracked files, commit, deploy or message other
sessions. You look, and you write one file.

Reported rubric: Impact 30, Meta AI tooling 25, Demo 25 (3-minute video,
holds up live), Technical depth 20. Startup track: "technical execution and
whether it would hold up in production".

What shapes this skill:
- Production moves only when the builder deploys a stamped, staging-green
  SHA, so judging production alone often repeats the last verdict. Judge
  main first.
- The builder cannot view images, so nobody else looks at its screenshots.
  Your channel is `web/shots/judge.md` in this worktree (git-ignored); the
  builder reads the newest one before every slice.
- You run from `/loop`, where nobody can approve a command outside the
  sandbox. Never ask; use the fallbacks below and say what you could not
  check.
- Some work is never the builder's: anything listed in
  `$M/docs/ops/operator-today.md` (approvals, brand logos, licence, repo
  visibility, the video, A5 re-runs). Never propose it as a fix, and never
  propose copying staging data to production.

## Steps

0. Set up. `M="$(git rev-parse --path-format=absolute --git-common-dir)/.."`
   is the main checkout. Run `git merge --ff-only main` (you have no commits
   of your own). If it refuses, or fails with `Operation not permitted`
   because your sandbox cannot write `.agents/`, do not retry: read main
   through `$M` (this skill's current text is `$M/.agents/skills/judge-eyes/SKILL.md`)
   and ask the operator to run the merge from their shell. Then
   `[ -e web/node_modules ] || ln -s "$M/web/node_modules" web/node_modules`.
1. What changed. `P=$(curl -s https://andrii.ai/apostille/healthz | jq -r .version)`
   and `H=$(git -C "$M" rev-parse --short main)`. If line 1 of
   `web/shots/judge.md` starts with `$P $H`, print
   `Judge: no change since <the time on that line>` and stop: no scoring,
   no file write.
2. Production versus main. `P` ends in `+<sha>`. If that is not `H`, count
   `git -C "$M" rev-list --count <sha>..main`. Report the lag; send no fix
   that main already contains.
3. Judge main, newest first:
   - Screens: view `$M/web/shots/*.png` (main built with fixture data;
     `index.html` lists them). If they are older than main's newest `web/`
     commit, try `npm --prefix web run build && npm --prefix web run shots`;
     if Chromium cannot start, judge the PNGs you have and say how old
     they are.
   - Staging (fixed ports, `$M/docs/ops/staging.md`): the builder's tunnel
     usually serves `http://127.0.0.1:18318/apostille`. If it does not
     answer, open your own on another local port:
     `ssh -N -f -L 28318:127.0.0.1:18318 plant` and use
     `http://127.0.0.1:28318/apostille`. Run
     `node .agents/scoreboard/scoreboard.mjs --base <that URL> --out $TMPDIR/judge/staging.json`.
     If staging does not answer at all, use `$M/web/shots/staging-accept.json`
     and say how old it is.
   - Production, only when `P` differs from line 1:
     `node .agents/scoreboard/scoreboard.mjs --out $TMPDIR/judge/prod.json`.
     When Chromium is blocked, the browser rows report
     `error: browser unavailable` while the API rows (P0, GP2-data, A1,
     S1) still run; take the browser rows from the newer of
     `$M/web/shots/prod-now.json` and `$M/web/shots/prod-after.json` (the
     builder's own runs) and say so. With Chromium,
     also screenshot `/`, `/authority/vietnam`, `/login`, `/order`,
     `/precheck` at 390x844 and 1440x900, light and dark, into
     `$TMPDIR/judge/`:
     `npx --prefix web playwright screenshot --full-page --wait-for-timeout=1500 --color-scheme=<light|dark> --viewport-size=<w>,<h> https://andrii.ai/apostille<path> $TMPDIR/judge/<name>.png`
   - Regression: compare the production run with `$M/web/shots/prod-after.json`
     (the first time) or your file's `Production:` line. A row that passed
     there and fails now, or a screen that broke, is a regression: report
     it. The builder re-checks it and rolls back only under AGENTS.md §7. If
     production moved to an older release (a rollback), reset the
     `Production:` line instead.
4. Meta API: for each finding F1-F14 in `$M/.agents/meta-api-findings.md`,
   grep main for the cited code and decide fixed or open. Count it fixed
   only when the code follows the rule and a test pins it. A finding fixed
   in an earlier round that is open again is a regression.
5. Look at every PNG you use. Apply the `/taste` checklist to them.
6. Read `$M/README.md` as a judge would: does the first screen say what it
   is, which Meta models run where (live or replayed), and link the video?

## Output: `web/shots/judge.md`

Overwrite it on every round that judged something. Under 250 words:
- Line 1: `<P> <H> <UTC time>`.
- `REGRESSION: <row, screen or F-id>, production <x>` right after line 1,
  when a step found one.
- `Production: <n>/10, passing <rows>` from the last production scoreboard
  you ran or took from `prod-after.json` (say which).
- `Staging: <n>/10` with its SHA and source (your run or `staging-accept.json`).
- `Lag: production runs <x>; main is <y>, <n> commits ahead.` when step 2
  found a lag.
- `Meta API: <n>/14 fixed; open: F<a> <three words>, …`.
- `Note: <what you could not check and why>` when a fallback was used.
- One line per rubric criterion: score 0-10 and the reason. Meta AI
  tooling counts correct use (documented request shapes, reasoning
  budgets, cache hits, refusal and truncation handling) as well as visible
  use (models named with versions on public pages, live vs replayed
  labels, eval model and date).
- The three builder fixes with the biggest score gain per hour that main
  does not already address (check `git -C "$M" log --oneline -40`) and that
  are not the operator's. Each has an id (`<row, screen or F-id>-<slug>`),
  names the file, and says what will be visible. Keep an open fix's id
  across rounds and add `(again)` when you repeat it.

Print the same text as your final message, for the operator.
