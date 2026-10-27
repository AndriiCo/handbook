# Deploy drill ×3 (roadmap item 11)

Three drills, each run to green THREE times before the Day-7
signoff. Every run is logged in the table at the bottom with date,
operator, build sha, and result. A failed run stops the count for
that drill (fix, then restart its ×3).

## Drill 1 — ship + smoke (prod-direct while G5 is relaxed)

```sh
VERSION=$(git rev-parse --short HEAD)
sudo /usr/local/sbin/apostille-ship $VERSION   # smokes itself; see deploy.md
curl -fsS https://andrii.ai/apostille/healthz
curl -fsS https://andrii.ai/apostille/ | head -c 120
```

Green: ship script exits 0, healthz `{"status":"ok"}`, shell 200.
When the staging box lands (G5), prepend: deploy to staging → `xtask
accept` green → stamp → then prod.

## Drill 2 — atomic rollback

```sh
PREV=<previous good short-sha>
ssh andrii.ai "ln -sfn /srv/apostille/releases/$PREV \
  /srv/apostille/current && sudo systemctl restart apostille"
curl -fsS https://andrii.ai/apostille/healthz
curl -fsS https://andrii.ai/apostille/version
```

Green: healthz 200 < 30s after the flip, version reports `$PREV`,
plus a revert commit with reason per repo policy. Rollback across a
migration needs the author's explicit call (SQLite: forward-only, no
down-migrations). Keep the last 3 releases; delete older.

## Drill 3 — backup + restore

Backup is defined in `prod.md` (SQLite backup API — never raw-copy
a WAL-mode DB — + `tar` of `store/`, offsite, 7 daily + 4 weekly).
The drill proves the restore side on a scratch dir (staging box when
it exists, otherwise a scratch path on prod — never over live data):

```sh
# Restore the LATEST nightly backup into a scratch dir, then:
STORE_DIR=$SCRATCH/data ./current/web &  # port 0 in tests; scratch port here
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
| (none yet) | — | — | — | — |
