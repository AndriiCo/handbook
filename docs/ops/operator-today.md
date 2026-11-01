# Operator — Saturday 2026-10-31 (Pacific; updated ~20:50 PDT)

Production runs `0.1.0+d93a58c` — verified 2026-11-01T03:46Z, scoreboard
**4/10** (P0, GP1, GP4, A2 pass). First red: GP2-data (0/8 accepted per
slug). Saturday deploy #2 (redesign slices P153–P157) is **blocked**:
plant `/home` is 100% full, so gate submits abort mid-upload and P154's
gate died compiling. Builder's orphan partial tarballs are removed
(queue empty). Free space and paste back `df -h ~`.

Rootless ops stand (run everything as `hal`, never root). The KEK
group-read pre-step from this morning is done — A2 reads prove it.

## Done (verified, struck through)

- [x] Proposals script — ran 01:41Z: vietnam 0 proposed / 0 accepted,
  usa-wa-sos 17 proposed / 0 accepted, analyze proposed 0 new. Nothing
  further to run until the next deploy ships new packs.
- [x] A2 recording — ran 01:41Z, 3 samples installed, prod A2 **PASS**
  (`verdict: pass`, 150 ms, sam-3.1 + muse-spark-1.3, verified 03:50Z).
  The `chgrp apostille /var/lib/apostille/demo_precheck` fix below is
  confirmed working — no action needed.
- [x] A2-fix chgrp line of 2026-11-01 — resolved by the passing verdict above.

## Still open (in order)

1. Approvals — no script: open the admin `/review` with your client
   certificate, accept every proposed fact EXCEPT 2 per slug (leave 2
   vietnam + 2 usa-wa-sos proposed for the video's approve scene).
   GP2-data needs >= 8 accepted each; currently 0/8. This flips
   GP2-data, A1 and GP2-page. Paste back: accepted counts per slug.
2. A5 eval — attempted 01:41Z, **ABORTED** (`no gold files`: `evals/`
   never reaches a release). Needs Sunday deploy #3 carrying fix 5,
   then re-run `bash ~/apostille/scripts/ops-a5-eval.sh` and paste
   back the quality.json lines.
3. S1: switch LICENSE to MIT, make the repo public, link the demo
   video in README (video lands 11-03). Video: record the production
   admin approve scene yourself (needs your client cert; builder takes
   desktop/mobile), upload to YouTube, paste the URL back.
4. GP3 brand assets (scoreboard demands a logo per configured
   provider button; dir still holds only README.md): run
   `cargo xtask brand-assets`, follow its four download steps (Apple,
   Google, Meta, FIDO), then `cargo xtask brand-assets --check
   --minutes 60`. Keep one SVG per provider prod configures in
   `web/src/lib/assets/brand/` (operator-owned, never committed),
   then redeploy — the login page renders text-only buttons until
   they land.
5. Plant disk (blocks everything): see top. Builder cannot free
   shared pipeline state itself.

## Queued (not today)

- New goal (memory `goals-queue.md`, handed over in chat — paste it
  into `.agents/goal.md` yourself; builder never edits it): Muse
  Glimmer self-hosted documents/facts evaluator on plant.
- Inference workstream (full design at docs/ops/plant-inference.md):
  `cargo xtask plant-run` + `ops/plant-inference/` Qwen2.5-VL-7B
  pilot (uncompiled/unproven — needs `cargo clippy -p xtask`, a live
  `setup`, and the `job-001` run with its three numbers).

## Builder state (info only)

Web redesign slices P153–P157 landed on main (shell + GP1–GP4 + login);
local gates green (check, both builds, imports, tailwind, 20/20 shots).
Staging 9/10 (only S1 red). Prod 4/10 until approvals + Sunday #3.
