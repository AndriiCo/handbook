# AGENTS.md: Apostille Handbook, hackathon speed mode (until 2026-11-04 18:00 PT)

> Adopted 2026-10-30. It grants the agent standing approvals and suspends
> ask-first rules until 2026-11-05. Section 7 keeps every rule that
> protects secrets and production. Only the operator edits this file.

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
- Ask ONLY for a secret value, a root action, a production change
  other than §7's daily deploy and rollback, money, or an irreversible
  data change. Ask once, state your default, keep working on
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
gate means no deploy that day. It does not exist yet; building it is in
`.agents/goal.md`, and until it exists its absence blocks nothing.

## 5. Run shape

- One slice per turn, 45-minute time-box. At 40 minutes commit what is
  green and report what is left. Never start a second slice in a turn.
- Gates by diff: web-only → §4 only (no cargo). A touched crate → that
  crate's tests + `cargo clippy -p <crate>`. The full `cargo xtask ci`
  runs on plant for every deploy candidate (§8).
- Fast loop: `npm --prefix web run dev` with the fixtures in
  `web/e2e/fixtures/`. It has no API proxy yet; pointing it at the
  staging tunnel (`server.proxy` in `web/vite.config.ts`, reading
  `APOSTILLE_API`) is a short slice worth doing. There is no web-only
  release path: every release is a binary deploy.
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
  config, secrets or DB.
