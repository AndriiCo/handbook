# Ops scripts

The release path and its checks are described in
[docs/ops/deploy.md](../../docs/ops/deploy.md). Shipping a release and
rebuilding a host from nothing are both `cargo xtask setup` (stamp,
migration rail, build, `xtask provision`, seal, stage, deploy). Setup
stages `ops-deploy.sh` and the three `ops-*` operator scripts into
`~/apostille/scripts` on every run.

| Script | Runs on | As | Purpose |
|---|---|---|---|
| `ops-deploy.sh <stage-dir> --skip-accept` | production | hal | Install the unit, `xtask deploy` (swap, restart, smoke, auto-revert), prune |
| `staging-accept.sh --sha <sha>` | Mac | you | Staging scoreboard plus a live demo take; prints the staging-green verdict |
| `scoreboard.mjs` | Mac | you | Release checks against a base URL (`--base`, `--out`, `--only`) |
| `ops-gp2-proposals.sh` | production | hal | Fetch and draft proposals for the hero authorities |
| `ops-a2-record.sh` | production | hal | Record the photo-precheck verdicts for the public sample (`xtask a2-record`) |
| `ops-a5-eval.sh` | production | hal | Re-run the gold-set eval behind the home page's quality numbers |

`systemd/` holds the units every release stages under `units/`:
`apostille.service` (the web server) and the `backup`, `watch`,
`mail`, `instruct` and `retention` service/timer pairs. `xtask
provision` writes them to `/etc/systemd/system` and enables the timers
the setup file's target lists; `ops-deploy.sh` reinstalls
`apostille.service` on each deploy. Edit them here, never on a server.

`wrappers/` holds the root-owned `/usr/local/sbin/apostille-*` wrappers
and hal's zero-wildcard sudoers drop-in. `xtask provision` embeds them
at build time and installs them (checked by `visudo`); edit them here,
never on a server. `ops/rehearsal/Dockerfile` is the blank server the
Docker rehearsal provisions.
