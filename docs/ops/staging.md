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
  alert on 3 consecutive failures. The deploy smoke stays on
  `/healthz`. Certs renew in-binary
  (ARI); weekly check `notAfter - now > 21 days`. Disk alert
  80%. Daily: 5xx count over the last hour; any spike →
  check, then roll back per § Steps 7.
- Readiness: `GET /apostille/readyz` reports per-check JSON
  (`database`, `tls_certificate`, `kek`, `timers`) — 200 ready,
  503 with the failed checks named. Plain-HTTP boots (staging,
  `xtask up`) record no serving leaf and never report ready, so
  monitoring targets `/readyz` on production only.
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

## Run 10 (2026-11-01, 0f53999) — 9/10 kept, admin live on staging
- Plant stamp `acceptance/0f53999e0724.json` (`ci_green: true`,
  run `20261101-185456-0f53999`).
- Exact checkout via xfer ref (`xfer-p225`), native release rebuild
  + both SPAs, restart on 18318/18319 by verified PID (DB reused:
  vietnam 8 + usa-wa-sos 9 accepted stand).
  `healthz` → `0.1.0+0f53999`.
- Rescore 9/10, only S1 red (operator-owned licence/video/
  visibility). Live demo take 3/3 on the exact SHA
  (`DEMO_BASE_URL=http://127.0.0.1:18318/apostille`).
- Live admin check (debug binary `0.1.0+0f53999` + `config-set
  admin.mtls_off=true` + minted staff session, all reverted after:
  session revoked, seam false, release binary back): Chromium on
  `127.0.0.1:18319/trace` clicked Draft → POST 202 (real CSRF
  double-submit) → `run:{id}` socket reported `Done — 2 proposed,
  0 rejected (muse-spark-1.3, 18862/8785 tok)`, zero console errors.

## Run 11 (2026-11-01, b74eb5c) — 9/10 kept, P1(1) + notify drill live
- Plant stamp `acceptance/b74eb5c5726d.json` (`ci_green: true`,
  run `20261101-201638-b74eb5c`). P232's run failed the gate on
  four clippy-1.99 lints in new `crates/notify` (P234 fixed).
- Exact checkout via xfer ref (`xfer-p234`), native release rebuild
  + both SPAs, restart on 18318/18319 by verified PID (DB reused).
  `healthz` → `0.1.0+b74eb5c` (guardrails live).
