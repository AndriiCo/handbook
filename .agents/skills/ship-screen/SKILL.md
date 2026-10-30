---
name: ship-screen
description: Build or redesign one Apostille Handbook web screen (scoreboard rows GP1-GP4, or the admin review/trace pages) to 2026 product quality, proven by screenshots and the scoreboard row. Use for any change under web/src that a user sees.
---

# ship-screen

One screen per slice, 45 minutes. The slice is done only when the
screenshots and the scoreboard row prove it. AGENTS.md §3 is the design law;
pasted reference images (`[Image #N]`) win over it.

## 0. First time only: the design system (row GP1 does this)

If `web/src/lib/components/ui/` does not exist yet, build it before any
screen:
- `@fontsource-variable` UI sans + mono (serif only for quoted source text);
  Georgia and system stand-ins gone; Tailwind preflight on.
- Tokens in `web/src/app.css` only, light + dark: cool neutrals, brand blue
  from `#0b4a75`, one verified green, one changed amber, shadow sm/md/lg,
  radius 8/12/16, a type scale.
- `npx shadcn-svelte@1.7.1 init`, pin `tailwind-variants@3.3.1`; add
  button, input, card, badge, tabs, dialog, command, popover, hover-card,
  tooltip, skeleton, separator. `@lucide/svelte` icons. `svelte-sonner`
  directly (no mode-watcher; keep the repo's data-theme boot script).
- Shell: sticky translucent top bar, ⌘K command dialog over authorities,
  one footer, real favicon/manifest/OG image.
- Fast loop: `npm run dev` with fixtures. Add a `server.proxy` in
  `web/vite.config.ts` that sends `/apostille/api` and `/apostille/ws`
  (`ws: true`) to `APOSTILLE_API` (the staging tunnel URL, AGENTS.md §5,
  §8), so HMR shows staging data and the scoreboard can run against it.

## 1. Set up (in a subagent worktree)

A fresh worktree has no `web/node_modules`. Link the main checkout's
instead of installing:
`ln -s "$(git rev-parse --path-format=absolute --git-common-dir)/../web/node_modules" web/node_modules`

## 2. Build against fixtures first

Fixtures in `web/e2e/fixtures/` shaped exactly like the API contracts in
`.agents/goal.md` (synthetic data only). Get the look right on fixtures,
then wire the live endpoint in the same slice.

Every screen gets: skeletons (never "Loading…"), empty states with one next
action, visible focus rings, hover/press states, 44 px tap targets at 390
px, motion 150-300 ms on transform/opacity behind `prefers-reduced-motion`.
Realtime goes through one store, `web/src/lib/live.svelte.ts`; the bus takes
one `?topic=` per connection, anonymous callers get `authority:{slug}` only.

## 3. Prove it

1. Run `/taste` on the screen and fix its top findings.
2. Screenshots at 390x844 and 1440x900, light and dark:
   `npx --prefix web playwright screenshot --full-page --wait-for-timeout=1500 --color-scheme=dark --viewport-size=390,844 <url> web/shots/<row>-390-dark.png`
   (or `npm --prefix web run shots -- <route>` once that script exists).
   Look at every PNG. If you cannot view images, write "not viewed".
3. `npm --prefix web run check`, `npm --prefix web run build`,
   `npm --prefix web run build:admin`, `npm --prefix web run check:imports`.
4. `node .agents/scoreboard/scoreboard.mjs --base http://localhost:5173/apostille --only <row>`
   (start `npm --prefix web run dev` in the background first).

## 4. Finish

Commit `P0NN: <row>: <what changed>`. Final message: the PNG paths, three
bullets of visible change, the scoreboard line for the row, anything
assumed (`Assumed: …`).

Never: arbitrary `text-[…]`/`py-[…]` in new code, tokens outside
`app.css`, real customer data in fixtures or screenshots, admin imports in
the public build, a provider button for a provider that is not configured.
