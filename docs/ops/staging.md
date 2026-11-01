# Staging → prod pipeline (the only deploy path)

Single box each end: staging on `plant`, prod on `andrii.ai`. Every
release: plant-gate stamp → exact-SHA staging build → staging
acceptance (scoreboard 9/10 + live demo 3/3) → `ship-release.sh`
build + stage → `ops-deploy.sh` → prod-after scoreboard, rollback
on any regression. Alias only, never a username, IP or key. Staging
is disposable (reset and full delete always allowed) and holds
synthetic fixtures only: no prod credentials, no SSH to prod, no
prod DB copies — ever.

## Steps (`ship-release.sh` is exact; this is the map)

1. Stamp: `ops/plant-pipeline/submit.sh <sha>` → plant gate
   (`cargo xtask ci`: fmt, clippy `-D warnings`, tests,
   secrets-scan) + package. The deploy needs the stamp for the
   exact SHA.
2. Sync the exact SHA to plant: `git push
   plant:staging/handbook <sha>:refs/heads/xfer-*` + checkout
   (plant has no GitHub creds, so no fetch). `BUILD_SHA`
   stamps the checked-out SHA.
3. Build on plant natively: `cargo build --release -p web
   -p xtask`, `npm run build` + `build:admin`.
4. Accept: forward 18318/18319 (`ssh -N -L 18318:… -L
   18319:… plant`), full scoreboard plus the live demo take
   (`DEMO_BASE_URL=http://127.0.0.1:18318/apostille`).
   9/10 with only operator-owned S1 red is staging-green.
5. Ship (Mac): `xtask/ops/ship-release.sh <sha>
   [--allow-today-overlap]` — plant stamp check, throwaway
   worktree, zigbuild x86_64 (`--target` is NOT optional),
   ELF + `BUILD_SHA` checks, both web builds, stage to
   `~/apostille/staging/<sha7>` (binary, SPAs, packs,
   static, xtask, evals, SHA, service), full root-process
   probe. Refuses on any failed rail, including the day gate
   (`--allow-today-overlap` only on explicit operator order).
6. Deploy (as hal): `bash ~/apostille/scripts/ops-deploy.sh
   ~/apostille/staging/<sha7> --skip-accept`
   (`--allow-today-overlap` only on explicit operator order —
   the script announces both). Auto-rolls-back on failed
   smoke; keeps live + newest 3 releases.
7. Verify: prod-after scoreboard against prod-before; roll
   back on any regression. Never the bare `xtask deploy` line.

## Pre-flight (every release, as hal — no root shell)

```sh
touch /srv/apostille/releases/.w && rm /srv/apostille/releases/.w && echo WRITE-OK
touch /srv/apostille/.w && rm /srv/apostille/.w && echo FLIP-OK
sudo -n systemctl status apostille >/dev/null && echo SUDO-OK
sudo -n -l | grep -q "stop apostille" && { echo "REFUSING: stop verb granted"; exit 1; } || echo STOP-ABSENT
echo 'SELECT 1;' | sudo -n -u andrii-ai /usr/local/sbin/apostille-sqlite >/dev/null && echo SQL-OK
printf '\n' | sudo -n /usr/local/sbin/apostille-journal >/dev/null && echo JOURNAL-OK
```

## Hosts

- `plant` staging: public **18318**, admin **18319** (store
  config — `run2.env` on plant is stale, ignore it). Server
  env: `STORE_DIR`, `SPA_DIR=…/web/build`,
  `STATIC_DIR=…/crates/web/static`,
  `CREDENTIALS_DIRECTORY` (0600 KEK copy). Start with
  `setsid -f` (without `-f` the ssh hangs forever); kill by
  verified PID, never `pkill -f` with your own command string
  in the pattern. Meta key for live runs:
  `~/.config/apostille/meta-staging.env` (0600, sourced in
  the remote shell only — record the path, never the value).
- `andrii.ai` prod: `enodia`, Ubuntu 26.04 x86_64. Serves
  ONLY the .41 v4/v6 addresses; research traffic (URL
  watching, fetches) originates ONLY from these prod IPs.
  Systemd unit runs `andrii-ai:andrii-ai`, in-binary ACME,
  KEK root-held (`/etc/apostille`, `LoadCredential`). No TPM
  → file-backed KEK is ACTIVE (revisit on hardware change).
  Releases in `/srv/apostille/releases`, live + newest 3.

## Parity (Mac dev vs prod box)

Pure Rust, no system libs: bundled SQLite, in-process rustls,
`cargo zigbuild --target x86_64-unknown-linux-gnu` (verify
ELF + max GLIBC at stage time). 9 jurisdiction packs validate
(`cargo xtask packs-validate`). Toolchain skew is resolved by
matching `rustc --version`, never by relaxing lints.

## Watch (monitoring, backup, posture)

