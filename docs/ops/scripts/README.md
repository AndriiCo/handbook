# Root-action scripts (AGENTS.md §7)

One script per operator action that needs root. The agent stages
these to `~/apostille/scripts/` on the server; the operator reviews
each file, then runs it with `sudo bash <script>`.

Rules for every script in this dir:

- Zero placeholders: derive values (VERSION/SHA from staging,
  build numbers from the DB) or prompt for secrets (hidden entry,
  never on the cmdline or in history).
- Fail fast with `ABORT: <reason>`; never plow past a failure.
- End by printing the evidence the agent needs pasted back.
- `bash -n` clean (and shellcheck-clean when available) before
  every commit.

Scripts:

- `ops-deploy.sh <staging-dir>` — install the staged unit file
  (diff shown first), run the accept gate + `xtask deploy`,
  verify the health stamp.
- `ops-acme-enable.sh` — guard on the fixed binary, flip
  `acme.enabled`, unpin the file cert, restart, poll health,
  print issuance evidence.
