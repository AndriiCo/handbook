---
name: ship-it
description: Hackathon submission work for the Apostille Handbook - the scripted 3-minute demo recording, the judge-facing README, the public-repo scrub and secret scan (scoreboard row S1). Use for the demo dry runs and takes, for the submission package, or when the operator asks.
---

# ship-it

Submissions close 2026-11-04 18:00 PST (02:00 UTC 11-05); the operator
submits by 12:00 PST. Judges see a 3-minute video and the live URL. Never
make the repo public, change the licence or upload anything yourself:
prepare it, then list the operator's steps in `docs/ops/operator-today.md`.

## Demo recording

Built: `docs/demo-script.md` (scenes, URLs, data, seconds, captions; 175 s
at most), `web/e2e/demo.spec.mjs` and `npm --prefix web run demo`, which
leaves three takes in `web/shots/demo/`: desktop (1440x900, GP1→GP4),
admin (review approve + trace, on the local admin build with fixtures) and
mobile (390x844). Public steps use `DEMO_BASE_URL` when set; captions come
from `addInitScript` when `DEMO_CAPTIONS=1`.

What is left, in order:
1. Staging: every staging acceptance includes a green live take,
   `DEMO_BASE_URL=http://127.0.0.1:18318/apostille npm --prefix web run demo`.
2. Production dry run, once the operator's approvals have put facts on
   production: `DEMO_BASE_URL=https://andrii.ai/apostille npm --prefix web run demo`.
   Fix what it finds on staging first.
3. The recording take, `DEMO_CAPTIONS=1`, against production. The operator
   records the production `/review` segment (it needs their client
   certificate) and uploads the video.

The story, in order: a stranger finds Viet Nam → the dossier shows
quote-verified facts → Muse Spark drafts a change from a new capture, quote
checked → the operator approves in `/review` → the public page updates live
→ a phone photo goes through SAM 3.1 + Spark precheck → the quality number.

Done: desktop take 150-175 s, zero console errors, no banned strings, paths
and durations reported.

## Submission package (row S1)

1. `README.md` for judges, first screen: one sentence on what it does, the
   live URL, the video link (the operator adds it on 11-03), which Meta
   models run where, what is live and what is replayed, eval numbers with
   their model and date, "built 2026-10-26 to 2026-11-04 with Muse Code"
   with links to `docs/prompts/` and `docs/prompts/sessions/README.md`.
   Repository paths it names must exist (`xtask/ops/`, not
   `docs/ops/scripts/`).
2. Regenerate the session log (`node .muse/hooks/session-log.mjs`), then a
   scrub list (delete nothing): every file under `docs/ops/`, `xtask/ops/`
   and `ops/plant-pipeline/`; server names and IPs, sudoers paths, user
   names, pasted shell or journal output and personal data in
   `docs/prompts/` (the 001-095 files, `LOG.md` and `sessions/`). Propose
   keep, redact or move for each. Redact the digests in the generator's
   output, not by hand: hand edits are overwritten on the next turn.
3. A full-history secret scan before the repo goes public:
   `gitleaks detect --log-opts="--all"` (or the repo's `secrets-scan` over
   every commit). Report clean, or each finding with its commit and the
   fix (history is never rewritten; a leaked credential is rotated by the
   operator).
4. Licence: the operator chooses (builder recommendation MIT, P183). Do
   not switch it.
5. `docs/ops/security-review-2026-10-27.md`: only H1 and H4 are marked
   fixed (P168), with parts of M1 and M2. Treat every other finding (H2,
   H3, H5, H6, M3-M6, L1-L4) and the deferred parts as open until you
   verify the code (e.g. H3: `DefaultBodyLimit::disable()` in
   `crates/web/src/admin.rs`; H5: sessions never checking `expires_at` in
   `crates/auth/src/lib.rs`). Before the repo goes public, each is fixed or
   the review moves out of the public tree. Say which.
6. `node .agents/scoreboard/scoreboard.mjs --only S1` and list what is
   still the operator's, including the Cerebral Valley form itself.
