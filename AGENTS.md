# AGENTS.md: Apostille Handbook, hackathon speed mode (until 2026-11-04 18:00 PT)

> PROPOSAL, review line by line before adopting. It grants the agent
> standing approvals and suspends ask-first rules, which is the point,
> so you, not the agent, decide which lines to keep. Sections 1-6 replace
> the current §1-§6. Section 7 keeps every rule that protects secrets and
> production. Suspended rules return on 2026-11-05.

Speed over caution. The product is the judged demo: a stranger opens
https://andrii.ai/apostille, a Meta model does visible work on real
data, and the page updates live. Every turn moves one golden-path (GP)
screen or one AI-lane item visibly forward, or removes a blocker for one.

## 1. Target

Startup track: "technical execution and whether it would hold up in
production". Reported rubric: Impact 30, Meta AI tooling 25, Demo 25
(3-min video, holds up live), Technical depth 20. Judges see a 3-minute
video and the live URL. They never read docs/.

Golden path (GP). Real slugs: `vietnam`, `usa-wa-sos`, `hcch`.
- GP1 `/apostille`: home, ⌘K search. `web/src/routes/(public)/+page.svelte`
- GP2 `/apostille/authority/[slug]`: dossier with live "verified" ledger
- GP3 `/apostille/login`: passkey autofill + branded Apple/Google/Meta
- GP4 `/apostille/order` → `/apostille/orders/[id]`: stepper, photo
  precheck (SAM 3.1 + Muse Spark), form PDF, live timeline
- GP5 admin `/review` + trace console: approve a Spark-drafted,
  quote-verified fact → GP2 updates over WS `authority:{slug}`
  (`crates/web/src/approval.rs:499`)

AI lane (AL): A1 quote-verified citations, A2 SAM 3.1 via Responses,
A3 frozen authority world, A4 trace console, A5 eval harness.

Frozen until Nov 5: ops, drills, backups, retention, timers, certs, ACME,
QUIC, mTLS, OTel, RISC/device caps, sealed-config/wizard changes, mail,
voice, billing/Stripe, Egypt chain, shipping spec, new auth methods, new
security or CI gates, new specs, new D-/OPEN-/SHP- numbers.

## 2. Decide, don't ask

- Standing approval: any change under `web/`; any Rust change a GP screen
  or AL item needs (an endpoint, a WS topic, a migration). Do not request
  confirmation for these.
- Ambiguity: choose what looks better in a screenshot or scores better on
  the eval, write `Assumed: <x>` in the final message, keep going.
- Ask ONLY for a secret value, a root/prod action, money, or an
  irreversible data change. Ask once, state your default, keep working on
  everything unblocked. Never end a turn waiting for an answer.
- Visual choices are never questions: build 2-3 variants (§6).

## 3. Design law (overrides docs/site-theme.md and docs/ui-framework-proposal.md)

Brief: a 2026 trust product. Evidence is the hero: every fact shows its
quote, source and date one hover away, and the screen changes while you
watch. Clarity of Stripe Checkout, density of Linear, Apple-grade sign-in.
Pasted reference images `[Image #N]` win over this section.

- Type: self-hosted variable fonts via `@fontsource-variable` (one UI
  sans, one mono for IDs, fees and dates). A serif is allowed only for
  quoted source text, so quotes read as the authority's own words. No
  Georgia, no system-ui stand-ins.
- Colour: cool neutrals, brand blue scale from `#0b4a75`, one "verified"
  green, one "changed" amber. Shadows, soft gradients and glass are
  allowed. Tokens live in `app.css` only, light + dark.
- Shape: radius 8/12/16, shadow tokens sm/md/lg, 4 px spacing scale, a
  real type scale. No arbitrary `text-[…]`/`py-[…]` in new code.
- Components: shadcn-svelte (pin 1.7.1) on bits-ui in
  `web/src/lib/components/ui/`, `@lucide/svelte` icons, `svelte-sonner`
  toasts used directly (not the shadcn wrapper, which pulls mode-watcher).
  Pin `tailwind-variants@3.3.1` (4.0.0 shipped 2026-10-29 with breaking
  changes). Composing primitives is required, not "speculative
  abstraction".
- Motion: 150-300 ms, transform + opacity, `svelte/transition`,
  `animate:flip`, View Transitions on navigation, all behind
  `prefers-reduced-motion`.
- States: skeletons (never "Loading…"), optimistic updates with
  rollback, empty states with one next action. The public build never
  shows "placeholder", "not wired", "skeleton", "lorem" or "TODO".
- Brand marks: official Apple / Google / Meta / FIDO passkey assets from
  `web/src/lib/assets/brand/`, used per each owner's guidelines. Render a
  provider button only when that provider is configured in prod.

## 4. Done = screenshots + measured AI

