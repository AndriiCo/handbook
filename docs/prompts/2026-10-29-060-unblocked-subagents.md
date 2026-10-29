# Prompt 060 — unblocked work in subagents while operator configures

- Prompt timestamp (UTC): 2026-10-29T02:22:00Z
- Response timestamp (UTC): 2026-10-29T02:30:54Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

OK. Working on it. What others parts of project you can work on
and not blocked? Work in sub-agents.

## Response

Fanned out to 5 subagents (3 read-only surveyors, 1 spec
drafter, 1 isolated builder); parent verified every claim and
wrote all parent-tree files. Four commits, CI 599/0. Nothing
deployed — operator is mid-wizard; the two SEC commits wait
for a pause.

Survey (read-only): 8 ranked unblocked items (forms DS-4194,
F8 intake, F9 redaction shell, Egypt chain-view, transcript
annotate, F10 Glimmer seam, webhook probes, accept drills);
SEC review found H1/H2 HIGH + M1–M3 + L1–L4; voice survey
mapped the credential-free slice. Full texts recovered from
child session logs (§6 truncation rule).

Committed (parent-implemented, SEC lane):

- `0a5f507` Burn setup window on first verified enrol in both
  modes (H1+M1+L2). Invite-mode first enrol never burned
  (verified in code); setup-mode burned before verify
  (one failed ceremony bricked fresh servers). Now verify →
  dedup → validate → burn → persist; setup always burns
  (lost burn 410s single-winner), invite burns only when no
  staff creds exist (break-glass survives). Invite compare
  via constant-time `tokens_match`. Staff-count SQL shared
  as `auth::passkey::live_staff_passkey_count` (xtask
  deduped onto it). Regression: failed setup ceremony keeps
  the window.
- `1cb02f6` Setup hardening: 72h key TTL (M2, from
  `updated_at`, no schema change) + registry-type
  enforcement on setup saves (H2 type half). Verified the
  wizard's 56 keys equal the registry minus `setup.key_hash`
  with zero type mismatches — neither change affects the
  legitimate flow. H2's scope half (block admin/tls/auth/
  net/paths/quic) REJECTED as stated: those keys are IN the
  approved 9 wizard steps the operator is filling now.
  Residual scope question surfaced to the author.
- `ab9aeb6` spec: voice-telnyx §11 credential-free
  validation slice (child-drafted; parent fixed 5 wrong
  claims: §8/§9 numbering, source_ref, TelnyxClient
  struct-not-trait, D35, webhook_url row).
- `9609086` (cherry-picked from isolated worktree)
  hermetic webhook negative probes + consent/audit tests
  (7 voice + 2 web-handler probes, tests-only, +723).

Deferred (LOWs, noted): L3 challenge-before-verify reorder
(invasive, leak-required precondition), L4 per-key finish
bucket (256-bit makes brute force infeasible), L1 homebrew
ct compare (fixed lengths; `subtle` follow-up).

Open items for the author: (1) deploy the 2 SEC commits when
pausing the wizard (restart blip only; key/DB untouched);
(2) OQ-3 sweep owner (voice §11.6); (3) whether single-factor
setup should write `admin.mtls_off` et al (approved surface
today — narrowing needs a product call).

## Files changed

- `crates/web/src/staff_passkey.rs` — burn reorder, H1 gate, L2
- `crates/web/src/setup.rs` — type enforcement, M1 test, TTL fixture
- `crates/auth/src/passkey.rs` — `live_staff_passkey_count`
- `crates/config/src/lib.rs` — TTL const + verify, tests
- `xtask/src/config.rs` — shared helper, verify call sites
- `docs/specs/secure-config.md` — §9.1/§9.2 contract notes
- `docs/specs/voice-telnyx.md` — new §11
- `crates/voice/tests/webhook_negative.rs` — new (child)
- `crates/web/src/admin_voice.rs` — tests only (child)

## Commits

- `0a5f507`, `1cb02f6`, `ab9aeb6`, `9609086`
- (this log; hash on commit)

## Verification

- `cargo xtask ci`: exit 0, 599 passed / 0 failed (587 →
  599: +TTL, +M1, +browser-GET, +7 voice, +2 web probes)
- Child worktree tests green before cherry-pick; clean apply
  (no conflicts — disjoint files)
- No deploy this turn (operator mid-wizard)
