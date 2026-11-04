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
  The operator step, on the operator's order only: as `hal`, back up
  the store online right before shipping
  (`echo ".backup /var/lib/apostille/pre-v<N>-<UTC>.sqlite" | sudo -n
  -u andrii-ai /usr/local/sbin/apostille-sqlite`), then
  `ship-release.sh <sha> --migration-ack <that path>` (refused unless
  the file is non-empty and under 2 hours old) and deploy as usual.
  Rollback is restoring that file with the service stopped; the
  migration must also be listed in `HARD_MIGRATIONS` (release.rs).
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

## Rebuild from scratch

One encrypted file holds every setting and secret for every target:
`private/config.yaml` (sops + age; `private/` is git-ignored, kept out
of Docker contexts and out of agent reads). Plaintext never touches
disk: `cargo xtask setup` decrypts into memory and seals each target's
values on the host with `config-import` in one transaction. Edit it in
the wizard or by hand with `sops private/config.yaml`.

```sh
cargo xtask setup                      # no file: create it, open the wizard
cargo xtask setup wizard               # edit, check keys, run, licences
cargo xtask setup check --target staging
cargo xtask setup --target docker --dry-run
cargo xtask setup --target docker      # rehearsal on this Mac
```

Targets (the template has all four):

| Target | Kind | What a run does |
|---|---|---|
| `local` | this machine | `xtask up` on loopback with the file's values imported; with `runtime: docker`, the judge image instead (`docker compose up --build --detach`, a fresh store, 127.0.0.1:18318/18319) |
| `docker` | rehearsal | `ubuntu:26.04` + systemd in Docker, playing a blank server |
| `staging` | `plant` | push the SHA, build, harden-check, seal, restart by PID |
| `production` | server | stamp, probe, DNS, build, provision, seal, deploy, ACME, staff, scoreboard |

Phases run in order and stop at the first failure; `--from <phase>`
resumes. Each step's commands are printed by `--dry-run`; the
production plan is pinned by a golden test
(`xtask/src/setup/testdata/production-plan.txt`). Production runs only
with `--operator-order "<who, when, what>"`, recorded in
`private/setup-log.jsonl`.

**System map.** The wizard opens on a UML deployment diagram of the
whole system: the front end (public and admin sites), the workers and
staging, each **Local** (the `local` target, **Docker** or **Native**:
its `runtime`) or **Server** (the `server` target for the front end, the
`plant` target for workers and staging). The choice is kept in the
file's `map` section; unset, the front end is the server target (else
local), staging the plant target, and workers run on plant beside a
server or with a local front end. Nodes show the HTTPS server name and
links to the public site, the admin console and its Work queue, the
staging URL (through `ssh -L`), and the box probe. The Meta Model API is
a `«use»` dependency labelled with the model ids, with a **Test** button
(`GET /v1/models` from the wizard; the key never reaches the page);
Hugging Face weights are a `«deploy»` onto the workers
(`map.weights_repo` → `<worker>:/srv/scratch/apostille/models`); Gmail,
Telnyx, FedEx, USPS and Stripe show their console and, when the file
sets it, their mode (Stripe from the key's prefix, FedEx from
`labels.live`, USPS from `usps.live`).

**Box probe.** A plant or server target's tab (and its map node) has **Probe over SSH**:
it logs in with your keys only (`BatchMode=yes`,
`StrictHostKeyChecking=yes`, 10 s to connect) and reads the host
name, user, OS, kernel and CPU (`uname -srm`), CPU count, memory, free
disk in the home directory, GPUs (`nvidia-smi -L`) and the Docker
version, and caches them in `targets.<name>.probe`. It never accepts a
host key: for a new host, run `ssh <host>` once in a terminal first.

**Builds.** Each target's CPU is detected (`uname -m` over SSH, or the
Docker daemon's architecture). The same triple as this machine builds
with `cargo build`, anything else with `cargo zigbuild` (zig 0.17).
sccache wraps rustc when installed. C dependencies get
`-fstack-protector-strong -fstack-clash-protection -D_FORTIFY_SOURCE=2`
and Rust links full RELRO; `cargo xtask harden-check <elf>` refuses a
binary without PIE, a non-executable stack, W^X segments, BIND_NOW and
`__stack_chk_fail`. A release is built from a throwaway worktree of an
exact commit and cached in `target/setup/<sha7>-<triple>/`.

**Provision** (`xtask provision`, root, idempotent, `--check` shows
changes): users `hal` and `andrii-ai`, group `apostille`; the release
and store directories; the KEK (created once, never overwritten); the
root-owned wrappers and one zero-wildcard sudoers drop-in (checked by
`visudo`); the systemd units; `ufw` on servers (skipped in a
container); `/srv/apostille/public-host`, which the deploy smoke reads;
and on a first install the release, the sealed config, the packs and
the service. Later releases never need root.