A `web/` change is done only when all hold:
1. `npm --prefix web run shots -- <routes>` passes: each touched route at
   390×844 and 1440×900, light and dark, against `web/e2e/fixtures/`,
   writing `web/shots/*.png` and `web/shots/index.html`. It asserts no
   horizontal scroll at 390, zero console errors, zero axe
   serious/critical, no banned strings (§3).
2. `npm --prefix web run check` and both web builds are green.
3. `/taste` ran on the touched screens; top findings fixed or listed.
4. The final message lists PNG paths and 3 bullets of visible change.
   Inspect every PNG before reporting; if you cannot, write "not viewed".

An AI-lane change is done only when it reports a live run (model,
p50/p95 latency, tokens), saves the recorded responses as replay
fixtures, and shows the eval delta (before → after on the gold set).

Demo gate: `npm --prefix web run demo` walks GP1→GP5, records video to
`web/shots/demo/`, fails on console errors or banned strings. A red demo
gate means no deploy that day.

## 5. Run shape

- One slice per turn, 45-minute time-box. At 40 minutes commit what is
  green and report what is left. Never start a second slice in a turn.
- Gates by diff: web-only → §4 only (no cargo). A touched crate → that
  crate's tests + `cargo clippy -p <crate>`. Full `cargo xtask ci` once a
  day, before the deploy.
- Fast loop: `npm --prefix web run dev` proxies `/apostille/api` and
  `/apostille/ws` to production's public read endpoints, so HMR runs on
  real data. Web-only releases go through `ops-web-ship.sh` (copy `spa/`,
  no zigbuild, no restart).
- Commits small, on main, prefixed with the prompt number (`P096: …`).
  Push after each green slice.
- Log: append ONE line to `docs/prompts/LOG.md` in the same commit:
  `| 096 | 2026-10-30 16:05Z | <first 80 chars of prompt> | <outcome, ≤15 words> |`
  No per-prompt files, no log-only or hash-fill commits. Files 001-095
  stay as they are.
- The full prompt-and-response record is `docs/prompts/sessions/`, which
  a Stop hook regenerates from Muse's session log for every session (the
  builder and the judge). Never edit it by hand; commit it with your next
  real change.
- Goal, oracle and recipes: `.agents/goal.md`, `.agents/scoreboard/`,
  `.agents/skills/`. Run them, never edit them (the operator does).
- Never edit this file mid-run; propose changes in the final message.

## 6. Scope: mock-first, then live; parallel only in worktrees

- Build each GP screen against fixture JSON shaped exactly like the real
  API (`web/e2e/fixtures/`, served with Playwright `page.route`). Get the
  screenshots green first, then wire it to the live endpoint in a short
  follow-up turn.
- Promote, don't duplicate: lift the designed parts of `(mock)/mock/*`
  (stepper, fee receipt, VerifiedChip) into the real routes, then delete
  that mock page.
- Realtime: existing `ws_bus` topics through one store,
  `web/src/lib/live.svelte.ts`. The bus takes one `?topic=` per
  connection and anonymous callers may subscribe to `authority:{slug}`
  only; any new topic (`run:{id}`) needs a `WsTopic` variant plus authz in
  `crates/auth/src/ws.rs`. No polling on GP screens.
- Parallel writers: write-capable children, each in an isolated worktree
  on its own branch, owning disjoint paths. Children never touch
  `app.css`, any `+layout.svelte`, `package*.json`, `vite.config.ts`,
  `AGENTS.md` or `docs/prompts/LOG.md`. At most 3 writing children.

## 7. Safety rails

Kept, never relaxed:
- No secret values in the repo, fixtures, screenshots, videos, logs or
  prompts. `secrets-scan` and the pre-commit hook stay on.
- Fixtures, screenshots and demo documents are synthetic: no real
  customer names, emails, addresses or document images.
- Admin code never ships in the public build (`check:imports` stays). Do
  not weaken auth gates on admin.andrii.ai. A public AI surface is
  rate-limited and cost-capped.
- Production: the agent runs no root actions and never hand-edits prod
  config, secrets or DB. At most one binary deploy per day via the
  existing path, after the demo gate and `cargo xtask ci` are green. All
  root steps for a day go into ONE operator script (zero placeholders,
  `ABORT: <reason>`, prints evidence, `bash -n` clean).
- Never rewrite history; roll back with a revert commit.

Suspended until 2026-11-05: old §1 "if uncertain, ask… stop"; old §2
"no abstractions for single-use code" and old §3 "match existing style"
for `web/`; old §5 per-prompt log files; old §6 single-writer checkout,
child output caps, decision/OPEN ledger and spec-before-surface; the bans
and JS budgets in `docs/site-theme.md`; the DO-NOTs in
`docs/ui-framework-proposal.md`.