- Liveness: `GET /apostille/healthz` → 200 + `{"status":"ok"}`;
  alert on 3 consecutive failures. Certs renew in-binary
  (ARI); weekly check `notAfter - now > 21 days`. Disk alert
  80%. Daily: 5xx count over the last hour; any spike →
  check, then roll back per § Steps 7.
- Backup: nightly SQLite backup API (never raw-copy WAL) +
  `tar` of `store/`, offsite, 7 daily + 4 weekly. Quarterly
  restore drill onto staging. Secrets backed up separately,
  encrypted, author-only.
- Posture: SSH key-only; firewall 80/443 (+UDP 443 with
  QUIC); unit hardening (`NoNewPrivileges`,
  `ProtectSystem=strict`); no PII in repo/logs/commits
  (`secrets-scan` + monthly server-log grep); suspected
  breach → rotate keys, ship audit log, then investigate.

## Retired (merged here 2026-11-01; process unchanged)

`staging-plan.md` (plant is the box; gated deploys won — the
conditional and cutover list are moot), `parity.md`,
`prod.md`, `deploy.md` (Oct 26–27 history: interim user-space
flow, root install), `runbook-release.md` (superseded by
`ship-release.sh` + `ops-deploy.sh`; §0 pre-flight kept
above verbatim).

