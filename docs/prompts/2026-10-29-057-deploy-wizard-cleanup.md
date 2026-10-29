# Prompt 057 — deploy the wizard + remove superseded playbooks

- Prompt timestamp (UTC): 2026-10-29T00:26:00Z
- Response timestamp (UTC): 2026-10-29T00:44:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

Was it deployed? How to start a configuration process? Can you
remove outdated playbooks for configurations already handled by
the config wizard?

## Response

No, it was not deployed — now it is (`30672c9` live,
`0.1.0+30672c9`), and the superseded playbook is removed
(`30672c9` also carries the removal; log `36f1961` predates).

Removal (committed in `30672c9`, docs-only):

- DELETED `docs/ops/config-values.md` (570 lines): every one
  of its retrieval sections (Google/Apple/Meta click-paths,
  mailer) is now a wizard guide in `setup-guides.js`; its
  staff section was a pointer. Git history keeps the text.
- KEPT `docs/auth-playbook.md`: six `customer-auth.md`
  references ground flow deviations and OUT-of-slice items
  (RISC/OPEN-7, Apple events/OPEN-11, Meta deletion/OPEN-13)
  on its §§2–4 — deleting them would orphan the spec. Added
  a header pointer: retrieval moved to the wizard, flow +
  linking + callback rules stay normative.
- `docs/ops/staff-enrolment.md`: backlink fixed; notes the
  first-boot exception (setup key instead of cert + invite,
  then burn — the runbook covers the second staffer onward).
- `secure-config.md` §5 (KEK provisioning, stays manual):
  gained step 5, the mint-then-wizard handoff.

Deploy (runbook §1–3 + `ops-deploy.sh`):

- zigbuild 6m39s + both web bundles (wizard chunk verified
  in `build-admin` before staging); staged + `SHA` + unit
  (no-op diff) via hal; no `MIGRATION_NOTES` (no schema
  change — setup rides the `config`/`audit` tables).
- `--skip-accept` again, agent-confirmed: prod `build`
  table still empty (no `build-handbook` stamp ever), so
  the gate stays vacuous; announced in the output.
- `deployed 30672c9 (previous: dc400a9, pruned: bb54d04)`;
  stamp `0.1.0+30672c9`; manifest schema 11 / rollback-safe.

Verification on prod:

- `/healthz` → `0.1.0+30672c9`; `/admin/config` → 200;
  `setup/status` → `{"setup_required":false}` (prod is
  configured with staff — the wizard correctly dormant).

How to start a configuration process (fresh server):

1. Server shell (root): KEK enrol per §5, then
   `xtask setup-key-mint --store /var/lib/apostille`
   (prints the key once).
2. Browser: open `https://<admin-host>/admin/config`,
   paste the key, work the 8 guided steps (27 required
   keys), then enrol the first passkey — the key burns
   and the normal config list takes over.
3. On THIS prod (already configured): nothing to start —
   `/admin/config` is already the full staff list.

## Verification

- Prod probes above (healthz, setup/status, config 200,
  manifest) all observed post-deploy.
- No code changed after the prompt-056 green CI
  (587/0 Rust, 95/95 node); this turn is docs + deploy.
- Staged-hook secrets-scan clean.

## Commits

- `30672c9` Remove config playbooks superseded by the
  setup wizard (+ ships the wizard binaries)
- (this log entry commits separately, per convention)

## Follow-ups

- First `build-handbook` stamp would re-arm the accept
  gate (still skipped per-release-confirmed).
- Staging ceremony drill with a real authenticator.
- §11 OQ-1..5 open; OPEN-32 (`docs/specs/p2-forms-labels.md`
  §9) awaiting approval.
