---
name: ship-it
description: Hackathon submission work for the Apostille Handbook - the scripted 3-minute demo recording, the judge-facing README, licence and public-repo preparation (scoreboard row S1). Use now for the demo script and the demo gate (goal.md, while rows wait on the operator), from 2026-11-02 for the submission package, or when the operator asks.
---

# ship-it

Submissions close 2026-11-04 18:00 PST (02:00 UTC 11-05; DST ends 11-01);
the operator submits by 12:00 PST. Judges see a 3-minute video and the
live URL. Never make the repo public, change the licence or
upload anything yourself: prepare it, then list the operator's steps in
`docs/ops/operator-today.md`.

## Demo recording

Write `docs/demo-script.md` first: one row per scene with its URL, the
production data it needs (e.g. two proposals per slug left unapproved for
the approve scene), its seconds and its caption, 175 s in total. Then build
the spec below, dry-run it against production the day after the data
lands, and record the public segments yourself. The operator records the
production admin segment and uploads the video.

`web/e2e/demo.spec.mjs` + `npm --prefix web run demo`: 1440x900 with
`recordVideo` into `web/shots/demo/`, `slowMo` 250, scripted from
`docs/demo-script.md`, captions via `addInitScript` when `DEMO_CAPTIONS=1`,
plus a 390x844 pass over the public screens. Public steps use
`DEMO_BASE_URL` (production) when set. Admin steps (review queue, trace
console) use the local admin build with fixtures; the operator records the
production admin segment because it needs their client certificate.

The story, in order: a stranger finds Viet Nam → the dossier shows
quote-verified facts → Muse Spark drafts a change from a new capture, quote
checked → the operator approves in `/review` → the public page updates live
→ a phone photo goes through SAM 3.1 + Spark precheck → the quality number.

Done: desktop run 150-175 s, zero console errors, no banned strings, paths
and durations reported.

## Submission package (row S1)

1. `README.md` for judges, first screen: one sentence on what it does, the
   live URL, the video link, which Meta models run where, what is live and
   what is replayed, eval numbers with their date, "built 2026-10-26 to
   2026-11-04 with Muse Code" with a link to `docs/prompts/` and to
   `docs/prompts/sessions/README.md` (every builder and judge turn).
2. Regenerate the session log (`node .muse/hooks/session-log.mjs`), then
   a scrub list (delete nothing): every file under `docs/ops/`, server
   names and IPs, sudoers paths, pasted shell or journal output and
   personal data in `docs/prompts/` (the 001-095 files, `LOG.md` and
   `sessions/`). Propose keep, redact or move for each. Redact the digests
   in the generator's output, not by hand: hand edits are overwritten on
   the next turn.
3. A one-paragraph MIT vs Apache-2.0 recommendation. Do not switch it.
4. `docs/ops/security-review-*.md` still lists unfixed findings (H1, H4,
   M1, M2): before the repo goes public, each is fixed or the review moves
   out of the public tree. Say which.
5. `node .agents/scoreboard/scoreboard.mjs --only S1` and list what is
   still the operator's, including the Cerebral Valley form itself.
