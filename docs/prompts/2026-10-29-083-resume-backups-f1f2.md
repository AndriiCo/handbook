# 083 — Resume: step 2 nightly backups + F1/F2 + step-3 packaging

- Prompt: 2026-10-29 23:30 UTC
- Response: 2026-10-29 23:40 UTC
- Author: Andrii (operator)
- Agent: Muse Code, session umber-hyperion

## Prompt

Resme previous tasks. Use sub-agents.

## Response

Resumed the prompt-081 plan (steps 0–1 done in 082; prior session
died mid-step-2 with a clean tree, so no work was lost). Recovered
the 11-step plan + the backups/drill3 draft + the steps-3–5
preflight from the prior session's logs, then executed step 2 with
three parallel read-only subagents (SEC backup review, step-3
operator package, steps-4–6 refresh); parent wrote all files.

- `xtask backup` + `xtask backup-verify` (new `xtask/src/backup.rs`,
  ~950 lines with 13 tests): SQLite backup API copies of both DBs
  (needs the `rusqlite/backup` feature — the research draft's
  "no new feature flags" was wrong; feature-only, `Cargo.lock`
  untouched) + tar of the file dirs into
  `daily/YYYY-MM-DD/` (manifest, `sha256sums`), verify-before-keep
  (`integrity_check` + `user_version` 11/2 on read-only opens, so
  no WAL residue), unique `.staging.<pid>.<rand>` + fsync +
  whole-dir rename publish, refuse-if-exists reruns, 7-daily /
  4-weekly retention via `release::select_prune`, Sunday promote.
  Reads the live store only (no migrate, no audit write — B5).
- SEC review verdict adopted: B1 (UMask=0077,
  StateDirectoryMode=0700, explicit 0600/0700 + perms test), B2
  (sums-first, `tar -tzf` member reject, no-symlink +
  canonicalize walk), B3 (unique staging, stale sweep, fsync,
  refuse-if-exists), B4 (read-only verify opens + exact-file-set
  test), B5 (no live-DB audit write), B6 (unit exceeds the watch
  baseline: ProtectProc + AF_UNIX AF_NETLINK). Reasoned
  deviations recorded: B6's ProcSubset/PrivateDevices/ProtectClock/
  ProtectHostname skipped (not in the verified `apostille.service`
  baseline; untestable from macOS — follow-up after the first
  green nightly); A3's offsite-confirmation prune gate deferred to
  the offsite pull (nothing sets the marker yet; time-prune bounds
  disk until then). A1/A6 trust boundaries documented in the
  module docs; per-file hashes print to stdout (journald second
  channel, A2).
- F1 fixed: `--store /var/lib/apostille` on watch (both lines),
  mail, and instruct `ExecStart`s; inert `Environment=STORE_DIR`
  removed with a comment; APPROVED units re-checked in full with
  header notes (still APPROVED). F2 fixed: `general.toml` ships
  inside staged `packs/` (runbook scp line; validate/load filter
  `*.json` so the `.toml` is inert), unit repointed at
  `current/packs/general.toml`.
- New `apostille-backup.service`/`.timer` (PROPOSED; 02:30 +
  300s jitter + Persistent). A test pins the unit's hardening
  lines (`backup_unit_keeps_hardening`).
- Step-3/4 operator packaging (committed, not yet staged to the
  server): `ops-loop-first-run.sh` (pinned `apostille-loop`
  wrapper running slug-free KEK-gated `analyze --check` —
  verified at `xtask/src/main.rs:416-418` — plus the sweep pair
  as andrii-ai, residue checks) and `ops-units-reinstall.sh`
  (validates the 8 unit files with F1/F2 gates, installs, enables
  watch + mail only). Both `bash -n` clean (shellcheck N/A —
  not installed); drill-3 re-port keys confirmed
  (`net.port`/`net.admin_port`, both text).
- Forward recon: step-4 VERSION recommendation is the new HEAD
  (077-only would keep the pre-QUIC-fix stop hang for the deploy
  + cert-flip restarts); Day-4 gate evidence checklist refreshed;
  ordered operator/author actions in the child result (session
  log, subagent `forward recon steps 4-6`).
- Rulings (gates / SHP-0 / staging ETA) still unanswered; the 082
  reversible assumptions stand. ACME challenge dir is a separate
  configured path from `acme/` (`config.rs` builder takes both),
  so no tar exclusion was needed.

Files changed:

- `xtask/src/backup.rs` (new: backup + backup-verify + 13 tests).
- `xtask/src/main.rs` (wiring + help).
- `xtask/Cargo.toml` (`rusqlite/backup` feature).
- `docs/ops/apostille-backup.service` / `.timer` (new, PROPOSED).
- `docs/ops/apostille-{watch,mail,instruct}.service` (F1; F2 on
  instruct; re-check notes).
- `docs/ops/runbook-release.md` (F2 stage line).
- `docs/ops/scripts/ops-loop-first-run.sh` /
  `ops-units-reinstall.sh` (new, +x) + `README.md` entries.
- `docs/prompts/2026-10-29-083-resume-backups-f1f2.md` (this entry).

Verification performed:

- `cargo test -p xtask backup`: 15/15 green.
- `cargo clippy -p xtask --all-targets -- -D warnings`: clean.
- Full `cargo xtask ci`: exit 0 (e2e 3/3, tailwind OK).
- `bash -n` on both new scripts: clean; `cargo fmt` clean;
  `git status`: only intended files; `Cargo.lock` untouched.
- Pre-commit `secrets-scan` (hook): clean on both commits.
- Child claims corroborated against primary sources:
  `analyze --check` slug-free path, `packs::validate_dir` /
  `packs-load` `*.json` filters, `ProtectProc` in
  `apostille.service`, ACME challenge/cert dir split.

## Commits

- 4d979d6 — Backups: nightly backup + verify tasks, F1/F2 unit fixes.
- acb678d — Ops: loop-first-run + units-reinstall scripts.
- (This 083 log rides in the commit below.)

## Follow-ups

- Steps 3–11 remain; step 3+ needs operator/author hands.
- Next deploy train at the new HEAD: reinstall `apostille.service`
  (QUIC budget) + 4 timer units + daemon-reload; stage scripts to
  `/home/hal/apostille/scripts/` and units to `units-staging/`;
  run `ops-loop-first-run.sh` then `ops-units-reinstall.sh`.
- Backup timer enable decision + offsite pull (author) still open;
  drill 3 goes green ×3 only after install + first nightlies.
- B6 extra hardening (ProcSubset et al.) + A3 prune gate land with
  the offsite/first-green follow-ups, not before.
- Rulings (gates / SHP-0 / staging ETA) still unanswered.
