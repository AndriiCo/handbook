# MISSION WIN lanes: shared rules (B0, 2026-11-02)

Plan: `docs/ops/mission-win.md` (operator-approved 11-02). Contracts and
route table: `docs/lanes/contracts.md`, JSON seeds in
`docs/lanes/contracts/`. Your own brief: `docs/lanes/sa-<NN>.md`. Rules:
AGENTS.md, which wins over this file. The builder is the Claude Code
session in the main checkout; it merges every `ready:` step at once.

## You are a lane
- A Claude Code session in your own git worktree, on your own branch
  (`sa-<NN>/<name>`). Commit only there. Never push `main`, never deploy,
  never restart the staging stack or bind plant ports 18318/18319.
- You own only the paths in your brief, plus `docs/lanes/sa-<NN>.md` and
  your own lines in `docs/prompts/LOG.md`. Need anything else (a new
  `main.rs` hook, a dependency, a `shots.mjs` ROUTES entry, a shadcn
  primitive, a config key, a WS kind)? Write a `request:` line in your
  `docs/lanes/sa-<NN>.md`, commit it, and keep working on something
  unblocked. Never end a turn waiting.
- Never edit: `AGENTS.md`, `.agents/`, `.muse/`, `docs/ops/**`,
  `crates/web/src/{main,admin,approval,ws_bus,draft}.rs`,
  `crates/auth/src/ws.rs`, `crates/config/src/lib.rs`, any `Cargo.toml`
  or `Cargo.lock`, `xtask/src/main.rs`, `web/package*.json`,
  `web/src/app.css`, any `+layout.svelte`, `web/src/lib/components/ui/**`,
  `web/scripts/{shots,smoke}.mjs`, `web/e2e/demo.spec.mjs`, `README.md`,
  `docs/demo-script.md`, or another lane's paths.

## Build environment (the Mac's disk is nearly full)
- Every shell: `export CARGO_TARGET_DIR=/Users/andrii/Code/handbook/target`.
  The pre-commit hook runs `cargo xtask secrets-scan`; without the
  variable it builds a second 2+ GB target in your worktree.
- Per-crate tests only (`cargo test -p web <filter>`, `cargo clippy -p
  <crate> --all-targets -- -D warnings`, `cargo fmt -p <crate>`). Never
  `cargo clean`, never `cargo xtask ci` locally (plant runs it on every
  merge), no `--release` builds except SA-29.
- Web lanes: `npm --prefix web ci` once in your worktree. Register your
  screens for the visual gate in `web/e2e/fixtures/lanes/sa-<NN>.json`
  (you own it; format at the top of `web/scripts/shots.mjs`: routes with
  `name`, `ready` text and `build: "admin"` for admin screens, API
  fixtures by pathname regex, recorded WS events by topic regex). Every
  screen needs exactly one `h1` built with the heading component.
- Live Meta calls run on plant only (`ssh plant`), from your own scratch
  directory `~/lanes/sa-<NN>/` with your own store (`xtask init --store
  ~/lanes/sa-<NN>/store`), with the staging key sourced in the remote
  shell (`set -a; . ~/.config/apostille/meta-staging.env; set +a`; never
  print, copy or commit it), on public pages or synthetic documents
  only. Push your SHA there with
  `git push plant:eval-v2 <sha>:refs/heads/xfer-sa<NN>-<n>` (or clone
  into your scratch directory from that repo).

## Product rules
- Only Meta models at runtime (`muse-spark-1.3`, `muse-wonder-1.0`,
  `sam-3.1`, `muse-voice-transcribe-1.0`), every call per
  `.agents/meta-api-findings.md` (no temperature, `reasoning_effort` set,
  stream long calls, jittered retries honouring Retry-After, refusal and
  length checks, stable developer prefix + `prompt_cache_key`). Every
  HTTP attempt writes one `model_call` row (SA-22 `telemetry.rs`).
- Runtime-only facts: no fee, address, hour, PO box, membership or form
  field is hardcoded in code or UI. Everything a visitor sees comes from
  accepted rows at runtime. Fixtures are synthetic and say so
  (`SYNTHETIC`, `example.test`).
