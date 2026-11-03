# Ops scripts

The release path and its checks are described in
[docs/ops/deploy.md](../../docs/ops/deploy.md). Rebuilding a host from
nothing is `cargo xtask setup` plus `cargo xtask provision`.

| Script | Runs on | As | Purpose |
|---|---|---|---|
| `ship-release.sh <sha>` | Mac | you | Stamp and rails checks, prod-before scoreboard, cross-build in a throwaway worktree, stage to production. Never deploys |
| `ops-deploy.sh <stage-dir> --skip-accept` | production | hal | Install the unit, `xtask deploy` (swap, restart, smoke, auto-revert), prune |
| `staging-accept.sh --sha <sha>` | Mac | you | Staging scoreboard plus a live demo take; prints the staging-green verdict |
| `scoreboard.mjs` | Mac | you | Release checks against a base URL (`--base`, `--out`, `--only`) |
| `ops-gp2-proposals.sh` | production | hal | Fetch and draft proposals for the hero authorities |
| `ops-a2-record.sh`, `a2-record.py` | production | hal | Record the photo-precheck verdicts for the public sample |
| `ops-a5-eval.sh` | production | hal | Re-run the gold-set eval behind the home page's quality numbers |

One-time provisioning scripts kept until `xtask provision` has rebuilt a
host end to end (they are superseded by it): `ops-hal-access.sh`,
`ops-rootless-access.sh`, `ops-units-reinstall.sh`,
`ops-backup-enable.sh`, `ops-acme-enable.sh`, `ops-acme-add-www.sh`,
`ops-packs-seed.sh`, `ops-hcch-seed.sh`, `ops-loop-first-run.sh`,
`ops-drill3-run.sh`.
