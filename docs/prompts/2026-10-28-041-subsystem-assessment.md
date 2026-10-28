# Prompt 041 — Subsystem assessment (6 streams + parent synthesis)

- Prompt timestamp (UTC): 2026-10-28T14:52:00Z
- Response timestamp (UTC): 2026-10-28T15:20:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Deleted. Resume.
```

(Operator deleted the 5 root-owned stale releases; resume work
towards the goal.)

## Response

Cleanup verified first: `/srv/apostille/releases/` holds 6
releases, all hal-owned (1afa66a live + 5 recent history);
hal now fully owns the release lifecycle. Live: `0.1.0+1afa66a`.

Then assessed every subsystem against the goal's done-criteria
(tests pass + v1 deployed + admin config UI + collecting data
per jurisdiction): 6 read-only streams (ingest, agent,
approval/publish, P2/MCP, voice, SPA — full drafts recovered
from child session logs to `/tmp/assess-*.txt` after truncated
delivery), parent synthesis, parent verification of one
load-bearing claim per stream + live probes.

Findings (unanimous pattern: working halves, missing joins):

- Collect works manually (`xtask fetch-url`, admin workers API,
  `"mode": "manual"` verified), analyze works on hand-fed text
  (Stub; rendered prompt discarded, verified), approval API +
  triage UI work, `build-handbook` builds atomically with rows.
- Nothing feeds the front (no capture→analyze bridge,
  `capture_id` always 0; no schedule; Meta client is an
  honest error) and nothing serves the back (public SPA is
  placeholders with zero backend calls, verified; `public_facts`
  live with zero UI callers; publish HTML unserved + unshipped;
  Svelte-vs-static undecided = O1).
- P2: form half + label half work, no shipment↔submission join
  (shell `form_template_id=0` row verified); carriers
  uncertified (empty Bearer), manual queue has no UI.
- Voice runtime path complete + tested; operator-blocked on
  Telnyx keys + proof call; no transcript→fact bridge; audio
  upload rejected (`UnsupportedType` verified).
- Prod content: 1 authority, 0 facts, 1 capture (read-only
  query via the hal-access wrapper) — the loop has never run
  in prod. `public_facts` route live (404 `unknown authority`
  for all slugs).

Central verification this turn: `cargo xtask ci` exit 0 (488
passed, 0 failed), `node --test web/test/` 66/66.

Ranked slices proposed (A–F in the durable todo list):
A serve gap (O1 decision + `public_facts` UI + banners),
B collection loop (source-driven sweep + capture→analyze
bridge), C operator unlocks (Telnyx/terms/META key),
D P2 join, E journeys depth, F CI hygiene. Recommendation:
A first (makes v1 real), B second (fills it), C in parallel;
O1 recommended = Svelte SPA + live `public_facts`, builds stay
audit artifacts (build-pinned reads as follow-up). Awaiting
author confirm via structured input.

No code changed this turn (assessment only).

## Commits

None (assessment only; no repo changes).

## Follow-ups

- Author confirms next slice + O1 serving decision (asked).
- O2 slug (`vietnam` vs `viet-nam`), O3 build runner/trigger
  ride with Slice A.
- Operator items unchanged and growing urgent for data:
  Telnyx keys + proof call, terms read, META_API_KEY, Gmail
  `mailer.*` rows, relay-sender registration.
