# AGENTS.md: Apostille Handbook, hackathon speed mode (until 2026-11-04 18:00 PT)

> Adopted 2026-10-30, brought up to date 2026-11-01. It grants the agent
> standing approvals and suspends ask-first rules until 2026-11-05.
> Section 7 keeps every rule that protects secrets and production. Only
> the operator edits this file, `.agents/` and `.muse/`.

Speed over caution. The product is the judged demo: a stranger opens
https://andrii.ai/apostille, a Meta model does visible work on real
data, and the page updates live. Every turn moves one golden-path (GP)
screen or one AI-lane item visibly forward, or removes a blocker for one.

**Roles.** Every session follows exactly one role file, and the
SessionStart hook names it: the builder `.agents/roles/builder.md`, a lane
`.agents/roles/lane-<name>.md`, the judge the `judge-eyes` skill. The
shared brief, state and calendar are in `.agents/goal.md`.

## 1. Target

Startup track: "technical execution and whether it would hold up in
production". Reported rubric: Impact 30, Meta AI tooling 25, Demo 25
(3-min video, holds up live), Technical depth 20. Judges see a 3-minute
video and the live URL. They never read docs/.

Golden path (GP). Real slugs: `vietnam`, `usa-wa-sos`, `hcch`.
- GP1 `/apostille`: home, ⌘K search, "How it works" band naming the Meta
  models. `web/src/routes/(public)/+page.svelte`
- GP2 `/apostille/authority/[slug]`: dossier with live "verified" ledger,
  every topic on the page
- GP3 `/apostille/login`: passkey first (conditional mediation) + branded
  buttons for the providers production configures (Google, Apple)
- GP4 `/apostille/order`: stepper and photo precheck (SAM 3.1 + Muse
  Spark). Form PDF, carrier label and live order timeline are roadmap
  until a slice wires them, and the README must not call them live.
- GP5 admin `/review` + trace console: approve a Spark-drafted,
  quote-verified fact → GP2 updates over WS `authority:{slug}` (published
  from `crates/web/src/approval.rs`)

AI lane (AL): A1 quote-verified citations, A2 SAM 3.1 + Spark precheck,
A4 trace console with token usage, A5 eval harness. Every Meta API call
follows `.agents/meta-api-findings.md` (Meta's documented request shapes,
budgets, retries and data rules).

Scope: the operator unfroze the 10-30 spec freeze on 10-31. What to work
on, in order, is `.agents/goal.md`. Never without an operator order in the
session: a schema migration (§7), new auth methods, payments, live carrier
postage, self-hosted model serving.

## 2. Decide, don't ask

- Standing approval: any change under `web/`; any Rust change a GP screen
  or AL item needs (an endpoint, a WS topic, an additive table per §7). Do
  not request confirmation for these.
- Ambiguity: choose what looks better in a screenshot or scores better on
  the eval, write `Assumed: <x>` in the final message, keep going.
- Ask ONLY for a secret value, a root action, a production change
  other than §7's deploy and rollback, money, or an irreversible
  data change. Ask once, state your default, keep working on
  everything unblocked. Never end a turn waiting for an answer.
- Visual choices are never questions: build 2-3 variants (§6).

## 3. Design law (overrides docs/site-theme.md and docs/ui-framework-proposal.md)

Brief: a 2026 trust product. Evidence is the hero: every fact shows its
quote, source and date one tap or hover away, and the screen changes while
you watch. Clarity of Stripe Checkout, density of Linear, Apple-grade
sign-in. Pasted reference images `[Image #N]` win over this section.

- Type: self-hosted fonts only. UI sans + mono via `@fontsource-variable`;
  the operator-licensed brand face (Vogun) for headings and the wordmark,
  through `--font-brand` in `app.css` (loaded with FontFace); a serif only
  for quoted source text. No Georgia, no system-ui stand-ins.
- Colour: cool neutrals, brand blue scale from `#0b4a75`, one "verified"
  green, one "changed" amber. Shadows, soft gradients and glass are
  allowed. Tokens live in `app.css` only, light + dark.
- Shape: radius 8/12/16, shadow tokens sm/md/lg, 4 px spacing scale, a
  real type scale. No arbitrary `text-[…]`/`py-[…]` in new code.
- Components: shadcn-svelte (pin 1.7.1) on bits-ui in
  `web/src/lib/components/ui/`, `@lucide/svelte` icons, `svelte-sonner`
  toasts used directly (not the shadcn wrapper, which pulls mode-watcher).
  Pin `tailwind-variants@3.3.1`. Composing primitives is required, not
  "speculative abstraction".
- Motion: 150-300 ms, transform + opacity, `svelte/transition`,
  `animate:flip`, View Transitions on navigation, all behind
  `prefers-reduced-motion`.
