# Root-action scripts (AGENTS.md §7)

One script per operator action that needs privilege. The agent
stages these to `/home/hal/apostille/scripts/` on the server.
Every script runs as hal (pinned sudo grants installed once by
`ops-hal-access.sh`) or root — `bash <script>`, no placeholders.

Rules for every script in this dir:

- Zero placeholders: derive values (VERSION/SHA from staging,
  build numbers from the DB) or prompt for secrets (hidden entry,
  never on the cmdline or in history).
- Fail fast with `ABORT: <reason>`; never plow past a failure.
- End by printing the evidence the agent needs pasted back.
- `bash -n` clean (and shellcheck-clean when available) before
  every commit.

Scripts:

- `ops-hal-access.sh` — ONE-TIME root setup: installs the pinned
  sudoers drop-in so hal deploys autonomously afterwards.
- `ops-deploy.sh <staging-dir> [--skip-accept]` — install the
  staged unit file (diff shown first), run the accept gate +
  `xtask deploy`, verify the health stamp. `--skip-accept` is an
  explicit, announced escape hatch for content/schema-free
  releases when prod has no build row (agent confirms per
  release; never the default).
- `ops-acme-enable.sh` — guard on the fixed binary, flip
  `acme.enabled`, unpin the file cert, restart, poll health,
  print issuance evidence.
- `ops-acme-add-www.sh` — guard on fixed binary + ACME mode +
  www DNS, set the 3-name `acme.domains`, drop the persisted
  pair (account kept), restart, verify 3 SANs + www serving.
- `ops-staff-access.sh <cn>` — root: verify the newest live
  client cert for a CN, export it as PKCS#12 for browser
  install, print staff user_ids for passkey login.
- `ops-loop-first-run.sh` — root: step-3 unpark (pinned
  `apostille-loop` wrapper + grant, sweep pair, KEK-gated
  `analyze --check` via the hal path, residue checks).
- `ops-units-reinstall.sh` — root: validate + install the 8
  watch/mail/instruct/backup unit files from
  `units-staging/` (F1/F2 gates), enable watch + mail only.
