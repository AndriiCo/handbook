# Plant build pipeline (async, gated, rootless)

Builds and full test suites run on the `plant` box (`ssh plant`,
user `chandra`, Ubuntu x86_64), never blocking local work. Every
deploy candidate is validated by the full suite on plant; a red
gate stops the job before anything is staged for production.

## Rules (mirrored in AGENTS.md §8)

- Never root on plant. Never ask for root. Everything runs as
  `chandra` in `~/apostille-pipeline/`.
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
- Plant never touches production. `artifacts/<run>/` holds the
  staging dir a later explicit deploy step may ship.

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

`worker.sh loop` runs as a systemd user service
(`apostille-pipeline-worker.service`, no root) with a `setsid`
fallback. Singleton via `flock -n`. One job at a time, 10 s poll.
After a plant reboot any ssh login re-arms it via `.bashrc` hook
(the hook is a no-op when the worker already holds the lock).

## Failure semantics

- `deps`/`gate` failure → job to `failed/`, `result: "failed"`,
  reason in history. Nothing packaged.
- Worker crash mid-job → job stays in `running/`; next worker
  start re-queues it once (`attempt: 2`), then fails it holding
  the crash log. Jobs never silently vanish.
- Deploy eligibility: `acceptance/<sha12>.json` exists with
  `ci_green: true` AND `done/<run>.json` has `result: "passed"`.
  The deploy step (a later slice) must re-check both before
  shipping. No stamp, no ship — no `--skip-*` flags, ever.
