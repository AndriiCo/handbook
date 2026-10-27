# Threat model: Apostille Handbook (Day-1 slice)

- Status: draft for author review. Scope: the code as it exists now
  (axum scaffold + demo sessions + store + analysis runner + xtask),
  plus the deploy path in `docs/ops/deploy.md`.
- Rule: a control is listed as IMPLEMENTED only with a `file:line`
  the author read; everything else is marked planned with its owning doc.
- Line numbers are as of commit `510afaa`; symbols over lines when
  they drift. Prompt-015 parent merge notes: Caddy is OUT (in-app
  rustls shipped), access logging SHIPPED, the systemd unit is
  DRAFTED (`docs/ops/apostille.service`, uninstalled).

## 1. Assets

| Asset | 1-line impact if compromised |
|---|---|
| Customer PII / uploads (photos, scans, IDs) | Identity theft, regulatory harm; slice holds synthetic data ONLY |
| Session tokens (`demo_session`) | Account impersonation within demo scope |
| Handbook integrity + citations | Users plan visits / mailings on false facts; trust loss |
| Audit log | Loss of attribution; tampered history hides abuse |
| TLS keys + ACME EAB secrets | Traffic interception / impersonation of andrii.ai |
| `META_API_KEY` + carrier/Stripe keys (future) | Spend, quota theft, customer-media exposure via processors |
| Server availability | Demo dead; watch/publish loop stalls |

## 2. Actors

- Anonymous network attacker: probes `/apostille/*`, WS, login; no creds.
- Authenticated demo customer: holds a self-minted session; tries IDOR,
  other customers' refs, stored-XSS via reflected fields.
- Malicious customer upload (F8 path): hostile filenames, polyglots,
  oversized media; arrives Day 3+ (no upload endpoint exists yet).
- Compromised dependency: malicious crate / npm package / poisoned
  zig or certbot binary; build-time code execution.
- Malicious insider / operator: root on the box, or push access to the repo.
- Thief with disk image: offline copy of SQLite + `store/` + creds.

NON-threats (explicitly not modeled): nation-state with Google-PKI or
Meta-internal access; the author's Mac being already rooted (deploy signer
is trusted); customers attacking each other's browsers outside our pages.

## 3. Trust boundaries (what crosses each)

- Browser ↔ axum (`/apostille`): JSON DTOs, `demo_session` cookie, WS
  frames, static HTML/JS. Untrusted input; validated server-side.
- axum ↔ SQLite/store: SQL via bound params; blob bytes via hex digests.
  Stored `capture.path` is UNTRUSTED (see §4).
- CLI (`xtask`) ↔ store: operator commands; `--actor` is self-declared.
- Deploy path (Mac → server): cross-built ELF + `web/build/` over scp;
  trust = author's Mac + SSH key + pinned zig tarball checksum.
- ACME/Google PKI: EAB secret + HTTP/TLS challenges; certs back.
- Meta / carrier / Stripe APIs (future; Meta client is an unwired
  skeleton): evidence text + media out, drafted records back. Never
  trusted as fact without validation + human approval.

## 4. Per-component threats (STRIDE-lite)

- SPA public/customer JS (`web/src/routes/**`): stored-XSS via reflected
  `customer_ref` (Svelte `{...}` escapes by default — verify per page);
  login page posts to demo login with `credentials: same-origin`. No
  third-party JS (none observed in `web/`).
- axum routes (`crates/web/src/main.rs`): healthz/version unauthenticated
  by design (version string leaks build id — minor); `/static` ServeDir
  traversal is handled by tower-http; unknown `/api/*` paths 404 as
  JSON via an explicit wildcard (FIXED in parent merge — was SPA
  fallback 200); `LoginBody` unknown-field rejection blocks
  mass assignment.
- Demo sessions: NO credential check — anyone mints a session for ANY
  `customer_ref` (`:98-112`); tokens stored PLAINTEXT in DB (`:99-101`
  comment); cookie lacks `Secure` and `__Host-` (`:130`); no rate limit
  (`:124` comment admits Day-7 work); no CSRF/Origin checks anywhere.
- WS echo (`:94-96`, `:201-218`): no auth, no Origin check — acceptable
  ONLY because it echoes bytes and touches no state; real topics
  (`authority:{slug}` …) MUST ship session+Origin+per-topic authz first
  (planned: `docs/SECURITY.md` §1.5–1.6).
- Blob store paths: `capture.path` from DB is untrusted; traversal guard
  fails closed (IMPLEMENTED, §5). `blob_path` hex-gated (IMPLEMENTED).
- Retention sweep: was non-atomic + traversalable (SECURITY F1/F2); both
  FIXED (IMPLEMENTED, §5). Remaining: file deletes are outside the DB txn
  by nature — crash between `remove_file` and COMMIT leaves row-without-
  file (fail-safe direction: re-run deletes the row; audit may double-log).
- Analysis runner (prompt injection!): untrusted captures/evidence flow
  into the model prompt; exfil/overclaim shapes are: fabricated citations,
  off-enum topics, `{{slug}}`-style template escape. Mitigations
  IMPLEMENTED: evidence substituted LAST (`analyze.rs:14-19`), byte-exact
  citation validator (`:42-88`), closed topic enum (`topics.rs:110-139`),
  proposed-only storage + atomic audit (`runner.rs:99-131`), never
  auto-accepts. Residual: validator checks citation SHAPE, not truth —
  human approval (planned, roadmap Day 3) is the truth gate.
