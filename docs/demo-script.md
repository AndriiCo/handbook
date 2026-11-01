# Demo script — 175 s, stranger to quality number

Public segments: recorded by the builder (`npm --prefix web run demo`
with `DEMO_BASE_URL=https://andrii.ai/apostille`, `DEMO_CAPTIONS=1`).
Admin approve scene: recorded by the operator (needs their client
certificate) and uploaded to YouTube. Captions below are the
`DEMO_CAPTIONS` lines.

| # | Time | Scene (URL) | What happens | Needs on production |
|---|------|-------------|--------------|---------------------|
| 1 | 0:00–0:20 | Home `/apostille` | Stranger types "Viet Nam" in the command search, picks the dossier. Caption: "Find your apostille authority in seconds" | Authority index live (done) |
| 2 | 0:20–0:55 | Dossier `/apostille/authority/vietnam` | Scrolls topics, opens "View quote" on a fact: exact quote, source domain, retrieved date. Caption: "Every fact shows its quote, source and date" | ≥ 1 accepted fact with a verified quote (open: approvals) |
| 3 | 0:55–1:20 | Review `/review` (admin) | A Spark-drafted fact sits proposed beside its capture quote. Caption: "Muse Spark drafts from a new capture, quote checked" | 2 proposed facts per slug left unapproved (operator keeps 2 back during approvals) |
| 4 | 1:20–1:45 | Approve → dossier live | Operator clicks Accept; the dossier's evidence ledger gains a verified entry over WS. Caption: "A human approves — the dossier updates live" | Same queue as scene 3 |
| 5 | 1:45–2:25 | Precheck `/precheck` (390px) | Phone photo of a synthetic sample; SAM boxes trace, verdict "Retake" with reasons. Caption: "Photo precheck before you order" | A2 recordings (done: pass/retake/retake) |
| 6 | 2:25–2:55 | Home quality chip `/apostille` | Eyebrow chip: verified-quote rate with model. Caption: "Most drafts carry a verified quote — Muse Spark" | Fresh eval run ≤ 72 h (open: A5 re-run after Sunday #3) |

Total: 20 + 35 + 25 + 25 + 40 + 30 = 175 s.

Fixture gate (`npm run demo`, no base URL) is green on 2026-11-01:
desktop + admin + mobile, 3/3, zero console errors. The live take
against production waits on the two "open" rows above.
