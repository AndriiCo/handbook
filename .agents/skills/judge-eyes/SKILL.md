---
name: judge-eyes
description: Read-only review of the Apostille Handbook as a hackathon judge would see it - main's newest screens (the builder cannot view images; you can), staging, and production when it changed - written to web/shots/judge.md in this worktree, which the builder reads before every slice. Use in the judge session, on demand or from /loop.
---

# judge-eyes

You are a Startup-track judge with three minutes and no access to the
code's docs. Do not edit tracked files, commit or deploy. You look, and you
write one file.

Reported rubric: Impact 30, Meta AI tooling 25, Demo 25 (3-minute video,
holds up live), Technical depth 20. Startup track: "technical execution and
whether it would hold up in production".

Two facts shape this skill:
- Production moves only when the operator deploys (once a day), so judging
  production alone repeats yesterday's verdict. Judge main first.
- The builder session has no name and cannot view images, so messages do
  not reach it and nobody else looks at its screenshots. Your channel is
  `web/shots/judge.md` in this worktree (git-ignored); the builder reads it
  before every slice. Never message other sessions.

## Steps

0. Set up. `M="$(git rev-parse --path-format=absolute --git-common-dir)/.."`
   is the main checkout. Run `git merge --ff-only main` (you have no commits
   of your own; if it refuses, say so and read main through `$M`). Then
   `[ -e web/node_modules ] || ln -s "$M/web/node_modules" web/node_modules`.
1. What changed. `P=$(curl -s https://andrii.ai/apostille/healthz | jq -r .version)`
   and `H=$(git -C "$M" rev-parse --short main)`. If line 1 of
   `web/shots/judge.md` starts with `$P $H`, print
   `Judge: no change since <the time on that line>` and stop: no scoring,
   no file write.
2. Production versus main. `P` ends in `+<sha>`. If that is not `H`, count
   `git -C "$M" rev-list --count <sha>..main`. The lag is the operator's
   item, not the builder's: it becomes the operator line of your output,
   and you send no fix that main already contains.
3. Judge main, newest first:
   - Screens: view `$M/web/shots/*.png` (main built with fixture data;
     `index.html` lists them). If they are older than main's newest
     `web/` commit, capture your own:
     `npm --prefix web run build && npm --prefix web run shots`.
   - Staging: when `$M/docs/ops/staging.md` says how to reach a running
     staging stack, run `node .agents/scoreboard/scoreboard.mjs --base <that URL>`.
   - Production, only when `P` differs from line 1:
     `node .agents/scoreboard/scoreboard.mjs`, and screenshots of `/`,
     `/authority/vietnam`, `/login`, `/order`, `/precheck` at 390x844 and
     1440x900, light and dark, into `$TMPDIR/judge/`:
     `npx --prefix web playwright screenshot --full-page --wait-for-timeout=1500 --color-scheme=<light|dark> --viewport-size=<w>,<h> https://andrii.ai/apostille<path> $TMPDIR/judge/<name>.png`
   If Chromium fails inside the sandbox (a MachPort error), ask for
   approval to run that one command outside the sandbox. If that is
   refused, judge from page text and the builder's PNGs, and say so.
4. Look at every PNG you use. Apply the `/taste` checklist to them.
5. Read `$M/README.md` as a judge would: does the first screen say what it
   is, which Meta models run where (live or replayed), and link the video?

## Output: `web/shots/judge.md`

Overwrite it on every round that judged something. Under 250 words:
- Line 1: `<P> <H> <UTC time>`.
- `Operator: production runs <x>; main is <y>, <n> commits ahead.` when
  step 2 found a lag.
- One line per rubric criterion: score 0-10 and the reason.
- The three fixes with the biggest score gain per hour of work that main
  does not already address (check `git -C "$M" log --oneline -40`). Each
  has an id (`<row or screen>-<slug>`, e.g. `GP1-hero-density`), names the
  file, and says what will be visible. Keep an open fix's id across rounds
  and add `(again)` when you repeat it.

Print the same text as your final message, for the operator.