## Run log

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
- P150/P151 proof (2026-11-01): P150's 3 guessed vietnam URLs all 404'd
  in the staging fetch (member page, e-register, www.mofa.gov.vn/en-us).
  P151 replaced them with 2 verified-200 URLs: HCCH authorities1/?cid=41
  (capture=9, names Viet Nam's authority) and https://mofa.gov.vn/
  (capture=10, Liferay chrome). analyze --live: cap9 proposed=0
  rejected=3 (nav-boilerplate quotes refused), cap10 proposed=0 (no
  quotable text on portal root). Staging facts stand at vietnam 8 +
  usa-wa-sos 9 accepted (Run 2's set). Full rescore via the kept tunnel:
  9/10, only S1 red (binary still 925f360; packs-only change).

## Regeneration (fresh staging DB)

1. `ssh plant 'pkill -f staging/handbook/target/release/web; rm -rf
   ~/staging/store ~/staging/kek ~/staging/creds'`
2. Re-run: sync, build (skip if tree unchanged), init, enroll, ports,
   seed, boot, tunnel, scoreboard — commands above verbatim.

## Run 3 (2026-11-01, c5e16e3 → edcfb84) — 8/10, ledger crash fixed
- Exact checkout via `git push plant:staging/handbook <sha>:refs/heads/xfer-*`
  (plant can't fetch; tar overlay rejected by checkout, bundle refused empty —
  push-to-side-ref works). `git checkout -f` after deleting paths that are
  untracked-but-tracked-upstream. BUILD_SHA stamps the checked-out SHA.
- Run2.env is stale (PUB=18317 = pipeline artifact server edeedc2, not ours).
  Real staging: public **18318**, admin **18319** (store config). Tunnel the
  same two ports; a pre-existing tunnel from the earlier session survived —
  reuse it (new ssh fails "Address already in use" = tunnel alive, not an error).
- Remote start: `setsid` WITHOUT `-f` hangs the ssh forever (server runs in
  foreground, health check never runs). Always `setsid -f`, or terminate the
  hung ssh — the setsid'd child survives. Never `pkill -f <pattern>` whose
  string appears in your own remote command (self-kill).
- c5e16e3 scored 7/10: P0/GP2-data/A1/GP1/GP4/A2/A5 pass; GP2-page 500'd
  (each_key_duplicate: conflict cliques share sibling ids → duplicate
  `conflict-*` ledger keys → render crash); GP3 passkey below fold; S1 operator-owned.
- P169 (edcfb84): `seedLedgerEvents` in handbook.js, parent-qualified
  conflict ids + clique regression test. SPA rebuilt, no restart needed.
  `--only GP2-page`: PASS (8 facts visible, live WS).
- P170 (088565c): login form-first on mobile → `--only GP3` PASS; full
  rescore 9/10 (only S1 red). P171 (7aa591a): demo spec live-base fix —
  fixture demo 3/3, live demo vs staging 3/3 (first real live take; prior
  live runs doubled /apostille and passed vacuously). Staging tree exact
  at 7aa591a (xfer ref; SPA rebuild unneeded — web/src identical).
  LIVE for demo: `DEMO_BASE_URL=http://127.0.0.1:18318/apostille`.

## Run 4 (2026-11-01, daa7768) — 9/10 kept, finding-12 live proof
- `run:{id}` staff-only WS topic (`crates/auth`, parse+authz+tests);
  `POST captures/{id}/draft` + `GET draft-runs/{id}` (admin, staff);
  trace console per-capture Draft button with run-state polling.
- Live proof on staging (vietnam capture 10): run 1 finished in 27 s,
  model `muse-spark-1.3`, proposed=2, rejected=0; the 2 proposed facts
  sit in the review queue. Tokens not surfaced (run_analysis reports
  no usage — follow-up). Steps also publish to `run:{id}` on the bus.
- Staging admin auth recipe (release refuses `admin.mtls_off`, so the
  seam is debug-only): debug binary + `config-set admin.mtls_off=true`
  + minted staff session + `Host: admin.andrii.ai`,
  `Origin: https://admin.andrii.ai`, `hb_csrf`/`x-csrf` double-submit.
  Reverted after (session revoked, seam false, release binary back).
  Tunnel still the surviving pre-existing forward.

## Run 5 (2026-11-01, 69ca7fa) — 9/10 kept, usage end to end
- `RunReport.usage` (input/output tokens, retry included; zero on
  skipped paths) flows into `run_finished` events, `draft-runs/{id}`
  state and the trace button label. Proved by a runner usage assertion
  (BadTopicClient {1,1} rides along on the rejected path) — no new
  live call spent (staging run 1 already gave model + 27 s + 2/0).
- Staging exact at 69ca7fa, release binary, rescore 9/10 (only S1 red).

## Run 6 (2026-11-01, 1aa6039) — 9/10 kept, scripts move
- `docs/ops/scripts/` → `xtask/ops/` (comment-only Rust change);
  release rebuild + restart, rescore 9/10 (only S1 red), live demo
  3/3 on the exact SHA.

## Run 7 (2026-11-01, 117ef58) — 9/10 kept, WS topics stack
- Plant stamp `acceptance/117ef58b75e6.json` (`ci_green: true`).
  First submit (56fb8ee) failed the gate on two
  `clippy::unnecessary_to_owned` in `xtask/src/plant_run.rs` (plant
  runs clippy 1.99, newer than the Mac) — P204 drops them, resubmit
  passed.
- Exact checkout via xfer ref, release rebuild, restart on
  18318/18319 (DB reused: vietnam 8 + usa-wa-sos 9 accepted stand).
  Restart lesson: the old server runs as `./target/release/web`
  (relative path), so a `staging/handbook/...` pkill pattern misses
  it; and never pkill a pattern that also appears literally in your
  own remote command (self-kill — it took staging down once, back
  up in minutes, nothing else touched).
- Rescore 9/10, only S1 red (operator-owned licence/video/
  visibility). Live demo take 3/3 on the exact SHA
  (`DEMO_BASE_URL=http://127.0.0.1:18318/apostille`).
- Staging-green deploy candidate for Monday 11-02 (Sunday spent,
  no order): stamp + 9/10 + live demo 3/3 all on 117ef58.

## Run 8 (2026-11-01, 5bb462a) — 9/10 kept, orders flow live on staging
- Plant gate first failed on `2a826de` (pre-guide CUJ locator went
  strict-mode ambiguous: the dossier guide quotes fact statements —
  P219 scopes it to the Fees region, verified locally, resubmitted).
  Stamp `acceptance/5bb462a383cb.json` (`ci_green: true`); short-SHA
  submits stamp under the short name, which `ship-release.sh` rejects —
  always submit the full SHA.
- Exact checkout via xfer ref, native release rebuild + both SPAs,
  restart on 18318/18319 by verified PID (DB reused: vietnam 8 +
  usa-wa-sos 9 accepted stand). `healthz` → `0.1.0+5bb462a`.
- Rescore 9/10, only S1 red (operator-owned licence/video/
  visibility). Live demo take 3/3 on the exact SHA
  (`DEMO_BASE_URL=http://127.0.0.1:18318/apostille`).
- Staging-green deploy candidate (Sunday open, cap lifted until 11-10):
  stamp + 9/10 + live demo 3/3 all on 5bb462a. Ship blocked on the
  unit-file drift rail: repo `docs/ops/apostille.service` line 6 is a
  P180 comment retarget vs installed — operator root step on
  `docs/ops/operator-today.md` item 1 (install + daemon-reload, no
  restart), then re-run ship + deploy.

## Run 9 (2026-11-01, 2cce16d) — 9/10 kept, guide v1 on staging
- Plant stamp `acceptance/2cce16d4cfe8.json` (`ci_green: true`).
- Exact checkout via xfer ref, native release rebuild + both SPAs,
  restart on 18318/18319 by verified PID (DB reused: vietnam 8 +
  usa-wa-sos 9 accepted stand). `healthz` → `0.1.0+2cce16d`.
- Rescore 9/10, only S1 red (operator-owned licence/video/
  visibility). GP2-data + A1 pass on the reused stand; GP2-page
  passes with the guide live. Live demo take 3/3 on the exact SHA
  (`DEMO_BASE_URL=http://127.0.0.1:18318/apostille`).
- Staging-green deploy candidate (Sunday open, cap lifted until
  11-10): stamp + 9/10 + live demo 3/3 all on 2cce16d. Ship needs
  the unit-file root step first (unchanged), plus prod
  `admin.allow_ips` set to the operator's IP before the binary
  starts (P221 default is loopback-only).
