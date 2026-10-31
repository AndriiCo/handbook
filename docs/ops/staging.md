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

## Regeneration (fresh staging DB)

1. `ssh plant 'pkill -f staging/handbook/target/release/web; rm -rf
   ~/staging/store ~/staging/kek ~/staging/creds'`
2. Re-run: sync, build (skip if tree unchanged), init, enroll, ports,
   seed, boot, tunnel, scoreboard — commands above verbatim.
