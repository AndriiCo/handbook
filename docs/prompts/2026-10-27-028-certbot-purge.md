# 028 — Purge certbot, enable in-binary ACME

- Prompt timestamp (UTC): 2026-10-27
- Response timestamp (UTC): 2026-10-27
- Author: Andrii Tereshchenko
- Agent/model: Muse Code, powered by Meta Muse Spark

## Prompt

I need admin UI to be working in production and let me enter config files and move forward with a project. Remove all mentions of certbot. Remove it from server and enable acme.enabled.

## Response

Repo purge done (commit below): zero live `certbot` /
`letsencrypt` / `cert-deploy` references outside immutable
prompt-log history. Deleted `docs/ops/apostille-cert-deploy.sh`;
rewrote `deploy.md` root-install step 2 for the in-binary flow;
`acme.md` / `parity.md` / `prod.md` / `threat-model.md` /
`secure-config.md` / `single-443-quic.md` / `web-stack.md` + one
code comment updated. 365 tests green.

Server-side removal + `acme.enabled` need root (hal lacks apt
+ DB-write + flip rights) — exact operator commands issued in
chat. Blockers: EAB mint (author gcloud), root flip to the
single-443 release, certbot purge.

## Commits

- (single commit below)

## Follow-ups

- Verify admin SAN on the first in-binary issuance; then admin
  UI works in browsers and config entry unblocks the project.
