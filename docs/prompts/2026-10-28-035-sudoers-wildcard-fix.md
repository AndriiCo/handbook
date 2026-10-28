# 2026-10-28-035-sudoers-wildcard-fix

- Prompt (UTC): 2026-10-28T02:30:00Z (approx; operator pasted visudo failure)
- Response (UTC): 2026-10-28T02:48:03Z
- Author: Andrii (operator, as root on enodia)
- Agent: Muse Code

## Prompt

```text
root@enodia:/home/hal/apostille/scripts#
root@enodia:/home/hal/apostille/scripts# /home/hal/apostille/scripts/ops-hal-access.sh
=== 0. writing /etc/sudoers.d/hal-apostille
/etc/sudoers.d/hal-apostille:16:5: syntax error: wildcards are not allowed in command arguments
    /usr/bin/cp /home/hal/apostille/staging/*/apostille.service /etc/systemd/system/apostille.service
    ^~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
visudo: invalid sudoers file
ABORT: sudoers validation failed — removing the drop-in
root@enodia:/home/hal/apostille/scripts#
```

## Response

Root cause: this box's sudo build rejects ALL wildcards in
command arguments, so no `*` grant (cp path, journalctl args,
xtask/sqlite args) can ever install. Redesigned around fixed-path
grants + root-owned validating wrappers in `/usr/local/sbin`:

- `apostille-install-unit` — staged unit source from hal's
  pointer file; content-pinned to
  `ExecStart=/srv/apostille/current/web` as `andrii-ai`
  (installs + daemon-reloads, deletes pointer).
- `apostille-journal` — stdin window strictly `-N min|h|d`
  (empty = last 50); unit fixed so output can't escape units.
- `apostille-config` — 4-line stdin protocol
  (op/key/type/value-to-EOF) for set/get/verify as `andrii-ai`.
- `apostille-sqlite` — SQL on stdin against the fixed DB path.

`ops-hal-access.sh` now also folds in the runbook §0 group
bootstrap (`apostille` group + group-write on `/srv/apostille`
for the atomic `current` swap — the missing piece that kept the
flip root-only) and retires the legacy `apostille-hal` drop-in
(backup at `/root/apostille-hal.sudoers.bak`), with a detector
for the legacy wildcard `apostille-ship` grant. All three
consumer scripts speak the wrapper protocols; `ops-deploy.sh`
drops its root-only gate. `runbook-release.md` §0 now delegates
to the script; stale `restart/status/reload` comments in
`xtask/src/release.rs` corrected (verified: xtask only shells
`systemctl restart`).

Decisions/assumptions: wrappers validate inside (content/window/
protocol); xtask/sqlite stay as `andrii-ai` (as root they'd be a
symlink-to-root hole via hal-controlled `/srv/apostille/current`);
install-unit is deliberately NOT live-probed (it installs for
real — first deploy exercises it); kept scalar `$SUDO`/`$SVC`
style (no shellcheck on this box; `bash -n` is the gate).

Files changed: `docs/ops/scripts/ops-hal-access.sh`,
`ops-deploy.sh`, `ops-acme-enable.sh`, `ops-acme-add-www.sh`,
`docs/ops/runbook-release.md`, `xtask/src/release.rs`.

Verification performed (all observed green):

- `bash -n` on all four scripts — clean.
- Sudoers heredoc wildcard regression check
  (`sed -n ... | grep '\*'` over the `SUDOERS` block) — zero
  wildcards.
- `/tmp/wrapper-validation-test.sh` (throwaway probe, kept in
  /tmp): extracts both validating wrappers, stubs privileged
  tails, feeds good + hostile inputs — 8/8 ALL-PASS (accepts
  `-5 min`/`-2 h`/empty default; rejects `-u shadow`, `-n 50`,
  `--merge`, non-staging pointer prefix, bad pointer chars).
- `cargo xtask ci` (with `RUSTC_WRAPPER=sccache`) — exit 0.
- Pre-commit `secrets-scan` hook — clean (6 files, staged).

Staging: the four scripts must be re-staged to
`/home/hal/apostille/scripts/` before the operator re-runs the
access script (agent to do over ssh as hal, or operator pulls —
see Follow-ups).

## Commits

- `84ab287` — ops: zero-wildcard sudoers via validating wrappers
  (this entry committed separately; see log commit).

## Follow-ups

- Re-stage the four scripts, then operator re-runs
  `bash /home/hal/apostille/scripts/ops-hal-access.sh` as root
  and pastes output; agent then deploys f5d199f as hal.
- Prompt-index gap: entries 025–034 exist as files but were never
  added to `docs/prompts/README.md`; only 035 is indexed below.
  Backfill deferred to the author (entries I didn't author).