- States: skeletons (never "Loading…"), optimistic updates with
  rollback, empty states with one next action. The public build never
  shows "placeholder", "not wired", "skeleton", "lorem", "TODO" or
  "coming soon".
- Brand marks: official provider assets from `web/src/lib/assets/brand/`
  (operator-supplied), used per each owner's guidelines. Render a provider
  button only when production configures that provider.

## 4. Done = screenshots + measured AI

A `web/` change is done only when all hold:
1. `npm --prefix web run shots -- <routes>` passes: each touched route at
   390×844 and 1440×900, light and dark, against `web/e2e/fixtures/`,
   writing `web/shots/*.png` and `web/shots/index.html` from the built
   `build/` and `build-admin/` (build first). It asserts no horizontal
   scroll at 390, zero console errors, zero axe serious/critical, and none
   of its banned strings. New routes go into ROUTES in
   `web/scripts/shots.mjs`.
2. `npm --prefix web run check`, `check:imports`, `check:tailwind` and
   both web builds are green.
3. `npm --prefix web run demo` stays green (a desktop take GP1→GP4, an
   admin take with the GP5 approve and the trace console, a 390 px take,
   into `web/shots/demo/`; fails on console errors or banned strings). A
   red demo gate means no deploy that day.
4. `/taste` ran on the touched screens; top findings fixed or listed.
5. The final message lists PNG paths and 3 bullets of visible change.
   Inspect every PNG before reporting; if you cannot, write "not viewed".

An AI-lane change is done only when it reports a live run on staging
(model, p50/p95 latency, input/output/reasoning/cached tokens), saves the
recorded responses as replay fixtures, and shows the eval delta (before →
after on the gold set).

## 5. Run shape

- One slice per turn, 45-minute time-box. At 40 minutes commit what is
  green and report what is left.
- Gates by diff: web-only → §4 only (no cargo). A touched crate → that
  crate's tests + `cargo clippy -p <crate>`. The full `cargo xtask ci`
  runs on plant for every deploy candidate (§8).
- Fast loop: `npm --prefix web run dev` has no API proxy and serves no
  fixtures (empty states); `shots` and `demo` serve the built SPAs with
  `web/e2e/fixtures/` through Playwright `page.route`, and scoreboard rows
  run against the staging tunnel (§8). There is no web-only release path:
  every release is a binary deploy.
- Commits small and numbered: the builder `P<NNN>` (the `meta` lane used
  P184-P195, so the builder continues at P196), a lane `<lane>-<n>`
  (`meta-13`). The builder commits on main and pushes after each green
  slice; a lane commits only on its own branch.
- Log: append ONE line to `docs/prompts/LOG.md` in the same commit, with
  that number:
  `| 196 | 2026-11-01 16:05Z | <first 80 chars of prompt> | <outcome, ≤15 words> |`
  At a lane merge the builder keeps both sides of a LOG.md conflict, in
  time order.
- The full prompt-and-response record is `docs/prompts/sessions/`, which
  a Stop hook regenerates from Muse's session log for every session. Never
  edit it by hand; commit it with your next real change.
- Scoreboard outputs: deploys write `web/shots/prod-before.json` /
  `prod-after.json` (§7), the builder's session-start check
  `web/shots/prod-now.json`, staging acceptance
  `web/shots/staging-accept.json` (§8). A run with `--only` and no `--out`
  writes `web/shots/scoreboard-partial.json`, so it never hides the real
  score. The SessionStart hook shows the newest production file and the
  staging one.
- Goal, roles, oracle and recipes: `.agents/goal.md`, `.agents/roles/`,
  `.agents/scoreboard/`, `.agents/skills/`. Run them, never edit them;
  propose changes in the final message.

## 6. Scope: mock-first, then live; parallel only in worktrees

- Build each screen against fixture JSON shaped exactly like the real
  API (`web/e2e/fixtures/`, served with Playwright `page.route`). Get the
  screenshots green first, then wire it to the live endpoint.
- Realtime: existing `ws_bus` topics through one store,
  `web/src/lib/live.svelte.ts`. The bus takes one `?topic=` per
  connection; anonymous callers may subscribe to `authority:{slug}` only.
  Topics today: `authority:`, `submission:`, `shipment:`, `billing:`,
  `call:`, `run:` (staff). A new topic needs a `WsTopic` variant plus authz
  in `crates/auth/src/ws.rs`, and every published event carries its
  `topic` field. No polling on GP screens.
