# Plant build pipeline (async, gated, rootless)

Builds and full test suites run on the `plant` box (reached as
`ssh plant` over LAN/VPN, Ubuntu x86_64), never blocking local work. Every
deploy candidate is validated by the full suite on plant; a red
gate stops the job before anything is staged for production.

## Rules (mirrored in docs/ops/deploy.md § Plant)

- Never root on plant. Never ask for root. Everything runs as
  the unprivileged plant user in `~/apostille-pipeline/`.
- Unprivileged ports only (reserved range `18080-18999`).
- Reproducible inputs: jobs ship a `git archive` tarball of a
  committed SHA (`Cargo.lock` + `package-lock.json` inside).
  Dirty trees are refused at submit time.
- Gated: stage order is `setup → deps → gate → package`.
  `gate` is `cargo xtask ci` (the whole monorepo suite).
  Any failure stops the job: no package, no staging, no prod.
- Async: `submit.sh` returns in seconds. Poll with `status.sh`.
- Logged: `logs/<run>.log` (full) + `logs/<run>.<stage>.log`.
- History: `history.jsonl`, one JSON object per stage/final event.
- The build pipeline never touches production. `artifacts/<run>/`
  holds the staging dir a later explicit deploy step may ship. (The
  separate work-queue worker below only dials OUT to production.)

## Layout on plant (`~/apostille-pipeline/`)

```
bin/          worker.sh + status.sh copies (from this dir)
queue/        <run>.json + <run>.tar.gz awaiting the worker
running/      claimed jobs
done/         passed jobs (result JSON)
failed/       failed/stopped jobs (result JSON)
logs/         <run>.log + <run>.<stage>.log
runs/         <run>/src extracted source (debug, pruned >3d)
artifacts/    <run>/ staging dir (gate-passed builds only)
history.jsonl append-only event log
worker.lock   flock singleton
worker.log    worker loop log
```

## Usage (from this repo on the Mac)

```sh
# one-time (idempotent): dirs, worker install + start, browsers
./ops/plant-pipeline/bootstrap.sh

# submit HEAD (async, returns in seconds)
./ops/plant-pipeline/submit.sh
./ops/plant-pipeline/submit.sh --type probe   # fast toolchain check
./ops/plant-pipeline/submit.sh <sha>          # a specific commit

# poll (never blocks the worker)
./ops/plant-pipeline/status.sh                # queue + recent history
./ops/plant-pipeline/status.sh <run-id>       # one job + log tail
```

## Worker

`worker.sh loop` persists via a user-crontab watchdog (every
minute; cron jobs live under `system.slice` and survive logout).
This is the real persistence because plant has `Linger=no`: the
systemd user manager stops user units at logout, and enabling
linger needs root (forbidden). A systemd user unit
(`apostille-pipeline-worker.service`) also exists for fast pickup
while logged in. Singleton via `flock -n`: whichever starts
first holds the lock, the other exits quietly. One job at a
time, 10 s poll. Worst-case pickup after a reboot is ~60 s.

## Work-queue worker (P392)

Production keeps pending work items (research, scrapes, evals) in
`work.sqlite`; `cargo xtask worker` on plant dials OUT to
`wss://andrii.ai/apostille/api/v1/worker/ws`, checks one job out at a
time, heartbeats its lease, and uploads the result (spec
`docs/specs/work-queue.md`). Staff queue jobs and download results on
the admin Work queue page (`/work`). Plant never accepts a connection
and holds no production credential except the worker token.

Layout on plant (operator decision 2026-11-03, `docs/ops/deploy.md` § Plant):

```
~/apostille-worker/src/          source of one committed SHA (.sha inside)
~/.config/apostille/worker.env   APOSTILLE_WORKER_TOKEN + META_API_KEY, mode 0600
/srv/scratch/apostille/work/     worker.lock, worker.log, job-<id>-a<n>/ scratch
/srv/scratch/apostille/fetch-cache/  URL-keyed page cache (reruns re-read)
/srv/scratch/apostille/meta-cache/   Meta call cache (record/replay layer)
/srv/scratch/apostille/models/   self-hosted weights (precheck.glimmer)
/srv/scratch/apostille/target/   cargo target dir for the worker build
~/apostille-pipeline/gpu.lock    held by GPU kinds for the whole job
```

Setup (from the Mac; idempotent):

```sh
# 0. once, as root on plant: the scratch root the worker writes to
ssh -t plant 'sudo mkdir -p /srv/scratch/apostille && sudo chown "$(id -un):" /srv/scratch/apostille'
# 1. token: mint once, save it as worker.token in admin /config
openssl rand -hex 32
# 2. secrets file on plant (paste the token and plant's Meta key)
ssh plant 'mkdir -p ~/.config/apostille && umask 077 && cat > ~/.config/apostille/worker.env'
#   APOSTILLE_WORKER_TOKEN=<64 hex chars from step 1>
#   META_API_KEY=<plant's Meta API key>
#   (Ctrl-D ends the input)
# 3. dirs, source of HEAD (or a SHA), release build, cron watchdog
./ops/plant-pipeline/worker-install.sh            # add --restart after an update (drains)
# 4. smoke: watch it connect, then check the admin Work queue page
ssh plant 'tail -n 20 /srv/scratch/apostille/work/worker.log'
```

The watchdog is a user-crontab line, same reasoning as the pipeline
worker above (Linger=no, so cron is the real persistence; the
`worker.lock` flock keeps one copy, extra starts exit quietly).
`worker-run.sh` moves `worker.log` to `worker.log.1` past 10 MiB while
no worker holds the lock, then starts the worker:

```
* * * * * sh $HOME/apostille-worker/src/ops/plant-pipeline/worker-run.sh wss://andrii.ai/apostille/api/v1/worker/ws # apostille-worker
```

Restarts drain: `worker-install.sh --restart` creates
`/srv/scratch/apostille/work/worker.drain`; the running worker takes no
new job, finishes and uploads the current one, removes the file and
exits, and cron starts the new build within a minute. `status.sh`
shows both workers and the queue worker's last log lines.

Disconnects reconnect with backoff (1 s doubling to 60 s); a job keeps
running through a dropped socket and completes on the next session
with the same lease token. A crashed worker's lease expires after 300 s
and the job requeues (3 attempts by default, then `failed`; staff can
Retry). One-off runs without the server:
`cargo xtask worker --run-local job.json` with
`{"kind": "scrape.urls", "payload": {"urls": ["https://…"]}}`.

## Failure semantics

- `deps`/`gate` failure → job to `failed/`, `result: "failed"`,
  reason in history. Nothing packaged.
- Worker crash mid-job → job stays in `running/`; next worker
  start re-queues it once (`attempt: 2`), then fails it holding
  the crash log. Jobs never silently vanish.
- Deploy eligibility: `acceptance/<sha12>.json` exists with
  `ci_green: true` AND `done/<run>.json` has `result: "passed"`.
  The deploy step (a later slice) must re-check both before
  shipping. No stamp, no ship — no flag ever skips this gate.
  (`ops-deploy.sh --skip-accept` is a different gate — the prod DB
  `build` row — scoped by docs/ops/deploy.md, not this pipeline.)
