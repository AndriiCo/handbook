# Ops scripts

The release path and its checks are described in
[docs/ops/deploy.md](../../docs/ops/deploy.md). Rebuilding a host from
nothing is `cargo xtask setup` plus `cargo xtask provision`.

| Script | Runs on | As | Purpose |
|---|---|---|---|
| `ship-release.sh <sha>` | Mac | you | Stamp and rails checks, prod-before scoreboard, cross-build in a throwaway worktree, refresh `licenses.html` there if stale (`xtask licenses --no-commit`), stage to production. Never deploys |
| `ops-deploy.sh <stage-dir> --skip-accept` | production | hal | Install the unit, `xtask deploy` (swap, restart, smoke, auto-revert), prune |
| `staging-accept.sh --sha <sha>` | Mac | you | Staging scoreboard plus a live demo take; prints the staging-green verdict |
| `scoreboard.mjs` | Mac | you | Release checks against a base URL (`--base`, `--out`, `--only`) |
| `ops-gp2-proposals.sh` | production | hal | Fetch and draft proposals for the hero authorities |
| `ops-a2-record.sh` | production | hal | Record the photo-precheck verdicts for the public sample (`xtask a2-record`) |
| `ops-a5-eval.sh` | production | hal | Re-run the gold-set eval behind the home page's quality numbers |

`wrappers/` holds the root-owned `/usr/local/sbin/apostille-*` wrappers
and hal's zero-wildcard sudoers drop-in. `xtask provision` embeds them
at build time and installs them (checked by `visudo`); edit them here,
never on a server. `ops/rehearsal/Dockerfile` is the blank server the
Docker rehearsal provisions.