- No schema migration: `CREATE TABLE IF NOT EXISTS` in your module's
  `migrate_tables` / `migrate_p2_tables` hook only; no ALTER, DROP,
  RENAME, no CHECK change, no `SCHEMA_VERSION` bump.
- Model output reaches the public page only through propose → a
  deterministic gate → staff accept → an ids-only event on
  `authority:{slug}` (`ws_bus::publish_authority_event`, kinds
  `fact_accepted`, `plan_accepted`, `offices_accepted`).
- Customer PII lives in `p2.sqlite` only; staff UIs show the
  `customer_ref`, never the email. Admin code never reaches the public
  build (`npm --prefix web run check:imports`).
- Design law: AGENTS.md §3 (shadcn-svelte on bits-ui, lucide, tokens in
  `app.css` only, skeletons not "Loading…", no banned strings).

## Hooks already wired by the builder (B0)
`crates/web/src/main.rs` "MISSION WIN lane hooks" calls, at boot and in
the prod and test routers alike:
- `migrate_tables(&conn)`: telemetry, plan, compile, factcheck, offices,
  forms_map. `migrate_p2_tables(&p2)`: orders, p2_labels, tracking.
- `public_routes()` (merged into `/apostille`, paths `/api/v1/…`): plan,
  stripe. `p2_routes()` (merged into the `/api/v1/p2` nest, relative
  paths): orders, forms_map. `admin_router(state)` (admin listener, paths
  `/apostille/api/v1/…`): telemetry, plan, compile, factcheck, offices,
  forms_map, fulfilment, admin_orders. `spawn(&state)`: reconcile,
  tracking.
- Seams: `offices::accepted_for_slug`, `forms_map::accepted_for_slug`,
  `factcheck::checks_for_slug` (read by `plan.rs`),
  `DraftRuns::reserve_id` (run ids for `run:{id}`),
  `ws_bus::publish_authority_event`, `ws_bus::publish_ops`, the
  staff-only `ops` WS topic, `AppState.config_cache`
  (`config::CachedReader`, 30 s TTL, invalidated by the config PUT), and
  the new config keys in `config::keys` (stripe.*, orders.*,
  labels.fedex_base_url + customer_*, usps.* , plan.auto_compose,
  compile.daily_runs, tracking.poll_enabled, meta.daily_call_cap).

## Steps, proof and the ready protocol
- One step at a time, each green before the next. Done means:
  - Rust: your crates' tests + clippy `-D warnings` + fmt, and a serde
    round-trip test of every contract seed you serve
    (`include_str!("../../../docs/lanes/contracts/<file>.json")`).
  - AI: a live staging run on plant (model, p50/p95 latency,
    input/output/cached/reasoning tokens), recorded responses saved as
    replay fixtures (scrubbed: no account numbers, tokens, addresses,
    tracking numbers), and the eval delta (before → after on the gold
    set; at most one gold case drop per rate).
  - Web: AGENTS.md §4 (shots of every touched route at 390/1440 light and
    dark from the built SPAs, check, check:imports, check:tailwind, both
    builds; the demo gate if you touch /review, /trace, /, /authority or
    /order). Inspect the PNGs.
- Commit `sa-<NN>-<n>: <what>` with one LOG line
  `| sa-<NN>-<n> | <UTC> | <first 80 chars of the step> | <outcome ≤15 words> |`.
- Then append to `docs/lanes/sa-<NN>.md`:
  `ready: sa-<NN>-<n> <sha7> — <what> — <gates run>` and commit. The
  builder merges it, alone, at once. After each builder merge, `git
  merge main` into your branch (LOG.md conflicts: keep both lines, time
  order) so you stay current.

## Trains (Pacific). Mark ready by the cut-off to ride the train.
| Train | Cut-off | Deploys |
|---|---|---|
| T1 | Mon 20:30 | Mon ~21:30-22:30 |
| T2 | Tue 07:30 | Tue ~09:00-10:00 |
| T3 | Tue 15:30 | Tue ~17:00-18:00 |
| T4 | Wed 07:00 | Wed by 08:30 (last) |
No lane merges after Wed 07:30 PST.
