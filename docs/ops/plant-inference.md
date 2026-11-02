# Plant inference workstream — REMOVED (organizer rule)

Status 2026-11-01 (P248): removed. The Qwen2.5-VL-7B pilot
(`cargo xtask plant-run`, `ops/plant-inference/`) called a non-Meta
model at runtime, which the organizer rule forbids: every model the
project calls must be a Meta model. The module, the payload scripts,
and the weights-pulling workflow are deleted from the tree.

Kept for reference: plant's verified hardware (RTX 3060 12 GB, CUDA
13.0 reachable unprivileged; 16 threads, ~110 GB free RAM) — still
true, still useful when a Meta open-weight replacement arrives.

Next: N6, a Meta open-weight model in place of Qwen, starts only on
the operator's order. Until then there is no self-hosted inference
work: drafts, precheck, and transcription all call the Meta Model
API (`https://api.meta.ai/v1`), and the public precheck serves
recorded replays.
