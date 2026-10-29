# Deploy drill ×3 (roadmap item 11)

Three drills, each run to green THREE times before the Day-7
signoff. Every run is logged in the table at the bottom with date,
operator, build sha, and result. A failed run stops the count for
that drill (fix, then restart its ×3).

## Drill 1 — ship + smoke (prod-direct while G5 is relaxed)

Per `runbook-release.md` (zigbuild → stage → `xtask deploy`,
which smokes itself), then:

```sh
curl -fsS https://andrii.ai/apostille/healthz   # {"status":"ok","version":"0.1.0+<sha>"}
curl -fsS https://andrii.ai/apostille/ | head -c 120
```

Green: deploy exits 0, healthz stamp matches, shell 200.
When the staging box lands (G5), prepend: deploy to staging → `xtask
accept` green → stamp → then prod.

## Drill 2 — atomic rollback

```sh
PREV=<previous good short-sha>
ssh andrii.ai "cd ~/apostille/staging/<live> && ./xtask rollback --to $PREV"
curl -fsS https://andrii.ai/apostille/healthz
curl -fsS https://andrii.ai/apostille/version
```

Green: rollback exits 0, healthz 200 < 30s after the flip,
version reports `$PREV`, plus a revert commit with reason per
repo policy. `xtask rollback` flips the symlink, restarts, and
smokes (auto-reverting on smoke failure). Rollback across a
migration needs the author's explicit call (SQLite:
forward-only, no down-migrations). Redeploy forward afterwards
to restore the live release. Space restarts: systemd allows 5
per 10 min, and every stop currently SIGKILLs after the drain
hangs (P0: QUIC serve has no shutdown path — see prompt-076
log); never run the ×3 back-to-back without watching.

## Drill 3 — backup + restore

Backup is defined in `prod.md` (SQLite backup API — never raw-copy
a WAL-mode DB — + `tar` of `store/`, offsite, 7 daily + 4 weekly).
The drill proves the restore side on a scratch dir (staging box when
it exists, otherwise a scratch path on prod — never over live data):

```sh
# Restore the LATEST nightly backup into a scratch dir, then:
# (scratch ports go in DB rows, not env: config-set net.port /
# net.admin_port to the scratch ports before booting.)
STORE_DIR=$SCRATCH/data ./current/web &
curl -fsS http://127.0.0.1:<scratch-port>/apostille/healthz
cargo xtask packs-validate   # pack/tooling sanity on the restored box
```

Green: restored DB opens, healthz 200, pack validation clean, and
the run is recorded with the backup's date. Secrets restore stays a
separate, author-only step (EAB/keys/KEK escrow per
`secure-config.md`); the drill asserts the operator KNOWS where that
envelope is without printing it.

## Drill log

| Date (UTC) | Drill | Operator | Build | Result + notes |
|---|---|---|---|---|
| 2026-10-29 | 1 ship+smoke ×3 | agent | b5b1c3f, b82c458, b577c10 | green ×3 (deploy 0, stamp, shell 200) |
| 2026-10-29 | 2 rollback ×3 | agent | b577c10↔b82c458 | rollback legs green ×3; run-3 restore hit restart-timeout outage (~4 min, recovered via rate-limit wait + restart) — every stop SIGKILLs (QUIC has no shutdown path, P0 in prompt-076 log) |
| 2026-10-29 | 3 backup+restore | agent | b577c10 | BLOCKED (0/3): no nightly backups exist; mechanism proven once (`.backup` → hal-readable copy → `integrity_check` ok → packs clean); scratch boot needs a KEK-holding operator for port rows; nightly automation + offsite + retention still unwired |
