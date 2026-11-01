# Demo script — 175 s, misroute to quality number

HERO: `usa-wa-sos` (Assumed — the operator names `vietnam` or
`usa-wa-sos`; switching HERO means the search text, the dossier URL,
and the CTA line below). The HERO dossier carries the richest
evidence: a step-by-step guide, a same-field conflict, and a stale
capture.

Takes: (1) fixture take, always green, `npm --prefix web run demo`;
(2) staging live take as fallback footage,
`DEMO_BASE_URL=http://127.0.0.1:18318/apostille npm --prefix web run demo`;
(3) production take once approvals land (same command against
`https://andrii.ai/apostille`). Recording takes run with
`DEMO_CAPTIONS=1`; the captions below are those lines. Captions name
the model doing the work; latency and token counts are read off the
take's own trace console (logged per take at the bottom).

| # | Time | Scene (URL) | What happens | Caption |
|---|------|-------------|--------------|---------|
| 1 | 0:00–0:15 | Problem (voiceover over `/apostille`) | On 13 Oct 2026 the site sent Viet Nam customers to consular legalization, though the Convention took effect 11 Sep 2026. Stale pages misroute people. | "Stale pages misroute people" |
| 2 | 0:15–0:50 | Search → HERO dossier `/apostille/authority/usa-wa-sos` | Stranger types "Washington", picks the dossier; the camera walks the guide steps (right office, before you start, costs, submit, how long). | "Muse Spark drafts every fact from official pages" |
| 3 | 0:50–1:05 | Evidence popover (same page) | "View quote" opens the popover: the exact quote with the value highlighted, source domain, retrieved date. | "Every fact shows its quote, source and date" |
| 4 | 1:05–1:45 | Review `/review` → approve → guide flips live | A Spark-drafted fact sits proposed beside its capture quote; staff clicks Accept; the HERO guide step flashes the new value over WS `authority:usa-wa-sos`. | "A human approves every Muse Spark draft — the dossier updates live" |
| 5 | 1:45–2:20 | Precheck `/precheck` (390px) | Phone photo of a synthetic sample; SAM boxes trace, verdict with reasons. | "SAM 3.1 and Muse Spark precheck the photo" |
| 6 | 2:20–2:45 | Eval chip + Muse Code beat (home, trace console) | Home chip: verified-quote rate with model and date. Cut to the trace console: one draft run's input/output tokens on screen. | "Muse Spark drafts live — watch the tokens" |
| 7 | 2:45–2:55 | Close on the URL | `https://andrii.ai/apostille` fullscreen. | "Every apostille fact, quoted" |

Total: 15 + 35 + 15 + 40 + 35 + 25 + 10 = 175 s
(search ≈ 8 s lives inside beat 2).

Needs per take: beat 2 wants ≥ 1 accepted fact with a verified quote
on the HERO slug; beat 4 wants 1 proposed fact left unapproved;
beat 6 wants an eval run ≤ 72 h old. The fixture take mocks all
three; staging carries 8 + 9 accepted facts and a 2026-11-01 eval;
production waits on approvals.

## Observed trace-console numbers per take

| Take | Run | Tokens in / out | Proposed / rejected |
|---|---|---|---|
| 1 fixture | mocked `run:9` | 100 / 50 | 2 / 1 |
| 2 staging | live draft on staging | read off the take | read off the take |
| 3 production | live draft on production | read off the take | read off the take |

Reference (staging Run 10, 2026-11-01): a live capture draft
reported `Done — 2 proposed, 0 rejected (muse-spark-1.3,
18862/8785 tok)`.
