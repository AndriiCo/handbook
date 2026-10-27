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

Flip unblocked mid-task (author granted the root-dir write):
prod flipped to `f2848a8` single-443 by hal (restart via pinned
sudoers) — public 200 + stamp, 444 closed, admin Host routing
401/SPA 200. Remaining: server certbot purge (root apt) +
`acme.enabled` (needs author EAB mint + root DB writes).

## Commits

- Purge commit + this log update (see git log).

## Follow-ups

- Verify admin SAN on the first in-binary issuance; then admin
  UI works in browsers and config entry unblocks the project.
