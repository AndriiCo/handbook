# Plant inference workstream (self-hosted batch vision)

Status 2026-10-31: pilot in progress. Qwen2.5-VL-7B picked for the
smoke test; `cargo xtask plant-run` drives it. Goal: electricity-cost
batch inference (second verdicts, sweeps, eval backfill) with a human
approver as the slowest step. Lease-queue pool + vendor overflow land
after the pilot reports numbers.

## Hardware (plant, verified 2026-10-31)

- GPU: RTX 3060 12 GB (GA106), driver 580.178.04 / CUDA 13.0,
  reachable unprivileged. ~1.5 GB taken by desktop/browser procs,
  ~10.5 GB free. No CUDA toolkit (container-free path: `cudart-`
  llama.cpp bundles its runtime).
- CPU/RAM: 16 threads, 125 GB RAM (~110 free). Disk: 71 GB free on
  /home; `~/apostille-pipeline` is 152 GB — prune before weights grow.
- No docker access (socket permission denied; deferred, not needed
  for the pilot). No tmux check yet (`plant-run start` fails fast
  with `MISSING: tmux` if absent).

## Decisions (author-set, in order)

1. Bare binaries over containers (no root, no group changes).
2. Open-weights with model swap = 4 lines (`ops/plant-inference/MODEL.conf`).
3. Pilot model: Qwen2.5-VL-7B-Instruct Q4 (~5 GB) + mmproj Q8.
   Llama 3.2 Vision 11B considered (fits tight, license friction).
   Muse Spark Open considered: real (816B MoE, multimodal, llama.cpp
   day-one support) but needs ~180-300 GB RAM+VRAM — a future pool
   member on 256 GB-class iron, not plant.
4. HF token hygiene: blind `read -s`, `~/.huggingface.env` at 600,
   sops-encrypted (`~/.sops.yaml` holds the public recipient).
   `cargo xtask plant-run models` reproduces this safely.

## Pilot (Slice A, manual, no repo contract yet)

One image, one verdict JSON: `~/models/run-job.sh <job> <image>`
under `flock ~/apostille-pipeline/gpu.lock`, VRAM sampled to
`<job>.vram.log`, `metrics.json` = verdict + wall_secs +
peak_vram_mib + model + recorded_at, `JOBS.md` append-only log.
Done = three numbers: secs/image, peak VRAM, disk bytes.

## xtask driver (Slice B plumbing, in tree, needs main-session gates)

`cargo xtask plant-run setup|models|start|status|log|fetch|attach`
(`xtask/src/plant_run.rs`, payload `ops/plant-inference/`).
Jobs run in tmux `infer-<job>` with pipe-pane console logs;
`fetch` lands in `web/shots/inference/<job>/` (git-ignored).
NOT YET PROVEN from the side chat (no shell there): `cargo fmt`,
`clippy -p xtask`, real `setup`/`models`/`start` runs. Fold into a
builder slice before trusting.

## Batch pool (design, not built)

- `batch` plant job type: `--type batch --task <name>`, same SHA
  tarball rules, worker pick-order build/probe-before-batch,
  per-task timeout (default 8 h), GPU lockfile, verdicts to
  `artifacts/<run>/verdicts/` in `a2-record.py` shape. Never writes
  `acceptance/*.json`. First tasks: `pilot-vlm`, `a5-backfill`,
  `change-second-verdict`, `citation-sweep`.
- Lease protocol (by Nov 10): queue table, `lease` (30 min +
  heartbeat) / `complete` / `fail` (backoff, dead-letter after N),
  expiry auto-requeues, idempotent by job id. Workers announce
  `{vram_mb, models[]}`. Admin trace gains a read-only workers view.
- Modal overflow (planned, not implemented): same worker, A10G for
  Qwen / H100+ for Spark-class, weights on a Modal Volume,
  plant-first dispatch (overflow past ~6 h queue age or unmet
  requirements), `local_only` opt-out, monthly cap + per-job
  cost ledger. Modal retries stay OFF (leases own retry).
- Vendor batch (Together native Batch API first: JSONL, 24 h
  window, ~50% off ≈ $0.63/$2.13 per M; Fireworks second):
  reference verdicts in the same JSON shape. Verify image input
  support + 3-image serverless pilot before committing.

## Open items for the builder (suggested slice order)

1. Gates for `plant_run.rs` + live `setup`/`start`/`job-001` run;
   record the three pilot numbers here.
2. `batch` job plumbing with a no-GPU smoke task (plant stamp).
3. Lease endpoints (must ride Mon 11-02 deploy or wait).
4. Together 3-image serverless pilot (cents).
5. Modal overflow only after 1-4 green.
