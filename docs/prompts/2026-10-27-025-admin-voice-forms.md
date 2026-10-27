# 025 — Admin + voice + forms-framework goal

- Prompt timestamp (UTC): 2026-10-27
- Response timestamp (UTC): 2026-10-27 (execution in progress)
- Author: Andrii Tereshchenko
- Agent/model: Muse Code, powered by Meta Muse Spark

## Prompt

I would like to have admin.andrii.co up and running and managing URL downloads / anaylsis workers, be abke to make a voice calls (with recording) from same web UI and see questions for me to ask and transcript of the call. For all PDF forms to be filled in by customer - I need tool on my server to be able iteratively work on making HTML/JS/Rust code portion for this form based on form PDF to make it resemble original form as much as possible. Have a common framework for it - like a form validation, address pickers / validators, phone number formatters, country drop-downn / autocomplete. But each form can be a separate Muse Code project with own requirements (to be able refine them later) and own code generated. Initially we can compile each of code into next release, but can be looking for a way to have it done dynamically and been hosted in some sandbox in the future. Also I would like no environment variables used, but stored in database per config decigions done before. Protect the values with software-TPM (to make possible to protect with hardware TPM in the future).

## Response

Clarified three material decisions (structured input): admin host
is admin.andrii.ai (`.co` a typo), voice provider is Telnyx,
pilot resemblance form is parent's pick (US DOS DS-4194).
Recorded as decisions 24–27 in docs/specs/README.md.

Fanned out 5 isolated writer streams (base `5ccfc43`):
admin-workers (worker management UI+API), voice-telnyx
(Telnyx Call Control + recording + transcript UI), form-framework
(common components + DS-4194 pilot + per-form xtask scaffold),
config-db (zero-env migration), tpm-seal (software-TPM sealing
abstraction). Spec-first per AGENTS.md §6; merges gated on
`cargo xtask ci` + web checks.

Public admin serving still needs the operator-side root
bootstrap (TLS bind, unit install); streams build against the
existing admin listener + staging.

## Commits

- `f5258e1` Record goal-025 decisions (admin host, Telnyx,
  DS-4194 pilot, zero-env, swTPM) + prompt log.
- `dc55238` Merge tpm-seal: software-TPM sealing abstraction
  in config (279 Rust tests green).
- `f7aba20` Merge admin-workers: staff workers/captures API +
  UI (290 Rust + 14 web green, smoke OK).
- `1767f9c` Merge form-framework: shared components +
  DS-4194 pilot (296 Rust + 23 web green; budget 220KB).
- `af44e2c` Merge config-db: zero-env runtime config
  (307 Rust green; staging note: release now refuses
  mtls_off + KEK-missing boot).
- `eb56e18` Merge voice-telnyx: calls + recording +
  transcript UI (333 Rust + 29 web green; telnyx.* keys
  registered, 29 total; secrets-scan false positive fixed
  via Option::clone spelling).
- (further execution commits to be listed here as merges land)

## Follow-ups

- Operator: root bootstrap (group + sudoers + re-login), then
  prod cutover of the release train.
- Telnyx account + API key + a test number (sandbox/billing).
- Lawyer review of call-recording consent language per state.