- Rescore 9/10, only S1 red. Live demo take 3/3 on the exact SHA.
- P1(1) live: slow-header socket closed at exactly 10.0 s, zero bytes.
- Notify drill (staging-only, file mailer — no external sends):
  `mailer.provider=file`, `notify.enabled=1`, `notify.daily_cap=3`,
  `notify.postal_address` = clearly-marked drill placeholder (the
  operator's real address still needed before production use).
  Subscribe `notify-drill@example.org` (synthetic) → 200 + confirm
  file; confirm → ok, replay → 410; synthetic `fact_accept` audit
  row over real accepted fact 17 → digest file within the 60 s tick
  with quote/source/date, dossier link, RFC 8058 pair, postal footer.
  One-click unsub → ok; resubscribe → pending. Drill found one bug:
  dossier link doubled `/apostille/apostille` (P235 `origin` fix,
  re-proof pending its stamp).
- Staging-green deploy candidate (Sunday open, cap lifted until
  11-10): stamp + 9/10 + live demo 3/3 + live admin check all on
  0f53999. Ship needs the unit-file root step first (unchanged),
  plus prod `admin.allow_ips` set to the operator's IP before the
  binary starts (P221 default is loopback-only). The 2 draft
  proposals from the live check sit in the staging queue
  (harmless synthetic data; approve or reject at will).

## Run 12 (2026-11-02, 3836bfb) — 9/10 kept, P240–P250 stack accepted
- Plant stamp `acceptance/3836bfb88b38.json` (`ci_green: true`,
  run `20261102-010558-3836bfb`: setup 0 s, deps 3 s, gate 210 s,
  package 134 s). First submit of the night failed the gate on
  `clippy::items-after-test-module` (plant clippy 1.99 vs older
  Mac toolchain — P250 moves `mod tests` to file end; plant is
  the lint source of truth).
- Exact checkout via xfer ref (`xfer-p250`), native release
  rebuild (31 s incremental) + both SPAs, restart on 18318/18319
  by verified PID (old 1583345, new 2498693; 18317 is a pipeline
  artifact server — untouched). DB reused (vietnam 8 +
  usa-wa-sos 9 accepted stand). `healthz` → `0.1.0+3836bfb`.
- Scoreboard ON plant (Mac Chromium broken in sandbox):
  `web/shots/staging-accept.json` 9/10, only S1 red
  (licence/video/visibility, operator-owned). GP2-data, A1,
  GP2-page, A2, A5 all pass on the reused stand.
- Live demo take 3/3 on the exact SHA
  (`DEMO_BASE_URL=http://127.0.0.1:18318/apostille`, run on
  plant): desktop GP1→GP4, admin review+trace, mobile.
- Monday deploy candidate: stamp + 9/10 + live demo 3/3 all on
  3836bfb. Ship still needs the unit-file root step (item 1),
  prod `admin.allow_ips`, and an explicit operator deploy order
  (`--allow-today-overlap` only on order).

## Run 13 (2026-11-02, 1992959) — 9/10 kept, SA-1/SA-2/SA-3 stack accepted, timed

- Plant stamp `acceptance/19929594dcde.json` (`ci_green: true`,
  run `20261102-030206-1992959`): submit 03:02:06Z, claimed
  03:02:22Z (queue 16 s), setup 0 s, deps 4 s, gate 209 s,
  package 134 s, stamp 03:08:09Z. Plant total 363 s — within
  2 s of the 3836bfb run (210/134), so the gate is stable.
- Exact checkout via xfer ref (`xfer-p256`): push <1 s,
  `git checkout -f` 0 s. Native release rebuild
  (`cargo build --release -p web -p xtask`) 33 s incremental;
  both SPAs 12 s (SA-2 touched unsubscribe + trace pages, so
  no skip). Kill-by-PID 2498693 + `setsid -f` restart 3 s;
  `healthz` → `0.1.0+1992959`. DB reused (vietnam/usa-wa-sos
  stand). Staging total push→serving ≈ 48 s of work.
- Scoreboard ON plant 9/10 in 17 s, only S1 red
  (licence/video/visibility, operator-owned — untouched per
  standing rules). GP2-data, A1, GP2-page, A2, A5 pass.
- Live demo take 3/3 in 5 s on the exact SHA.
- Deploy candidate: stamp + 9/10 + demo 3/3 all on 1992959.
  Second Monday deploy only on explicit operator order
  (`--allow-today-overlap`), ready by 22:00 PST, never after
  23:59 PST; needs the unit-file root step + prod approvals.
- Timing notes (see builder report): gate 58% of plant time;
  xfer push + staging build can overlap the gate (saves
  ~6 min wall); staging rebuilds are incremental already;
  sccache on plant is the open expedite for the gate.

## Run 14 (2026-11-02, a39495a) — 9/10 kept, archive stack accepted

- Plant stamp `acceptance/a39495ab0c61.json` (`ci_green: true`,
  run `20261102-050130-a39495a`): submit 05:01:30Z, setup 1 s,
  deps 3 s, gate 225 s (longer: world.rs + warc.rs + crawl.rs +
  archive.rs are big new modules), package 147 s, stamp
  05:07:54Z. Plant total 384 s.
- Exact checkout via xfer ref (`xfer-p265`), native release
  rebuild 42 s (new flate2/sha1/data-encoding/webpki-roots edges
  in ingest) + both SPAs, kill-by-PID 2573024 + `setsid -f`
  restart 3 s; `healthz` → `0.1.0+a39495a`. DB reused.
- Scoreboard ON plant 9/10 in 16 s, only S1 red
  (operator-owned). Live demo take 3/3 in 5 s on the exact SHA.
- Deploy candidate: stamp + 9/10 + demo 3/3 all on a39495a
  (supersedes the 1992959 candidate). Second Monday deploy only
  on an explicit operator order, ready by 22:00 PST, never after
  Mon 23:59 PST; still needs the unit-file root step + prod
  approvals + brand SVGs + S1.

## Run 19 (2026-11-02, bde970e) — 9/10 kept, wave-2 stack accepted

- Plant stamp `acceptance/bde970e5b921.json` (`ci_green: true`,
  run `20261102-172905-bde970e`, stamp 17:37:11Z). P304
  (`8cafecc`, release-train gap) stamped first
  (`acceptance/8cafecc.json`, 17:31:10Z); P305 adds the
  wave-2 merges + SA-21 wiring.
- Exact checkout via xfer ref (`xfer-p305`), native release
  rebuild 56 s + `npm --prefix web ci` + both SPAs, kill-by-PID
  3587169 + `setsid -f` restart;
  `healthz` → `0.1.0+bde970e`. DB reused (vietnam 8 +
  usa-wa-sos 9 accepted stand).
- Scoreboard 9/10 in `web/shots/staging-accept.json`
  (17:45:45Z), only S1 red (operator-owned
  licence/video/visibility). Live demo take 3/3 on the exact
  SHA (`DEMO_BASE_URL=http://127.0.0.1:18318/apostille`).
- Deploy candidate: stamp + 9/10 + demo 3/3 all on bde970e
  (supersedes 2b4bc8d; adds SA-18 egypt-chain, SA-17
  north-america-106, SA-21 walk-in reports + wiring, P304
  release-train gap). Ships on the operator's next "deploy
  now" via `release-train.sh` (no day gate anymore).
- Tip `4dcd234` stamped `ci_green: true` (run
  `20261102-174624-4dcd234`, 17:52:29Z; SA-15 merge
  `294c5c2` also queued). README/docs-only since bde970e, so
  the serving bits are identical — candidate moves to the tip.
  RO-2 `up --check` green on the tip in a fresh local clone
  (healthz + authorities + setup/status 200). Node matrix
  ready on the Mac: Node 22/npm 10 and Node 24/npm 11 via
  Homebrew keg-only (`PATH=/opt/homebrew/opt/node@22/bin:$PATH`
  or `node@24`; bare `npm` abort-traps — the paired node must
  lead PATH). `--from-archive` still needs a crawl package.

## Run 18 (2026-11-02, 2b4bc8d) — 9/10 kept, login-padding fix accepted

- Plant stamp `acceptance/2b4bc8d5d5b7.json` (`ci_green: true`,
  run `20261102-120244-2b4bc8d`, stamp 12:08:44Z).
- Exact checkout via pre-pushed xfer ref (`xfer-p298`), native
  release rebuild 23 s + `npm --prefix web ci` + both SPAs,
  kill-by-listening-PID + `setsid -f` restart;
  `healthz` → `0.1.0+2b4bc8d`. DB reused.
- Scoreboard ON plant 9/10 in `web/shots/staging-accept.json`,
  only S1 red (operator-owned licence/video/visibility).
  Live demo take 3/3 on the exact SHA.
- Deploy candidate: stamp + 9/10 + demo 3/3 all on 2b4bc8d
  (supersedes 6e16ded; adds P297 operator page + P298 login
  padding). Second Monday deploy only on an explicit operator
  order with `--allow-today-overlap`, ready by 22:00 PST, never
  after Mon 23:59 PST.

## Run 17 (2026-11-02, 6e16ded) — 9/10 kept, wave-1 stack accepted

- Plant stamp `acceptance/6e16dedb18f1.json` (`ci_green: true`,
  run `20261102-114309-6e16ded`): setup 0 s, deps 3 s,
  gate 217 s, package 137 s, stamp 11:49:15Z.
- Exact checkout via xfer ref (`xfer-p295`), native release
  rebuild 35 s + `npm --prefix web ci` + both SPAs, kill-by-PID
  3442846 + `setsid -f` restart; `healthz` → `0.1.0+6e16ded`.
  DB reused (vietnam 8 + usa-wa-sos 9 accepted stand).
- Scoreboard ON plant 9/10 in `web/shots/staging-accept.json`,
  only S1 red (operator-owned licence/video/visibility).
  GP2-data, A1, GP2-page, A2, A5 pass on the reused stand.
  Live demo take 3/3 on the exact SHA.
- Deploy candidate: stamp + 9/10 + demo 3/3 all on 6e16ded
  (supersedes eb57cdc). Contains SA-19 wiring (fedex-live),
  SA-20 translate, SA-16 es/fr reading, SA-6 open-weights code,
  gold set 30. Second Monday deploy only on an explicit operator
  order with `--allow-today-overlap`, ready by 22:00 PST, never
  after Mon 23:59 PST.

## Run 16 (2026-11-02, eb57cdc) — 9/10 kept, P-ADMIN-IP staged + proven

- Plant stamp `acceptance/eb57cdc05648.json` (`ci_green: true`,
  run `20261102-101340-eb57cdc`): setup 1 s, deps 3 s,
  gate 215 s, package 137 s. (An earlier run for 1dd14b0 also
  gated green; P284's e2e spec is web-only, Rust identical, so
  the tip re-stamped and staged.)
- Exact checkout via xfer ref (`xfer-p284`), native release
  rebuild 25 s + both SPAs + release xtask, kill-by-verified-PID
  + `setsid -f` restart; `healthz` → `0.1.0+eb57cdc`. DB reused.
- P-ADMIN-IP staging proof (temporary `admin.allow_ips`
  `203.0.113.7`, row deleted after — `config-get` confirms
  unset): boot log `admin.allow_ips effective = 203.0.113.7/32`;
  refused loopback peer got 403 JSON (`code ip_not_allowed`,
  `ip 127.0.0.1`, family + transport + fix named) and the static
  403 HTML page (no SPA/JS, Chromium screenshot
  `web/shots/p-admin-ip-403-1440.png`); UI error screenshot
  `web/shots/p-admin-ip-ui-1440.png`. Restored to unset
  (`loopback only (unset)` in boot log); loopback allowed again
  (`/me` 401, `/review` 200).
- Scoreboard 9/10 in `web/shots/staging-accept.json`, only S1 red
  (operator-owned licence/video/visibility). Live demo take 3/3
  on the exact SHA.
- Deploy candidate: stamp + 9/10 + demo 3/3 all on eb57cdc
  (supersedes f861e44). Contains B1-B5 + SA-7..SA-13 + P-ADMIN-IP
  (F1-F5). Second Monday deploy only on an explicit operator
  order with `--allow-today-overlap`, ready by 22:00 PST, never
  after Mon 23:59 PST.

## Run 15 (2026-11-02, f861e44) — 9/10 kept, B1-B5 + all lanes

- First stamp attempt for a27f13d failed the plant gate on
  `up::tests::wait_stops_when_web_exits` (expected Exited, got
  Healthy): the test probes fixed port 18318 and staging serves
  200 there during the gate — a Mac-green/plant-red race. Fixed
  in P278 (port-parameterized `wait_healthy_on`, test uses a
  freshly closed port; proven with a live 200-server on 18318).
- Plant stamp `acceptance/f861e44ae32e.json` (`ci_green: true`,
  run `20261102-094502-f861e44`): setup 1 s, deps 6 s,
  gate 217 s, package 138 s, stamp 09:51:15Z. Plant total 366 s.
- Exact checkout via xfer ref (`xfer-p278`), native release
  rebuild 35 s + both SPAs, kill-by-PID 2768464 + `setsid -f`
  restart 3 s; `healthz` → `0.1.0+f861e44`. DB reused.
- Scoreboard ON plant 9/10 in 16 s, only S1 red
  (operator-owned). Live demo take 3/3 in 9 s on the exact SHA.
- Deploy candidate: stamp + 9/10 + demo 3/3 all on f861e44
  (supersedes a39495a). Contains B1-B5 + SA-7..SA-13. Second
  Monday deploy only on an explicit operator order, ready by
  22:00 PST, never after Mon 23:59 PST; still needs the unit-file
  root step + prod approvals + brand SVGs + S1.
