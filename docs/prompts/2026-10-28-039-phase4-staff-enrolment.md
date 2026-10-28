# Prompt 039 — Phase 4 staff enrolment (streams + parent implementation)

- Prompt timestamp (UTC): 2026-10-28T05:59:37Z
- Response timestamp (UTC): 2026-10-28T06:23:50Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Continue
```

## Response

Phase 4 (staff passkey enrolment + login) completed end to
end. The spec's own roadmap (§1: "Staff enrolment endpoints
are Phase 4") set the scope; 4 read-only streams ran first
(spec, backend, UI, guide — drafts recovered from child
session logs to `/tmp/phase4-{spec,backend,ui,guide}.txt`
after truncated delivery), then the parent implemented as
single writer per AGENTS.md §6.

Parent reconciliation decisions (drafts conflicted):

- Enrol URL: `https://admin.andrii.ai/admin/enrol?invite=`
  (admin `/admin/*` convention; spec/guide drafts each named
  a different wrong path).
- Enrol finish mints the staff session (mirrors customer
  register; UI draft said otherwise).
- Separate challenge kinds `staff_register`/`staff_login`
  (branch confused-deputy boundary, cf. OAuth D2; backend
  draft proposed reusing customer kinds).
- Offboard: single `admin-offboard`, cert-first (prevents
  resurrection races), per-step transactions (no rollback of
  the kill); `--user` + `--cn` until OPEN-15.
- xtask flag style (`--role/--cn/--days/--origin`), not the
  positional sketch.
- Consume order at finish: cert recheck → CN match → consume
  → verify → persist (mismatch never burns — burning would
  let any cert-holder DoS an enrolment; burn-on-verify-fail
  fails closed with fresh-invite recovery).
- login_finish requires a live presented cert (any CN until
  OPEN-15) and records the fingerprint in the audit.
- Deferred: standalone cert-probe endpoint (enrol/start
  echoes role+cn+fp_suffix instead), conditional-ui build
  (§2.5 claim corrected, API verified, OPEN-16 schedules).
- OPEN-3/OPEN-4 parent-recommended (admin-assisted recovery
  only; no admin-branch OAuth) pending author confirm.

Verified against sources (not assumed): webauthn-rs 0.5.5
`start_*` hardcode UV Required so both finishes enforce it
(no registration-side `user_verified()` exists — the
planned enrol assert was removed); `Option<Extension<T>>`
is axum-0.8-native; `credential_owner`/counter/`Role::parse`
shapes mirrored exactly; admin `csrf_guard` layer exempts
only ceremony paths (start has no token to echo yet).
Finding: `AdminAuth`'s cert check is revoked-only (no
expiry) — pre-existing SEC hardening item, deliberately
unchanged; enrolment enforces expiry.

Files changed (commit `df4bc6f`):

- Spec: `docs/specs/customer-auth.md` (§5 expansion,
  §§2.5/1.5 corrections, OPEN-15..24, appendix folded into
  §8), `docs/specs/README.md` (decision 34).
- Auth: `src/lib.rs` (`staff_invite` DDL),
  `src/passkey.rs` (staff kinds, invite fns, staff account,
  cert-CN lookup + tests).
- xtask: `Cargo.toml` (+auth path dep, zero lock crates),
  `admin_cert.rs` (`valid_cn` shared), new
  `admin_staff.rs` (`admin-invite` + `admin-offboard` +
  tests), `main.rs` (wiring + help).
- Web: new `staff_passkey.rs` (4 handlers + 10 HTTP tests),
  `admin.rs` (`presented_cert_cn`, `require_step_up`,
  ceremony routes + guard exemption, `set_config` step-up
  pilot, `DEV_CERT_HEADER` shared), `main.rs` (mod),
  `passkey.rs` (two helpers shared, module-doc fix).
- UI: `lib/admin/staff-auth.js` + `test/staff-auth.test.mjs`
  (11 tests), `(admin)/admin/login` + `/admin/enrol`
  pages, prerender-safe layout gate, `smoke.mjs` routes.
- Guide: `docs/ops/staff-enrolment.md` runbook,
  `config-values.md` keyless pointer, playbook header fix
  (OPEN-18).

Verification performed (all observed this turn):

- `cargo xtask ci` → exit 0 (fmt, clippy `-D warnings`,
  29 suites: auth incl. 4 new invite tests, web incl. 10
  new staff tests, xtask incl. 2 new lifecycle tests).
- Live CLI drills on scratch stores: invite cert-gate +
  role validation fail correctly; invite prints the
  single-use URL.
- `node --test web/test/` → 57/57 (11 new).
- `npm run check` → 0 errors; `check:imports` OK (after
  moving auth pages under `/admin/*` to resolve a
  route-conflict with `(customer)/login`); `build` +
  `build:admin` done; `smoke` OK (new routes asserted in
  the admin manifest, absent from public).
- `web-budget` → 225690/226304 (public bytes identical —
  staff UI ships in the admin bundle only);
  `packs-validate` clean; `secrets-scan` clean (314 files).
- `Cargo.lock` diff is exactly the xtask→auth path edge.

## Commits

- `df4bc6f` — Phase 4 work (this entry describes it).
- `1c6856c` — this log entry + README index (docs only).

## Deploy addendum (appended 2026-10-28T06:26Z, same turn)

Staged and deployed as hal (third fully-hal end-to-end ship):

- `cargo zigbuild --release -p web -p xtask --target
  x86_64-unknown-linux-gnu` AFTER the log commit (Phase-3
  lesson), so binary SHA == version `1c6856c`. Freshness
  verified via `strings` (`staff_enrol_refused` +
  `1c6856c` present) before staging.
- Staging `/home/hal/apostille/staging/1c6856c/` (ELF,
  40-hex SHA, `packs/` included per the Phase-3 lesson).
- `ops-deploy.sh 1c6856c --skip-accept` (announced: zero
  diff in `packs/`, `handbook-store/`, no `*.sql`; the new
  `staff_invite` table self-migrates idempotently at boot).
  Over bb54d04 (kept, 9 releases total, within the 10 cap).
- Live proof: `healthz` → `0.1.0+1c6856c`; enrol/start
  malformed invite → 400 `invite malformed`; no-Origin →
  403 `csrf check failed` (handler-level, through the
  exemption); login/start unknown → 401; GET → 405;
  `/admin/login` + `/admin/enrol` → 200; `current` →
  1c6856c; service active.

## Follow-ups

- OPEN-15..24 carried in spec §8 (6 author-owned ops
  questions: CA custody, delivery channel, key copies,
  minters, quorum, plus CN mapping; SEC: destructive list,
  CRL/OCSP; OPEN-16 schedules conditional-ui).
- Stage + deploy `df4bc6f` (or the log commit for SHA
  alignment, per the Phase-3 lesson), verify live, then
  the staff-enrolment staging checklist (needs operator:
  test CNs, staging admin origin, real ceremony on
  hardware keys — no full enrol/login finish is unit
  testable without an authenticator).
- Pre-existing operator items unchanged: Telnyx keys + 8am
  proof call, www SAN, Google/Apple/Meta portal work,
  `apostille-ship` grant removal.