**Docker rehearsal.** `setup --target docker` builds for the daemon's
CPU, starts `handbook-rehearsal` (privileged, cgroupns host; SSH on
127.0.0.1:2222, the container's 443 and 80 on 127.0.0.1:8443 and :8080,
set by `targets.docker.publish`), and runs the same provision, seal,
deploy, verify and staff steps over SSH. Its SSH key, known hosts and a
throwaway TLS CA live in `private/rehearsal/`. The hosts are
`https://rehearsal.localhost:8443/apostille` and
`https://admin.rehearsal.localhost:8443/` (both resolve to loopback;
`admin.allow_ips` is Docker's bridge range, since only this Mac can
reach the ports). To try passkeys there, trust
`private/rehearsal/ca.pem` in your browser yourself and paste the setup
key the run prints into admin `/config`. `setup wipe --target docker` deletes the
container; the next run rebuilds it from nothing. The rehearsal runs
on the Docker host's own CPU: systemd does not boot under user-mode
emulation (sshd, dbus and `tmp.mount` fail), so on an Apple-silicon Mac
it rehearses arm64, and x86_64 is proven by the hardened x86_64
cross-build plus staging on plant. On an x86_64 Docker host the same
command rehearses x86_64.

**Brand files.** The wizard's brand step lists Apple, Google, Meta,
the FIDO passkey icon and the Vogun face: the owner's terms, what they
allow, where to get the file. You read the terms in your own browser
and tick acceptance; the file records your name, the time, the terms
version and each file's sha256. Then **Fetch** (enabled only once you
have ticked) downloads the file from its source URL (HTTPS only,
20 MB, 30 s), takes the named entry out of a zip (`unzip`) or an Apple
disk image (read in memory, no mount), refuses an SVG with a script, an
`on…=` handler, an outside link or a `<foreignObject>` holding anything
but empty styled divs (Figma's gradient paint), writes it to
`private/brand/` and adds its hash to your acceptance. Uploads meet the
same check. Sources live in `brand.sources` (per file, overridable).
Defaults (downloaded and opened 2026-11-04):

| File | Source | Entry |
|---|---|---|
| `google.svg` | `https://developers.google.com/static/identity/images/signin-assets.zip` | `Theme=Neutral, Show text=No, Shape=Square, Platform=Android+Web.svg` |
| `apple.svg` | `https://devimages-cdn.apple.com/design/resources/download/Logo-Sign-in-with-Apple.dmg` | `Logo - SIWA - Left-aligned - Black - Medium.svg` |
| `passkey.svg` | `https://fidoalliance.org/wp-content/uploads/2024/01/FIDO_Passkey_mark_A.zip` | `FIDO_Passkey_mark_A_black.svg` |

FIDO's agreement is formed by its download form
(<https://fidoalliance.org/passkey-download/>): complete it before you
tick acceptance. Meta (guidelines to accept on its site) and Vogun
(purchased) stay manual: download in your browser, drop the file in,
and tick acceptance again. Builds ship only files whose hash matches a
recorded acceptance (from `private/brand/`).

**Rebuilding production** (only on the operator's order; a freeze
blocks it):

1. Take a backup off the server first (the nightly in
   `/var/lib/apostille-backups/`, plus the KEK if the old store must
   stay readable).
2. Fill `targets.production` in the wizard: `ssh` (hal) and
   `ssh_admin` (an account with passwordless sudo, used only to
   provision), `bind`/`bind6`, the DNS names, timers, and every secret.
   Mint a fresh single-use ACME EAB key. Run `setup check --target
   production` until every row you need is `ok`.
3. Point DNS at the new server and wait until the `dns` check passes.
4. Get a plant stamp for the SHA, then
   `cargo xtask setup --target production --operator-order "<text>"`.
   The first staff setup key prints once at the end; open admin
   `/config` with it and enrol a passkey.

## Plant

- Never root, never ask for root. Plant may write its home directory,
  `/srv/scratch/apostille/` (models, fetch and Meta-call caches, worker
  scratch: `work/`, `meta-cache/`, `fetch-cache/`, `models/`) and
  `/srv/backup/apostille/` (plant-local backups and rollbacks only;
  production backups stay on andrii.ai). Operator decision 2026-11-03.
- No production credentials on plant, no SSH from plant to
  production, no production database copies. The one link is
  outbound: plant workers (`cargo xtask worker`) dial
  `wss://andrii.ai/apostille/api/v1/worker/ws` with the sealed
  `worker.token` and check queued jobs out of production's
  `work.sqlite` (operator decision 2026-11-03; spec
  `docs/specs/work-queue.md`, runbook `ops/plant-pipeline/README.md`
  § Work-queue worker). Production never connects to plant.
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
