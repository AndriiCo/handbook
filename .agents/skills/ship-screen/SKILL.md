---
name: ship-screen
description: Build or redesign one Apostille Handbook web screen (scoreboard rows GP1-GP4, the dossier guide, or the admin review/trace pages) to 2026 product quality, proven by screenshots, the demo gate and the scoreboard row. Use for any change under web/src that a user sees.
---

# ship-screen

One screen per slice, 45 minutes. The slice is done only when the
screenshots, the demo gate and the scoreboard row prove it. AGENTS.md §3 is
the design law; pasted reference images (`[Image #N]`) win over it.

## 0. The design system exists: extend it, don't fork it

`web/src/lib/components/ui/` (shadcn-svelte on bits-ui), tokens in
`web/src/app.css` only (light + dark), `@fontsource-variable` sans + mono,
the brand face through `--font-brand` (loaded with FontFace in the root
layout, falling back when the operator's font files are absent),
`@lucide/svelte` icons, `svelte-sonner` directly. Realtime goes through one
store, `web/src/lib/live.svelte.ts`. A new token, component or variant goes
into the system, never inline in a route.

## 1. Set up (in a subagent worktree)

A fresh worktree has no `web/node_modules`. Link the main checkout's
instead of installing:
`ln -s "$(git rev-parse --path-format=absolute --git-common-dir)/../web/node_modules" web/node_modules`

## 2. Build against fixtures first

Fixtures in `web/e2e/fixtures/` shaped exactly like the API contracts in
`.agents/goal.md` (synthetic data only), served by Playwright `page.route`
inside `shots` and `demo`. `npm --prefix web run dev` has no API proxy and
serves no fixtures, so it shows empty states; a `server.proxy` in
`web/vite.config.ts` that sends `/apostille/api` and `/apostille/ws`
(`ws: true`) to `APOSTILLE_API` (the staging tunnel,
`http://127.0.0.1:18318`) is a short slice worth doing. Get the look right
on fixtures, then wire the live endpoint in the same slice.

Every screen gets: skeletons (never "Loading…"), empty states with one next
action, visible focus rings, hover/press states, 44 px tap targets at 390
px, motion 150-300 ms on transform/opacity behind `prefers-reduced-motion`.
The bus takes one `?topic=` per connection; anonymous callers get
`authority:{slug}` only.

## 3. Prove it

1. Run `/taste` on the screen and fix its top findings.
2. `npm --prefix web run check`, `check:imports`, `check:tailwind`, then
   `build` and `build:admin` (`shots` and `demo` serve the built `build/`
   and `build-admin/`, so build first or the screenshots are stale).
3. `npm --prefix web run shots -- <route>`: 390x844 and 1440x900, light and
   dark, files named like `gp1-home-390x844-dark.png`. A new route goes
   into ROUTES in `web/scripts/shots.mjs` first. Look at every PNG; if you
   cannot view images, write "not viewed".
4. `npm --prefix web run demo` (the plant gate also runs `smoke` and
   `test:e2e`).
5. The scoreboard row runs against a server with an API: the staging
   tunnel once the slice is on staging (AGENTS.md §8,
   `--base http://127.0.0.1:18318/apostille --only <row>`), not the bare
   dev server. With `--only` and no `--out` it writes
   `web/shots/scoreboard-partial.json`.

## 4. Finish

Commit `P<NNN>: <row>: <what changed>` with its LOG.md line (a lane: no
LOG line, its own branch). Final message: the PNG paths, three bullets of
visible change, the scoreboard line for the row, anything assumed
(`Assumed: …`).

Never: arbitrary `text-[…]`/`py-[…]` in new code, tokens outside
`app.css`, real customer data in fixtures or screenshots, admin imports in
the public build, a provider button for a provider that is not configured.
