---
name: ship-it
description: Hackathon submission work for the Apostille Handbook - the scripted 3-minute demo recording, the judge-facing README, licence and public-repo preparation (scoreboard row S1). Use from 2026-11-03 or when the operator asks for the demo or the submission package.
---

# ship-it

Submissions close 2026-11-04 21:00 ET (18:00 PT). Judges see a 3-minute
video and the live URL. Never make the repo public, change the licence or
upload anything yourself: prepare it, then list the operator's steps in
`docs/ops/operator-today.md`.

## Demo recording

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
4. `node .agents/scoreboard/scoreboard.mjs --only S1` and list what is
   still the operator's.
