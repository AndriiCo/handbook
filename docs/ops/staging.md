# Staging acceptance (`plant`, AGENTS.md §8)

Run log + regeneration recipe. Staging is disposable: reset and full
delete are always allowed. No prod credentials, no SSH to prod, no prod
DB copies — ever. Alias only, never a username, IP or key.

## Run 1 (2026-10-30, main @ 801b2a3) — 5/10, infra green

- Sync: `rsync -az --exclude target/ --exclude web/node_modules/
  --exclude web/build/ --exclude web/build-admin/ --exclude web/shots/
  --exclude store/ ./ plant:~/staging/handbook/` (plant has no GitHub
  creds, so `git fetch` is not an option; later runs are delta rsyncs).
- Toolchain on plant: cargo, node, npm, rsync, git. x86_64, 16 cores.
- Build: `npm --prefix web ci`, `cargo build --release -p web -p xtask`
  (native, ~2 min), `npm --prefix web run build` + `build:admin`.
- Ports (dynamic per run, recorded here): public 50369, admin 34515.
- Store: `xtask init --store ~/staging/store`;
  `config-enroll --kek-file ~/staging/kek` (enroll generates the KEK;
  staging-only file, 0600, never leaves plant);
  `config-set --key net.port / net.admin_port`.
- Server env: `STORE_DIR`, `SPA_DIR=$HOME/staging/handbook/web/build`,
  `CREDENTIALS_DIRECTORY=$HOME/staging/creds` (contains a 0600 copy of
  the enrolled KEK as `handbook-kek`; release builds refuse dev KEK env).
  Start detached; it logs to `/tmp/staging-web.log`.
- Seed: `packs-load --packs` (vietnam-routing + usa-wa), `fetch-slug`
  per slug, stub `analyze --scan-captures`, stub `eval` (quality.json).
- Tunnel (macOS has no `setsid`): route through the managed session —
  `ssh -N -L <pub>:127.0.0.1:<pub> plant`, then
  `node .agents/scoreboard/scoreboard.mjs
  --base http://127.0.0.1:<pub>/apostille`.
- Result: P0/GP1/GP3/GP4/A5 pass; GP2-data + A1 need accepted facts
  (live drafts + /review approval); GP2-page needs only the facts
  (h1/WS/clean already pass); A2 POST is 503 until verdicts are
  recorded (needs `META_API_KEY`); S1 reads the local repo, not staging.

## Run 2 (2026-10-31, main @ 925f360 + live-path fixes) — 9/10, data green

- Build: delta rsync, `cargo build --release -p web -p xtask` (BUILD_SHA
  stamps the git SHA — plant tree has `.git`), `npm ci`, `npm run build`
  + `build:admin`. No pipeline artifact (its BUILD_SHA names the wrong SHA).
- Ports (this run, recorded here): public **18318**, admin **18319**.
  18317 belongs to a pipeline artifact server (not ours — leave it).
- Store: fresh `~/staging/store`; `config-enroll --kek-file ~/staging/kek`;
  `config-set net.port/net.admin_port`. Server env: `STORE_DIR`,
  `SPA_DIR=~/staging/handbook/web/build`,
  `STATIC_DIR=~/staging/handbook/crates/web/static`,
  `CREDENTIALS_DIRECTORY=~/staging/creds` (0600 KEK copy). Release binary,
  `nohup … &`, log `/tmp/staging-web2.log`. Currently RUNNING — leave it.
- Staging Meta key: plant `~/.config/apostille/meta-staging.env` (0600,
  one `META_API_KEY=` line). Source with `set -a; . <path>; set +a` (the
  file has no `export`) in the remote shell only; never print, copy or log
  the value. Record the path, never the value.
- Data: packs-load, fetch-slug (2+3 fetched, 1 failed each), `analyze
  --live` (vietnam 8 proposed, usa-wa-sos 9), 17/17 accepted through the
  admin review API (staff session minted in staging DB + Origin + CSRF;
  release builds refuse `admin.mtls_off`, so no dev seam — approvals used
  the real session path), A2 recordings installed to
  `~/staging/store/demo_precheck` (clean=pass, dark/wash=retake, 0 SAM
  boxes on flat synthetics), `eval --live --evals evals/authorities`
  (26 gold, quote_found 0.7, muse-spark-1.3 → quality.json).
- Live-path fixes proven here (commit with next green tree): chat role
  `developer`→`user` (endpoint 400s otherwise), 32k completion budget
  (reasoning tokens), 8 records/call, Spark-vision verdicts in
  a2-record.py, all-sections dossier (GP2-page needs 8 statements in text).
- Tunnel (keep between slices; macOS has no `setsid`, managed session):
  `ssh -N -L 18318:127.0.0.1:18318 -L 18319:127.0.0.1:18319 plant`, then
  `node .agents/scoreboard/scoreboard.mjs --base
  http://127.0.0.1:18318/apostille`.
- Result: 9/10 (only S1 red: licence/video/visibility are operator-owned).
  Stale-note rule: whenever a stack starts, its ports + tunnel go here.
- P147 proof (2026-11-01): delta rsync; packs-load +2 vietnam sources;
  fetch-slug vietnam urls=5 fetched=3 (full-text ?cid=41 capture=7,
  section hub capture=8, mofa re-fetch capture=6) failed=0; analyze
  --live vietnam proposed=3 (cap6: 2, cap7: 1 with 6 quote-refusals,
  cap8 hub: 0 draftable — kept as change-watch). Rescore 9/10, only S1 red.

## Regeneration (fresh staging DB)

1. `ssh plant 'pkill -f staging/handbook/target/release/web; rm -rf
   ~/staging/store ~/staging/kek ~/staging/creds'`
2. Re-run: sync, build (skip if tree unchanged), init, enroll, ports,
   seed, boot, tunnel, scoreboard — commands above verbatim.
