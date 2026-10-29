# Prompt 077 — agree-all execution: RISC + device cap + 16 decisions

- Prompt (UTC): 2026-10-29 (session turn; concise "go" continuing prompt 076)
- Response (UTC): 2026-10-29
- Author: Andrii (operator)
- Agent: Muse Code (parent) + 4 read-only children (scribe, risc, devcap, ops)

## Prompt

```text
go
```

(Context: continues the prompt-076 "agree-all" execution — apply the 16
agreed OPEN outcomes via subagents, implement RISC + device cap.)

## Response

Harvested 4 child briefs from session logs (delivery truncated; full
texts recovered per cohort rule 6), verified every outcome against
code/spec, and found only 4 of 16 outcomes matched their numbered
rows. Applied with corrections rather than verbatim:

- Clean (applied as keyed): OPEN-1 (live-proof sprint), OPEN-8
  (method-only profile), OPEN-19 (offline CA custody).
- Re-keyed to the rows they actually answer: author's "OPEN-13"
  (reject TOTP) → OPEN-3; author's "OPEN-22" (envelope+drill) →
  OPEN-23; author's "OPEN-24" (build RISC) → OPEN-7.
- Already-normative (confirmed, no new row): 15-min single-use
  magic links (`MAGIC_LINK_TTL_SECS`=900 + §12); UV-required
  (spec §5 — code gap flagged: builder sets no explicit policy);
  no avatar surface; no `family_uid` — all verified absent/present
  by grep. True OPEN-4/15/28/32 stay open as written.
- New scope (recorded as decisions, no rows): delete+30-day purge
  (55), device cap 5 (54, built), 30-day second staffer (56),
  suspend=revoke-all (57, actionable via existing per-CN
  commands). True OPEN-14/17/21/23-quorum stay open.
- 24h invites recorded as a §5.1/runbook amendment (58), not an
  OPEN-20 answer (delivery channel stays open, split default).

Implemented this turn:

1. RISC receiver (OPEN-7, spec §9.9): `POST /auth/google/risc`
   (`crates/web/src/oauth.rs::risc`, routed in `main.rs`),
   `auth::risc::verify_set` (kid→JWKS→RS256→claims→events/
   subject, `RISC_CERTS_URL`, 300s skew), actions reuse
   `signal_action` verbatim, 202+audit on unknown (R6), rate
   `gr:{ip}` 30/min, `spawn_blocking` per callback precedent.
   Re-auth revives disabled links (`revive_disabled_google_link`;
   login + link mode; `google_link_revived` audit) instead of
   forking accounts.
2. Device cap 5 (decision 54): `MAX_CREDENTIALS_PER_ACCOUNT`,
   `live_credential_count` (live credential rows only),
   `store_credential_capped` (count inside the immediate txn),
   `clear_step_up`; mode-(b) start 409s early, finish is
   authoritative; staff unchanged (admin-assisted per #50).

Docs: spec §9.9 + §2 cap notes + §5.1 mandate; §8 rows resolved
(OPEN-1/3/7/8/19, 23 partly); README decisions 49–60; runbook §1
custody, §2 24h examples, §4 suspend, header 30-day commitment,
§6 item 9 sprint note; Drill 4 break-glass in `drill.md`.

Assumptions: revive-on-verified-login implements existing spec
text ("disabled until the user re-auths"), not a new decision;
expired-`exp`/email-subject/unknown-format handling per the
researched draft; custodian name + 30-day dates left TBD for
operator fill (not re-asked as decisions).

Files changed: `crates/auth/src/risc.rs` (new), `lib.rs`,
`oauth.rs`, `passkey.rs`; `crates/web/src/oauth.rs`, `main.rs`,
`passkey.rs`; `docs/specs/customer-auth.md`, `docs/specs/README.md`,
`docs/ops/staff-enrolment.md`, `docs/ops/drill.md`.

Verification: `cargo test -p auth` (81 pass, 11 new); `cargo test
-p web oauth::` 21 pass ×3 runs (10 new RISC); `cargo test -p web
passkey::` 23 pass (1 new cap); full `cargo xtask ci` exit 0
(fmt + clippy `-D warnings` + all suites + playwright + tailwind).
Pre-commit secrets-scan clean on all three commits (RSA fixtures
are JWK JSON, no PEM armor).

## Commits

- `17dcf56` RISC receiver (OPEN-7): POST /auth/google/risc + re-auth revive
- `1efa5e1` Device cap 5 (decision 54): capped credential insert, 409 at cap
- `d83f177` Record 16 OPEN outcomes as decisions 49-60 + runbook/Drill 4
- (this log entry + index, committed immediately after)

## Follow-ups

- Operator fills: CA custodian name (§1/commitment), 30-day dates
  from first `staff_enrolled` audit row; Google stream registration
  (`stream:update` + verification handshake) before prod RISC.
- Staging live-verify: SET `iss` slash-vs-bare distribution,
  subject formats seen (R1/R4), certs fetch latency.
- Code gaps noted, not built: UV builder policy enforcement,
  single-command suspend wrapper, offline CA issue/import path,
  self-serve delete + 30-day purge (decision 55 scope).
- True OPENs still open: 4, 13, 14, 15, 17, 20, 21, 22, 23
  (quorum), 24, 28, 32 (+ others outside this batch).
- Not deployed this turn (no deploy request since the go-plan).
