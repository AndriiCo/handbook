# Deploy and operations

The one manual for building, staging, shipping, rolling back and
watching the Apostille Handbook. Hosts are named by SSH alias only:
the repo never records a username, IP address, DNS name of a private
box, or key.

| Host | Alias | Role |
|---|---|---|
| This Mac | — | Control: commits, cross-builds (`cargo zigbuild`), runs the scoreboard |
| Build and staging box | `plant` | Build gate (`cargo xtask ci`), staging stack on 127.0.0.1:18318 (public) / 18319 (admin). User-level only, never root |
| Production | `andrii.ai` (as `hal`) | Ubuntu, systemd unit `apostille`, releases in `/srv/apostille/releases`, store `/var/lib/apostille`, KEK `/etc/apostille/handbook-kek` |

## Rules that always hold

- No secret values in the repo, fixtures, screenshots, videos, logs
  or prompts. `cargo xtask secrets-scan` (pre-commit hook and `ci`)
  stays on.
- Fixtures, screenshots and demo documents are synthetic: no real
  customer names, emails, addresses or document images. Only
  synthetic or public text goes to a `-contributor` model.
- Admin code never ships in the public build (`npm --prefix web run
  check:imports`). Admin auth gates (host guard, `admin.allow_ips`,
  staff passkey session, CSRF) are never weakened. A public AI
  surface is rate-limited and cost-capped.
- Production config, secrets and the database are never edited by
  hand. Staging data never goes to production; facts reach
  production only through staff approvals in admin `/review`.
- History is never rewritten; roll back code with a revert commit.

## Release path

1. **Stamp.** `ops/plant-pipeline/submit.sh <sha>` queues a committed
   SHA on plant; `ops/plant-pipeline/status.sh <run>` polls it. The
   gate runs `cargo xtask ci` (fmt, clippy `-D warnings`, tests,
   secrets-scan, web gates). Green writes
   `acceptance/<sha12>.json` with `ci_green: true`. No stamp, no ship;
   no flag skips this gate.
2. **Exact SHA to plant.** `git push plant:staging/handbook
   "${SHA}:refs/heads/xfer-<name>"` (quote `${SHA}` in zsh, or `:r`
   is read as a modifier), then on plant `git checkout -f <sha>`.
   Plant has no GitHub credentials, so it never fetches.
3. **Staging build** on plant: `cargo build --release -p web -p xtask`,
   `npm --prefix web ci`, `npm --prefix web run build` and
   `build:admin`.
4. **Staging restart** by verified PID: find the `web` process whose
   cwd is `~/staging/handbook` and whose command is
   `./target/release/web`, kill it, start the new one with
   `setsid -f` (without `-f` the ssh hangs) with `STORE_DIR`,
   `SPA_DIR`, `STATIC_DIR` and `CREDENTIALS_DIRECTORY` (0600 KEK
   copy). Never `pkill -f` with a pattern that matches your own
   command.
5. **Staging acceptance** through the tunnel
   (`ssh -N -L 18318:127.0.0.1:18318 -L 18319:127.0.0.1:18319 plant`):
   `xtask/ops/staging-accept.sh --sha <full-sha>` runs the scoreboard
   against `http://127.0.0.1:18318/apostille` and a live demo take.
   **Staging-green** = every row that passes on production passes on
   staging, no staging row gains an `HTTP`, `->`, `visible "`,
   `error:` or `timed out` problem or more console errors than
   production's row, and the demo take is 3/3. A run that is not
   staging-green blocks the deploy and nothing else.
6. **Ship** (Mac): `xtask/ops/ship-release.sh <sha>` checks the
   stamp, the schema/migration/unit-file rails and the root-process
   probe, runs the prod-before scoreboard, cross-builds that exact
   SHA in a throwaway worktree, and stages it to
   `~/apostille/staging/<sha7>` on production. It never deploys.
7. **Deploy** (as `hal`, right after a fresh root-process probe):
   `bash ~/apostille/scripts/ops-deploy.sh ~/apostille/staging/<sha7>
   --skip-accept`. It installs the unit through the root-owned
   wrapper, runs `xtask deploy` (atomic `current` swap, restart,
   `/healthz` smoke naming the SHA, automatic revert on a failed
   smoke) and prunes old releases.
8. **Verify:** `node xtask/ops/scoreboard.mjs --out
   web/shots/prod-after.json` and compare with
   `web/shots/prod-before.json`.

Root-process probe (must print nothing, checked again right before
step 7):

```sh
ssh andrii.ai 'pgrep -u 0 -c . >/dev/null || echo ROOT-PROCS-HIDDEN; pgrep -af "[a]postille/scripts/"'
```

### Rails

- Only a SHA with a green plant stamp and a staging-green run of the
  same SHA ships. It is built from a throwaway worktree, never from
  a dirty checkout (`ship-release.sh` does this).
