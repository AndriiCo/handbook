# Demo script (175 s, 3-minute hackathon video)

Story: a stranger finds Viet Nam → quote-verified facts → Muse Spark
drafts a change from a new capture, quote checked → the operator approves
in `/review` → the page updates live → a phone photo passes SAM 3.1 +
Spark precheck → the quality number. Public segments record against
production (`DEMO_BASE_URL`); the admin approve scene is recorded by the
operator (client certificate) against the local admin build with fixtures
until the staging key lands, then against staging.

Production data needed before recording day: vietnam + usa-wa-sos seeded
with accepted facts; **two proposals per slug left unapproved** for the
approve scene; one fresh capture queued for the draft scene; A2 verdicts
recorded; a same-week eval run for the quality number.

| # | Scene | URL | Data needed | s | Caption |
|---|-------|-----|-------------|---|---------|
| 1 | Stranger finds Viet Nam | `/apostille` | 3+ authorities indexed | 20 | "Find any apostille authority in one search." |
| 2 | Quote-verified dossier | `/apostille/authority/vietnam` | ≥8 accepted facts w/ quotes | 30 | "Every fact carries its source quote." |
| 3 | Spark drafts from a new capture | `/review` (queue) or trace | 1 fresh capture, Spark draft | 30 | "Muse Spark drafts; the quote must be on the page." |
| 4 | Operator approves (admin) | `/review` | 2 unapproved proposals/slug | 30 | "A human approves — nothing goes live alone." |
| 5 | Page updates live | `/apostille/authority/vietnam` | WS `authority:vietnam` | 15 | "The dossier updates while you watch." |
| 6 | Phone photo precheck | `/apostille/precheck` | A2 verdicts recorded | 30 | "SAM 3.1 reads the photo; Spark judges." |
| 7 | Quality number | `/apostille` | eval run ≤72 h | 20 | "88% of drafts carry a verified quote." |

Total: 20+30+30+30+15+30+20 = 175 s. Captions via `DEMO_CAPTIONS=1`
(`web/e2e/demo.spec.mjs`, `npm --prefix web run demo`): 1440x900
`recordVideo` into `web/shots/demo/` at `slowMo` 250, plus a 390x844
pass over the public scenes. Done = desktop run 150–175 s, zero
console errors, no banned strings.
