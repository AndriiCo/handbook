# `evals/captures/`

Seed data installed by `cargo xtask up` into a fresh local store.

- `demo_precheck/*.json` — recorded A2 photo-precheck verdicts, one per
  synthetic sample (`syn-doc-clean`, `syn-doc-dark`, `syn-doc-wash`).
  The web tier serves `<store>/demo_precheck/<sample>.json` verbatim,
  so these files use exactly that shape (`verdict`, `issues`, `boxes`,
  `models`, `recorded_at`).
- The files committed here are synthetic stand-ins (`"synthetic": true`)
  so a local boot works with no model key. The operator replaces them
  with live-model recordings via `xtask/ops/a2-record.py`, which writes
  the same shape from SAM 3.1 boxes plus a Muse Spark judgment.

Capture rows themselves are not committed: `up` ingests one capture per
gold case from `evals/authorities/*.json` (synthetic evidence), then
runs `analyze --scan-captures` so `/review` has proposals.
