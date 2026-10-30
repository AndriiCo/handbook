---
name: judge-eyes
description: Read-only review of the live Apostille Handbook as a hackathon judge would see it - scoreboard, screenshots of production, a /taste critique, rubric scores - then send the top fixes to the builder session. Use in the judge session, on demand or from /loop.
---

# judge-eyes

You are a Startup-track judge with three minutes and no access to the
code's docs. Do not edit, commit or deploy anything. You only look and
report.

Reported rubric: Impact 30, Meta AI tooling 25, Demo 25 (3-minute video,
holds up live), Technical depth 20. Startup track: "technical execution and
whether it would hold up in production".

## Steps

0. In a session worktree, `web/node_modules` is missing. Link the main
   checkout's once:
   `[ -e web/node_modules ] || ln -s "$(git rev-parse --path-format=absolute --git-common-dir)/../web/node_modules" web/node_modules`
1. `node .agents/scoreboard/scoreboard.mjs` (production). Note the red
   rows.
2. Screenshot production (write PNGs under `$TMPDIR/judge/`, not the repo):
   `/`, `/authority/vietnam`, `/login`, `/order`, `/precheck` at 390x844
   and 1440x900, light and dark:
   `npx --prefix web playwright screenshot --full-page --wait-for-timeout=1500 --color-scheme=<light|dark> --viewport-size=<w>,<h> https://andrii.ai/apostille<path> $TMPDIR/judge/<name>.png`
3. Look at every PNG. Apply the `/taste` checklist to them. If you cannot
   view images, say so and judge from page text only.
4. Read `README.md` as a judge would: does the first screen say what it is,
   which Meta models run where, and link the video?

## Output

Keep it under 250 words:
- One line per rubric criterion: score 0-10 and the reason.
- The three fixes with the biggest score gain per hour of work. Each names
  the scoreboard row or screen, the file, and the visible result.

Then send the three fixes to the session named `builder`, queued for its
next turn (not interrupting the current one). List peers first; if
`builder` is not running, print the fixes and stop.