- A SHA that changes `SCHEMA_VERSION` or `P2_SCHEMA_VERSION`
  (`crates/handbook-store/src/db.rs`), edits
  `crates/handbook-store/migrations/`, or adds `ALTER`, `DROP` or
  `RENAME` SQL is not a deploy but an operator step: no older binary
  opens a migrated database, so neither the smoke revert nor a
  rollback undoes it. Add tables with `CREATE TABLE IF NOT EXISTS`.
- A SHA whose `docs/ops/apostille.service` differs from the
  installed unit is an operator step too (a rollback does not
  restore the unit).
- Never pass `--force`, `--backup-note`, `--skip-packs`,
  `--no-restart`, `--root` or `--smoke-url` to `xtask` on production.
- During a freeze (set by the operator), no deploy without the
  operator's explicit order; a rollback only on the operator's word
  or when `/healthz` fails or is not `ok` on three tries over 5
  minutes.
- Every deploy report names the version, the previous version, both
  scoreboards and any `--skip-accept` (production has never had the
  accept gate's `build` row).

### Rollback

Roll back at once, only to the release that was live before the most
recent deploy (the `previous:` in the `ops-deploy.sh` output), with
that release's own xtask:

```sh
ssh andrii.ai '/srv/apostille/releases/<previous>/xtask rollback --to <previous>'
```

Triggers: a scoreboard row that passed now fails; a row gains an
`HTTP`, `->`, `visible "`, `overflow`, `error:` or `timed out`
problem or more console errors; `/healthz` is not `ok`; or it does
not name `0.1.0+<deployed sha7>`. A row that fails from time or data
(the eval window, approvals) is never a rollback. A rollback the
guard refuses (across a hard migration, or away from a
`rollback_safe = false` release) is the operator's call. Fix forward
on staging afterwards.

## Plant

- Never root, never ask for root; home directory only.
- No production credentials on plant, no SSH from plant to
  production, no production database copies.
- The gate is asynchronous: `submit.sh` returns in seconds; never
  block on it. The pipeline keeps the newest 3 runs
  (`ops/plant-pipeline/README.md` is its runbook).
- Live model calls on staging use the staging-only key in
  `~/.config/apostille/meta-staging.env` (0600, sourced in the remote
  shell only), on synthetic documents or public pages only.
- Staging data may be reset or deleted at any time:
  `rm -rf ~/staging/{store,kek,creds}` then re-run init, enrol,
  config and the boot of step 4.

## Production pre-flight (as hal, no root shell)

```sh
touch /srv/apostille/releases/.w && rm /srv/apostille/releases/.w && echo WRITE-OK
touch /srv/apostille/.w && rm /srv/apostille/.w && echo FLIP-OK
sudo -n systemctl status apostille >/dev/null && echo SUDO-OK
sudo -n -l | grep -q "stop apostille" && { echo "REFUSING: stop verb granted"; exit 1; } || echo STOP-ABSENT
echo 'SELECT 1;' | sudo -n -u andrii-ai /usr/local/sbin/apostille-sqlite >/dev/null && echo SQL-OK
printf '\n' | sudo -n /usr/local/sbin/apostille-journal >/dev/null && echo JOURNAL-OK
```

## Watch

- Liveness: `GET /apostille/healthz` returns 200 and
  `{"status":"ok","version":"0.1.0+<sha7>"}`; alert on 3 failures in
  a row.
- Readiness: `GET /apostille/readyz` reports `database`,
  `tls_certificate`, `kek` and `timers` (200 ready, 503 naming the
  failed checks). Plain-HTTP boots (staging, `xtask up`) have no
  serving certificate and never report ready.
- Certificates renew in the binary (ACME, ARI); check weekly that
  `notAfter` is more than 21 days away (`docs/ops/acme.md`).
- Backups: nightly SQLite backup API plus a tar of the store
  (`apostille-backup.timer`, `xtask backup`), 7 daily + 4 weekly,
  verified with `xtask backup-verify`. Secrets are backed up
  separately by the operator.
- Posture: SSH keys only; firewall TCP 80/443 and UDP 443; unit
  hardening (`NoNewPrivileges`, `ProtectSystem=strict`); suspected
  breach: rotate keys, keep the audit log, then investigate.

## Drills (quarterly)

- **Ship and smoke:** the release path above; green when the deploy
  exits 0, `/healthz` names the SHA and the shell answers 200.
- **Rollback:** `xtask rollback --to <previous>` from that release's
  directory; green when `/healthz` is 200 within 30 s and names the
  previous version. Space restarts: systemd allows 12 starts per
  10 minutes.
- **Backup and restore:** restore the latest nightly backup into a
  scratch directory (never over live data), set scratch ports with
  `config-set net.port / net.admin_port`, boot
  `STORE_DIR=<scratch> ./current/web`, check `/healthz` and
  `cargo xtask packs-validate`.
- **Setup-key re-mint:** on a scratch store,
  `cargo xtask setup-key-mint --store <scratch> --kek-file <kek>`;
  a store with live staff passkeys refuses without `--force`, and a
  forced re-mint invalidates the old key.