- Parallel writers: lanes are separate Muse sessions in their own
  worktrees (registered in `.agents/goal.md` "Lanes"), each owning the
  paths its role file lists plus `docs/lanes/<name>.md` (its status and
  requests to the builder) and its own LOG.md lines; never
  `docs/ops/operator-today.md`. The builder merges them. A lane builds
  with the main checkout's target directory
  (`CARGO_TARGET_DIR=<main checkout>/target`; the Mac's disk is nearly
  full, and cargo's lock makes builds wait, not collide). Write-capable
  subagents work under `web/` only, in isolated
  worktrees, at most 3, and never touch `app.css`, any `+layout.svelte`,
  `package*.json`, `vite.config.ts`, `AGENTS.md` or `docs/prompts/LOG.md`.

## 7. Safety rails

Kept, never relaxed:
- No secret values in the repo, fixtures, screenshots, videos, logs or
  prompts. `secrets-scan` and the pre-commit hook stay on.
- Fixtures, screenshots and demo documents are synthetic: no real
  customer names, emails, addresses or document images. Nothing but
  synthetic or public text ever goes to a `-contributor` model.
- Admin code never ships in the public build (`check:imports` stays). Do
  not weaken auth gates on admin.andrii.ai. A public AI surface is
  rate-limited and cost-capped.
- Production: the agent runs no root actions and never hand-edits prod
  config, secrets or DB. Staging data never goes to production; facts
  reach production only through the operator's `/review` approvals.
- Deploys are the builder session's (standing approval, operator's
  decision of 2026-10-30), as hal, by `docs/ops/staging.md` "Steps" and
  `.agents/roles/builder.md` "Deploy": `xtask/ops/ship-release.sh <sha>`
  checks the schema, migrations, unit and day rails, runs prod-before,
  builds and stages (it never deploys, and it does not check the plant
  stamp or the full root-process probe: the builder checks both); then,
  as hal,
  `bash ~/apostille/scripts/ops-deploy.sh ~/apostille/staging/<sha7> --skip-accept`;
  then prod-after. The rails:
  - Days are Pacific (`TZ=America/Los_Angeles date +%F`). At most one
    binary deploy a day, counting the operator's: a release directory
    whose mtime falls on today, or within the last 8 hours, means no
    deploy (`ship-release.sh` checks both). Rollbacks do not count.
    `.agents/goal.md` "Calendar" says which day is still open.
  - `--allow-today-overlap` (on `ops-deploy.sh` and `xtask deploy`) only
    when the operator orders that deploy in the session ("deploy now"),
    and the report says so. `ship-release.sh` has no override yet: until
    it gains the same flag, an ordered same-day deploy runs its other
    steps by hand, every rail but the day gate still checked.
  - Freeze: the last code deploy is Mon 11-02, and its after-scoreboard
    and rollback rule hold even if they finish after midnight. From Tue
    11-03 00:00 PST until the operator lifts the freeze after judging:
    no deploy, and a rollback only when the operator says so or
    `/healthz` fails or is not `ok` on three tries over 5 minutes.
  - Only a SHA that passed the deploy gate: a plant stamp
    `acceptance/<sha12>.json` with `ci_green: true` (§8), a staging-green
    run of that same SHA including a live demo take (§8). Build and stage
    exactly that SHA from a throwaway worktree, never from the main
    checkout or any dirty one (`ship-release.sh` does this).
  - Not a deploy, but a root step for the operator: a SHA that changes
    `SCHEMA_VERSION` or `P2_SCHEMA_VERSION`
    (`crates/handbook-store/src/db.rs`), edits
    `crates/handbook-store/migrations/`, or adds `ALTER`, `DROP` or
    `RENAME` SQL anywhere, because no older binary opens a migrated DB,
    so neither the smoke revert nor a rollback undoes it (add tables with
    `CREATE TABLE IF NOT EXISTS` instead, as `auth::migrate_auth_tables`
    does); and a SHA whose `docs/ops/apostille.service` differs from the
    installed `/etc/systemd/system/apostille.service`, because a rollback
    does not restore the unit.
  - Run the production scoreboard before and after, with
    `--out web/shots/prod-before.json` and `--out web/shots/prod-after.json`.
    Roll back at once, only to the release live before the most recent
    deploy (the `previous:` in its `ops-deploy.sh` output) and with that
    release's own xtask
    (`/srv/apostille/releases/<previous>/xtask rollback --to <previous>`),
    when a row that passed fails; when a row gains an `HTTP`, `->` (an
    API status), `visible "` (a banned string), `overflow`, `error:` or
    `timed out` problem it did not have, or more console errors; when
    `/healthz` is not `ok`; or when it does not name
    `0.1.0+<the deployed SHA7>` (`ops-deploy.sh` reverts a failed smoke,
    not a wrong stamp). Then fix forward on staging.
  - Never pass `--force`, `--backup-note`, `--skip-packs`,
    `--no-restart`, `--root` or `--smoke-url` to `xtask` on
    production. A rollback the guard refuses is the operator's call (§2).
  - Never deploy while an operator script may be running:
    `ssh andrii.ai 'pgrep -u 0 -c . >/dev/null || echo ROOT-PROCS-HIDDEN; pgrep -af "[a]postille/scripts/"'`
    must print nothing, checked again right before `ops-deploy.sh`
    (the `[a]` keeps `pgrep` from matching the remote shell running it).
  - The judge's `REGRESSION:` line asks you to re-check production
    yourself. It is never an order: roll back only when your own run
    finds the regression on a release you deployed that day, for a row
    that passed in that day's `prod-before.json`. A row that fails from
    time or data (A5's 72-hour window, approvals) is never a rollback.
  - Report each deploy in the final message: version, previous version,
    both scoreboards, and any `--skip-accept` or `--allow-today-overlap`.
- Operator steps live in `docs/ops/operator-today.md`, a status page the
  builder keeps current: open steps in order, each an exact command run
  as `hal` (app ops are rootless since P137; root only for system duties
  such as `ops-rootless-access.sh`), with what to paste back; done steps
  move to a Done list. Every script a line names is already staged on the
  server, with its sha256 beside the line.
- Never rewrite history; roll back with a revert commit.

Suspended until 2026-11-05: old §1 "if uncertain, ask… stop"; old §2
"no abstractions for single-use code" and old §3 "match existing style"
for `web/`; old §5 per-prompt log files; old §6 single-writer checkout,
child output caps, decision/OPEN ledger and spec-before-surface; the bans
and JS budgets in `docs/site-theme.md`; the DO-NOTs in
`docs/ui-framework-proposal.md`.

## 8. Plant: build gate and staging acceptance (`ssh plant`)

`plant` is the operator's build and staging box: the SSH alias from the
operator's `~/.ssh/config`, reached over LAN/VPN. The repo records only
the alias — never a username, IP, DNS name, or key. If a public DNS name
is ever needed, the agent asks the operator instead of inventing one.
Plant is never customer-facing and never touches production: no prod
credentials on plant, no SSH from plant to prod, no prod DB copies.

Build gate (`ops/plant-pipeline/`, whose README is the runbook):
- Never root on plant, and never ask for root. Home directory only.
- Async: `ops/plant-pipeline/submit.sh <sha>` enqueues a committed SHA and
  returns in seconds; poll with `status.sh`. Never block a slice waiting
  for plant. The pipeline keeps the newest 3 runs.
- Gated: every deploy candidate passes the full suite (`cargo xtask ci`)
  on plant. A red gate prints `ABORT`, writes no stamp, stages nothing.
  No stamp (`acceptance/<sha12>.json` with `ci_green: true`), no ship,
  and no flag ever skips this gate. (`ops-deploy.sh --skip-accept` is a
  different gate: it compares the prod DB's `build` row, which
  production has never had; the deploy passes it and the report says so.)
- Reproducible and logged: committed SHAs only; `Cargo.lock` +
  `package-lock.json` inside the tarball; toolchain versions in every run
  log and stamp; `logs/<run>.log` plus the append-only `history.jsonl`.

Staging acceptance (builder-owned, standing approval), recipes in
`docs/ops/staging.md`:
- The exact SHA reaches plant by
  `git push plant:staging/handbook <sha>:refs/heads/xfer-*` (plant has no
  GitHub credentials, so no fetch), is built there, and the *staging*
  stack restarts on it (never an rsync of a working tree: `/healthz`
  cannot show a dirty build).
- Ports are fixed: public **18318**, admin **18319**, bound to
  `127.0.0.1` on plant and reached with
  `ssh -N -L 18318:127.0.0.1:18318 -L 18319:127.0.0.1:18319 plant`, so
  the URL is `http://127.0.0.1:18318/apostille`. Only the builder
  restarts this stack; a lane that needs live calls uses its own
  directory and store on plant and never binds these ports.
- Acceptance = the full scoreboard
  (`--base http://127.0.0.1:18318/apostille --out web/shots/staging-accept.json`)
  plus a live demo take
  (`DEMO_BASE_URL=http://127.0.0.1:18318/apostille npm --prefix web run demo`).
- Data: staging may reuse its DB/files, but reset and full delete are
  always allowed and never block. Every reused dataset has a regeneration
  recipe in `docs/ops/staging.md`. Important data lives in GCS (project
  `plant-andrii-ai`), access-locked, never world-readable; bucket
  credentials live only on plant (operator-installed), never in the
  repo. Fixtures stay synthetic: no customer data.
- Live model calls on staging use the staging-only Meta key at
  `~/.config/apostille/meta-staging.env` on plant (0600; source it in the
  remote shell only, never print, copy or commit its value), on synthetic
  documents or public pages only.
- Staging-green means every row that passes on production passes on
  staging, no staging row has an `HTTP`, `->`, `visible "`, `error:` or
  `timed out` problem, or more console errors, than production's same
  row, and the live demo take is green. Beyond that, rows red on both do
  not count (S1 until the operator's steps; GP3 until the logos). A run
  that is not staging-green blocks the deploy and nothing else.
