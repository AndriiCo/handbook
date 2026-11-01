# Apostille Handbook

Every apostille fact shows its quote, source and date — drafted by Meta AI,
approved by a human. Live at <https://andrii.ai/apostille>.

> Demo video: _no video yet — operator replaces this line with the video URL on 2026-11-03._

Built by Andrii Co. for the
[Meta AI Developer Hackathon](https://cerebralvalley.ai/e/meta-ai-developer-hackathon.md)
(Startup Track), 2026-10-26 to 2026-11-04 with Muse Code — every builder
and judge turn retained in [docs/prompts/](docs/prompts/) (see
[docs/prompts/sessions/README.md](docs/prompts/sessions/README.md)).

## Which Meta models run where

- **muse-spark-1.3** drafts every proposed fact from fetched captures
  (quote-checked, refused quotes retried), judges photo-precheck verdicts,
  and grades the eval gold set.
- **sam-3.1** locates document marks in precheck photos via the Responses
  API (`<|box;…|>` tokens become the overlay boxes).

## What is live and what is replayed

Live: drafts, eval, and approvals run against the Meta API (sealed
production key; staging-only key on plant). The public precheck endpoint
serves recorded verdict JSONs verbatim — zero marginal model cost, no key
on the serving path. Replays and fixtures: `evals/authorities/` (26-case
gold set), `web/e2e/fixtures/`, crate unit tests.

## Eval numbers (2026-10-31, staging, live key)

26 gold cases, model muse-spark-1.3: quote_found_rate **0.7**,
value_in_quote_rate 0.7, topic_accuracy 0.7; 17/17 staged proposals
accepted through the review flow (8 Viet Nam + 9 USA-WA-SOS); A2
clean=pass, dark/wash=retake.

## Eval numbers (2026-11-01, production, live key)

26 gold cases, model muse-spark-1.3: quote_found_rate **1.0**,
value_in_quote_rate 0.969, topic_accuracy 0.641; written to the live
store by `ops-a5-eval.sh`, shown on the home page.

## The problem

People send documents overseas once every few years, so they shouldn't have
to study government sites that change quietly.

- Washington's apostille counter left Olympia for Tumwater in June 2025, yet
  searches still surface superseded forms.
- On 13 October 2026 even our own site sent Viet Nam customers to consular
  legalization, though the Apostille Convention took effect there on
  11 September 2026.

## The vision

The Apostille Handbook aims to be one source for every country: every detail
and form, then prepaid labels and envelopes delivered to your door, kept
current so nobody plans a visit around a two-year-old address or mails the
wrong form. It plans to cover all authorities on Earth, starting with
106 apostille authorities in the U.S., Canada, and Mexico.

## The 10-day build (with Muse Code)

Automate that upkeep for the U.S., Canada, and Mexico first, then add other
countries (Build for Your World) — a new country means new sources, not new
code.

- **Days 1–4:** watch the handbook's 500+ cited URLs, HCCH's feed, and an
  inbox for authority notices; Muse Spark turns only changes into cited
  records the author approves, reading English, Spanish, or French pages and
  forms to flag contradictions like Washington's two PO Boxes.
- **Days 5–6:** one consular legalization chain (Egypt in USA) with
  pre-filled authority forms.
- **Days 7–8:** walk-in reports (route photos, parking tips, rejections)
  update the handbook, SAM 3.1 masking faces and plates; self-hosted Muse
  Glimmer pre-checks document photos.
- **Days 9–10:** approved changes reach the handbook and MCP server.

## Success criteria

What is live at <https://andrii.ai/apostille> today: search the
authority index, sign in with passkey, run the photo precheck, and place
an order whose timeline updates live. Dossiers show quote-carrying facts
(staging-proven; production facts appear once the operator's approval
steps below land). What is demo-only: document photos are synthetic
samples, and there is no real mail, fees, or carrier charges anywhere.

## Repository

Rust workspace plus a Svelte front end (`web/`). Project tasks live in
`xtask/` and run via `cargo xtask <task>`; operator runbooks live in
`docs/ops/scripts/`.

```text
crates/handbook/   library crate (design arrives in a later prompt)
xtask/             task runner (`cargo xtask ci` = fmt check, clippy, tests)
docs/prompts/      complete log of prompts and responses
AGENTS.md          working agreements for coding agents
LICENSE            proprietary license, Andrii Co.
```

Build, test, and lint:

```sh
cargo build
cargo test
cargo xtask ci
```

## Design

See [docs/design.md](docs/design.md) — the proposed evidence-store design
(Step 1: collection infrastructure). Feature specs, visual design, and
security baseline live in [docs/specs/](docs/specs/),
[docs/site-theme.md](docs/site-theme.md), and
[docs/SECURITY.md](docs/SECURITY.md) (all drafts for review).

## Prompt log

Every prompt and response is retained with timestamps in
[docs/prompts/](docs/prompts/), one Markdown file per prompt, committed
alongside the work it produced.

## License

Proprietary. Copyright (c) 2026 Andrii Co. All rights reserved.
See [LICENSE](LICENSE).

Brand font: headings use Vogun Medium (Yukita Creative), a proprietary
face licensed to Andrii Co. It is staged at build time from
`$BRAND_FONT_DIR` (see `web/scripts/brand-fonts.mjs`) and never enters
this repo. Public builds fall back to a metric-matched Source Serif 4
face (OFL, vendored in `web/static/fonts/fallback/`).
