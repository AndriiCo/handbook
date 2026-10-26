# Apostille Handbook

One source for every country's apostille details and forms — kept current,
with prepaid labels and envelopes delivered to your door.

Built by Andrii Co. for the
[Meta AI Developer Hackathon](https://cerebralvalley.ai/e/meta-ai-developer-hackathon.md)
(Startup Track). Design and implementation happen in follow-up prompts;
this commit establishes the project scaffold, license, and working agreements.

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

Synthetic-document demo: a customer can request an apostille for Viet Nam
online in a very convenient way, and envelopes with the completed form are
delivered to their house via FedEx tomorrow.

## Repository

Rust workspace. There are no shell scripts: project tasks live in `xtask/`
and run via `cargo xtask <task>`, reusing the project's language and
libraries.

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
(Step 1: collection infrastructure).

## Prompt log

Every prompt and response is retained with timestamps in
[docs/prompts/](docs/prompts/), one Markdown file per prompt, committed
alongside the work it produced.

## License

Proprietary. Copyright (c) 2026 Andrii Co. All rights reserved.
See [LICENSE](LICENSE).
