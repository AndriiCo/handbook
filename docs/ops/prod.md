# Production ops: monitoring, logging, backup (plan)

Single Ubuntu box, systemd + one `web` process (Rust terminates
TLS itself; no proxy). Boring
on purpose; every item below is checkable by one command.

## Logging

- App logs: structured lines to stdout
  (`ts lvl msg=<...> req=<id> route=<...> status=<...>
  ms=<...>`), collected by journald
  (`journalctl -u apostille`). Request ids are random hex,
  not customer data.
- Access log: the app's own `request` spans on stdout
  (collected by journald): `id method uri` in, `status
  latency_ms` out, same ID echoed in `x-request-id`. No query
  strings with tokens are ever logged: the app never puts
  secrets in URLs.
- **PII rule (hard):** no secrets, tokens, document contents,
  emails, or addresses in ANY log. Audit rows record THAT
  something happened (actor, action, entity id), never the
  payload. Violations are P1 bugs. The `secrets-scan` hook
  covers the repo; a monthly `grep` over rotated logs covers
  the server (add to the runbook, not to code).
- Log levels: `info` default; `debug` only via env on
  staging, never prod (debug paths may print request bodies).

## Monitoring (dead-simple, no vendor)

- Liveness: systemd `Restart=on-failure` + a 60s cron-style
  check (author's existing monitor or a second box later):
  `GET /apostille/healthz` → 200 + `{"status":"ok"}`.
  Alert on 3 consecutive failures.
- Cert expiry: in-binary ACME renews automatically (ARI);
  a weekly check asserts `notAfter - now > 21 days` for
  andrii.ai (one openssl line in the runbook).
- Disk: alert at 80% (SQLite + store + logs share one disk).
  Biggest knob: `store/blob` growth and journald vacuum.
- Error budget: `journalctl -u apostille -p err --since
  "1h" | wc -l` reviewed daily until a dashboard exists.
  Any 500 spike → check, then rollback per `deploy.md`.

## Backup / DR

- Nightly: stop nothing (SQLite backup API or
  `sqlite3 .backup` — never raw-copy a WAL-mode DB) +
  `tar` of `store/` + ship offsite (author's storage;
  encrypted at rest). Keep 7 daily + 4 weekly.
- Quarterly: restore drill onto staging (proves the
  backups); record result in `docs/ops/` with date.
- Secrets (EAB values, API keys, TPM-sealed config) are
  backed up SEPARATELY from data, encrypted, author-only
  access. Losing the TPM seal = re-issuing everything
  (documented in `secure-config.md`).

## Security posture (prod box)

- SSH: key-only, non-root service user, unattended-upgrades
  on. Firewall: TCP 80/443 from anywhere (plus UDP 443 once
  `quic.enabled` flips — QUIC serves on the same port), SSH
  from author's IPs only. The old 444 opening is moot after
  single-443 (nothing binds it).
- Service runs as unprivileged user, `NoNewPrivileges`,
  `ProtectSystem=strict`, `ReadWritePaths` limited to
  `/var/lib/apostille` + log dir (systemd hardening in the
  unit file).
- No customer PII in the repo, ever (schema + migrations
  only). No PII in session logs, prompt logs, or commit
  messages — enforced by `secrets-scan` + reviewer habit.
- Incident note: suspected breach → rotate EAB/keys, ship
  offsite audit log, then investigate. Data is SQLite +
  files: easy to snapshot, easy to reason about.