- Migrations: forward-only at boot, single txn (IMPLEMENTED); rollback
  across a migration needs explicit author call (`deploy.md:100-102`).
  005's id-0 sentinel rows are labeled quorum-breakers — queries must
  exclude them where provenance matters.
- xtask CLIs: duplicate flags silently take the FIRST (`main.rs:66-74`,
  SECURITY F6 still open); `--actor` defaults to `xtask` (unattributed
  audit rows); `@path` input reads arbitrary local files (operator-only
  tool — acceptable, document it).
- Service process: hardened unit DRAFTED
  (`docs/ops/apostille.service`: `User=andrii-ai`, only
  `CAP_NET_BIND_SERVICE`, `NoNewPrivileges`,
  `ProtectSystem=strict` — UNINSTALLED, needs root review).
  Live state is still interim user-space: `setsid
  ./current/web` as user `hal`, localhost-only, dies on reboot
  (`deploy.md` interim section). In-app rustls terminates TLS
  (axum-server, certs from files, fail-closed config).
- secrets-scan hook: pre-commit trampoline + CI gate (IMPLEMENTED, §5).
  Gaps: binary files SKIPPED (`secrets.rs`); untracked files never
  scanned until staged; `--staged` covers ACMR only.
- Logs: request-ID access log SHIPPED (one span per request:
  id/method/uri/status/latency, `x-request-id` echo,
  `request_id_and_log` in `crates/web/src/main.rs`). PII-free by
  construction (fixed fields; URLs never carry secrets).

## 5. Mitigations mapped to IMPLEMENTED controls

- Bound SQL params everywhere, no `format!` SQL: `db.rs` (all queries).
- FK enforcement + WAL/busy-timeout/sync PRAGMAs: `db.rs:30-37`.
- Transactional migrate: `db.rs:43-73`. CHECKs + FKs + id-0 sentinels:
  `migrations/005_constraints.sql`. Append-only audit triggers:
  `migrations/002_hardening.sql:8-16`, recreated `005:250-257`.
- Atomic retention sweep + `saturating_sub` + traversal guard:
  `db.rs:141-189`, `:194-200`.
- Hex-validated `blob_path → Result`: `store.rs:54-71`. Atomic
  temp+rename blob write, random suffix + `create_new`: `store.rs:79-129`.
- 256-bit CSPRNG session tokens: `main.rs:113-117`. `HttpOnly` +
  `SameSite=Lax` + `/apostille` path: `main.rs:130`. Expiry enforced
  server-side: `main.rs:170-172`. Expired-row sweep on login: `:125`.
  Generic error shapes (no SQLite text leaks): `:193-199`.
- Citation validator / prompt ordering / redaction helper:
  `analyze.rs:14-19`, `:42-88`, `:95-117`. Proposed-only atomic runner:
  `runner.rs:99-131`. API-key Debug redaction: `model.rs:137-144`.
- secrets-scan rules + CI-first gate + hook installer:
  `xtask/src/secrets.rs:21-44`, `:87-183`; `xtask/src/main.rs:96-98`,
  `:319-370`, `:373-402`; `.githooks/pre-commit`.
- Deploy: localhost-only bind `main.rs:239`; zig tarball sha-pinned
  (`deploy.md:22-25`); SSH key never in repo.

PLANNED (owning doc): rate limits + lockout (`SECURITY.md` Baseline);
Origin/CSRF + security headers + `__Host-` cookies (same); token hashing
(demo-login comment); real auth playbook (`auth-playbook.md`);
TPM-sealed config (`secure-config.md`); certbot issuance + unit install
(`certificates.md`, `apostille.service` — both drafted, need root);
offsite audit ship + backup drills (`prod.md`, `SECURITY.md` Q8);
upload allowlist/magic-bytes/EXIF/redaction pipeline (F8/F9,
`SECURITY.md` Uploads).

## 6. Residual + accepted risks

- ACCEPTED (slice): demo auth with no password and plaintext tokens —
  safe ONLY while data stays synthetic and the slice stays localhost /
  author-audience. Real auth before any PII (`roadmap.md` §2).
- ACCEPTED: env-only secrets — box has NO TPM (`deploy.md:11-12`), so the
  TPM design's fallback is ACTIVE. Revisit on hardware change.
- RESIDUAL: no staging; direct Mac→prod deploys allowed until day 10
  (`deploy.md:104-109`). `xtask release` gate still to be built.
- RESIDUAL: supply chain — `cargo install cargo-zigbuild` unpinned, no
  `cargo audit`/`cargo-deny` in CI yet, no npm audit gate; `Cargo.lock` +
  `package-lock.json` committed (good) but unaudited.
- RESIDUAL: audit triggers stop code bugs, not `DROP TRIGGER` by SQL
  access (`SECURITY.md` Q9); no offsite audit ship yet.
- RESIDUAL: Meta API data-handling terms unread — no real media to Meta
  until SEC reads them (`roadmap.md` §5).
- RESIDUAL: single box, no reboot survival (Linger=no), no backup proven
  by restore drill yet.

## 7. Out of scope

- DDoS absorption (single small box; 429s + firewall only raise the bar).
- Physical theft beyond disk encryption; powered-off FDE assumed, not this doc.
- Breaches inside Meta / carrier / Stripe / Google PKI — we bound exposure
  (least data sent, write-only secrets, single-use EAB), we cannot fix them.
- The author's Mac / SSH key hygiene (trusted signer; key compromise =
  redeploy from a clean checkout + rotate EAB/keys).