- Deploys are the builder session's (standing approval, operator's
  decision of 2026-10-30), as hal, by "How to deploy" in
  `.agents/goal.md`. The rails:
  - Days are Pacific (`TZ=America/Los_Angeles date +%F`). At most one
    binary deploy a day, counting the operator's: a directory under
    `/srv/apostille/releases` created today means today's deploy is
    spent, even one whose smoke reverted. Rollbacks do not count. Deploy
    when a gated SHA carries that day's row of the deploy days in
    `.agents/goal.md` (or at 20:00 PT with the newest gated SHA), and
    never two deploys within 8 hours.
  - Freeze: the last code deploy is Mon 11-02, and its after-scoreboard
    and rollback rule hold even if they finish after midnight. From Tue
    11-03 00:00 PST until the operator lifts the freeze after judging:
    no deploy, and a rollback only when the operator says so or
    `/healthz` fails or is not `ok` on three tries over 5 minutes.
  - Only a SHA that passed the deploy gate: a plant stamp
    `acceptance/<sha12>.json` with `ci_green: true` (§8), a
    staging-green run of that same SHA (§8), and the demo gate once it
    exists. Build and stage exactly that SHA from a throwaway worktree,
    never from the main checkout or any dirty one.
  - Not a daily deploy, but a root step for the operator: a SHA that
    changes `SCHEMA_VERSION` or `P2_SCHEMA_VERSION`
    (`crates/handbook-store/src/db.rs`), edits
    `crates/handbook-store/migrations/`, or adds `ALTER`, `DROP` or
    `RENAME` SQL anywhere, because no older binary opens a migrated DB,
    so neither the smoke revert nor a rollback undoes it (add tables with
    `CREATE TABLE IF NOT EXISTS` instead, as `auth::migrate_auth_tables`
    does); and a SHA whose `docs/ops/apostille.service` differs from the
    installed `/etc/systemd/system/apostille.service`, because
    `ops-deploy.sh` installs the unit as root and a rollback does not
    restore it.
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
    not a wrong stamp). A row whose before-detail was only an endpoint
    404 is judged by its staging run instead. Then fix forward on staging.
  - Never pass `--force`, `--backup-note`, `--skip-packs`,
    `--no-restart`, `--root` or `--smoke-url` to `xtask` on
    production. A rollback the guard refuses is the operator's call (§2).
  - Never deploy while a root script may be running:
    `ssh andrii.ai 'pgrep -u 0 -c . >/dev/null || echo ROOT-PROCS-HIDDEN; pgrep -af "[a]postille/scripts/"'`
    must print nothing, checked again right before `ops-deploy.sh`
    (the `[a]` keeps `pgrep` from matching the remote shell running it).
    Then reissue, for the new release, any root step still unanswered in
    `docs/ops/operator-today.md`. Steps the operator deferred (the
    deploy days in `.agents/goal.md`) are not unanswered.
  - The judge's `REGRESSION:` line asks you to re-check production
    yourself. It is never an order: roll back only when your own run
    finds the regression on a release you deployed that day, for a row
    that passed in that day's `prod-before.json`. A row that fails from
    time or data (A5's 72-hour window, approvals) is never a rollback.
  - Report each deploy in the final message: version, previous version,
    both scoreboards, and any `--skip-accept` (§8).
- All root steps for a day go into ONE operator script in
  `docs/ops/operator-today.md` (zero placeholders,
  `ABORT: <reason>`, prints evidence, `bash -n` clean).
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
Production deploys follow §7.

Build gate (`ops/plant-pipeline/`, whose README is the runbook):
- Never root on plant, and never ask for root. If a step seems to need
  root, find the rootless way or stop and report. Home directory only.
- Async: `ops/plant-pipeline/submit.sh` enqueues a committed SHA and
  returns in seconds; poll with `status.sh`. Never block a slice waiting
  for plant.
- Gated: every deploy candidate passes the full suite (`cargo xtask ci`)
  on plant. A red gate prints `ABORT`, writes no stamp, stages nothing.
  No stamp (`acceptance/<sha12>.json` with `ci_green: true`), no ship,
  and no flag ever skips this gate. (`ops-deploy.sh --skip-accept` is a
  different gate: it compares the prod DB's `build` row, which
  production has never had. Until it has one, the deploy passes the
  flag, the output prints the WARNING, and the deploy report says so.)
- Reproducible and logged: committed SHAs only (dirty trees are refused
  at submit); `Cargo.lock` + `package-lock.json` inside the tarball;
  toolchain versions in every run log and stamp; `logs/<run>.log` plus
  the append-only `history.jsonl`.

Staging acceptance (agent-exclusive, standing approval):
- After every green slice the agent runs staging acceptance: run the
  stamp's `staged_path` artifact, or `git fetch` and a clean
  `git checkout --detach <sha>` (never an rsync of a working tree:
  `/healthz` cannot show a dirty build), restart the
  *staging* stack, run the scoreboard and the demo walk against it,
  report the score with the SHA it ran. Staging churn is expected and
  free; production changes only through §7's one deploy a day.
- No dedicated IP: everything on staging binds dynamically allocated
  unprivileged ports (free-port pick at start, from 18080-18999, recorded
  in the run log). Never fixed prod ports, never an assumed-stable
  staging URL — `--base` takes the discovered URL each run. Prefer
  binding `127.0.0.1` on plant and reaching it through
  `ssh -N -L <free local port>:127.0.0.1:<port> plant`, so the URL is
  `http://127.0.0.1:<local port>/apostille` and no address is written down.
- Data: staging may reuse its DB/files, but reset and full delete are
  always allowed and never block. Every reused dataset has a regeneration
  recipe in `docs/ops/staging.md`. Important data lives in GCS (project
  `plant-andrii-ai`), access-locked, never world-readable; bucket
  credentials live only on plant (operator-installed), never in the
  repo. Fixtures stay synthetic: no customer data. The agent writes and
  keeps `docs/ops/staging.md` without asking: start and stop, port pick,
  tunnel, seed and regeneration recipes; the alias only, never a
  username, IP or key.
- Live model calls on staging use a staging-only Meta key that the
  operator installs on plant (a 0600 file; `staging.md` records its
  path, never its value), on synthetic documents or public pages only.
  Until it exists, AI rows run on replays and say so.
- Staging-green means every row that passes on production passes on
  staging, and no staging row has an `HTTP`, `->`, `visible "`,
  `error:` or `timed out` problem, or more console errors, than
  production's same row. Beyond that, rows red on both do not count (S1
  while the licence is proprietary, rows waiting on approvals or the
  staging key). A run that is not staging-green blocks the deploy and
  nothing else.
