# Apostille Handbook

Every apostille fact shows its quote, source and date — drafted by Meta AI,
approved by a human. Live at <https://andrii.ai/apostille>.

> Demo video: _no video yet — operator replaces this line with the video URL on 2026-11-03._

Built by Andrii Co. for the
[Meta AI Developer Hackathon](https://cerebralvalley.ai/e/meta-ai-developer-hackathon.md)
(Startup Track), 2026-10-26 to 2026-11-04 with Muse Code — every builder
and judge turn retained in [docs/prompts/](docs/prompts/) (see
[docs/prompts/sessions/README.md](docs/prompts/sessions/README.md)).

## Models

Every model called at runtime is a Meta model (`cargo xtask ci`
fails the build otherwise). Organizer rule, enforced in code.

| Model | Where it runs | What it does |
|---|---|---|
| `muse-spark-1.3` | Meta Model API (`https://api.meta.ai/v1`) | Drafts every proposed fact from fetched captures (quote-checked, refused quotes retried); judges photo-precheck verdicts |
| `sam-3.1` | Meta Model API (`https://api.meta.ai/v1`) | Locates document marks in precheck photos via the Responses API (`<\|box;…\|>` tokens become the overlay boxes); the public endpoint serves recorded verdicts |
| `muse-voice-transcribe-1.0` | Meta Model API (`https://api.meta.ai/v1`) | Transcribes call recordings after hangup (multipart `request` JSON + `audio` WAV); without the data-handling terms read, calls stay untranscribed |

The eval gold set is graded by deterministic checks (quote found,
value in quote, topic match) — no model grades anything.

## What is live and what is replayed

Live: drafts and eval run against the Meta API; approvals are staff
decisions in /review (deterministic quote checks only — no model call).
The public precheck endpoint serves recorded verdict JSONs verbatim —
zero marginal model cost, no key on the serving path. Replays and fixtures:
`evals/authorities/` (26-case gold set), `web/e2e/fixtures/`, crate unit
tests.

## Eval numbers (2026-11-01, production, live key)

Model muse-spark-1.3, written to the live store, shown on the home page:

| Metric | What it measures | Latest |
|---|---|---|
| `quote_found_rate` | Share of drafted records whose quote is found verbatim on the captured page | **1.0** |
| `value_in_quote_rate` | Share of records whose fact value appears inside its own quote | 0.969 |
| `topic_accuracy` | Share of records filed under the correct frozen topic | 0.641 |

Planted faults (`cargo xtask eval --planted`; deterministic, no
model): **157/158 caught** after the verifier gates (105/158
before). Per class over the 26 gold cases: value swap 26/26, span
moved 26/26, digit change 26/26, empty value 26/26 (before 0/26),
truncated value 26/26 (before 0/26), dropped "not"/"only" 1/2,
wrong topic 26/26. The one miss is `dropped_negation:vn-04`:
dropping trailing "only" from "8 Le Van Huu, Hanoi, mornings
only" leaves a string still contained in the quote, so no
substring gate can see it — that edit needs a human (or a judge
model) to catch.

Honest note: the gold set is 26 synthetic passages in
`evals/authorities/`, not live government pages — it grades the
drafter in isolation, before the quote-checker and human review add
their own gates. `topic_accuracy` understates by construction: each
gold case carries one topic, but every case drafts several records
(two from the stub, up to eight live), so at most one record per
case can match — the other drafted records count as misses even
when correctly cited.

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
the U.S., Canada, and Mexico.

## What shipped

What is live at <https://andrii.ai/apostille> today: search the
authority index; dossier pages with a step-by-step guide, cited evidence,
and live updates; sign in with passkey; run the photo precheck; save an
order draft. Staff approve every fact in the review queue and watch captures in
the trace console; approvals reach the public dossier live. Dossiers show
quote-carrying facts where approved — production's dossiers fill in as
facts are approved. The watch packs define 24 source URLs across 15
authorities (`packs/`, 9 packs); 3 authorities are loaded in production,
not 500+. What is demo-only: document photos are synthetic
samples, and there is no real mail, fees, or carrier charges anywhere.

## What's next

The live order timeline, the form PDF, and the carrier label are roadmap.
More authorities load as their packs verify; the eval gold set grows past
the first 26 synthetic passages.

## How we built it with Muse Code

One builder session ships numbered slices (`P196`, `P197`, … in
[docs/prompts/LOG.md](docs/prompts/LOG.md)); Meta-model work happens in
lane sessions on their own worktree branches, merged back by the builder.
A judge agent re-reads the live screens like a juror would. The oracle is
a script, not an opinion: `node .agents/scoreboard/scoreboard.mjs`
scores ten rows (home, dossier data and page, login, order, precheck,
eval, submission) against production and staging, and every deploy needs
its green stamp from the plant build gate. The full prompt-and-response
record sits in [docs/prompts/sessions/](docs/prompts/sessions/).

## Architecture

```text
                    ┌──────────────────────────────┐
                    │  single Rust binary (axum)   │
                    │  TLS + HTTP/3, one listener  │
                    └───────┬──────────────┬───────┘
                            │ Host         │ Host
                            ▼              ▼
                 public SPA (build/)  admin SPA (build-admin/)
                 search · dossiers    review · trace · config
                 anonymous + login    staff session + IP allowlist
                            │              │
                            └──────┬───────┘
                                   ▼
                     WS bus (authority:{slug} public;
                             proposals / run:{id} staff)
                                   │
                            SQLite (WAL) + timers
                    (captures · facts · sessions · audit)
                                   │
            plant gate ──► staging ──► production
         (full test suite)  (9/10 +   (scoreboard
                             live demo) before/after)
```

## Holds up in production

- Every deploy is scoreboarded before and after; a regression rolls
  back to the previous release, never forward into the unknown.
- Schema changes are additive-only, and a build refuses to open a
  store it cannot read — a bad migration fails closed, it never
  corrupts data.
- Public AI surfaces are rate-limited per IP and cost-capped; the
  public precheck serves recorded verdicts, so a traffic spike
  costs nothing in model calls.
- Nightly backups with a restore check; security headers (HSTS,
  same-origin framing) on every response.

## Repository

Rust workspace plus a Svelte front end (`web/`). Project tasks live in
`xtask/` and run via `cargo xtask <task>`; deployment runbooks live in
`xtask/ops/`.

```text
crates/handbook-store/   evidence store (authority / fact / capture tables, WAL)
crates/handbook-agent/   analysis runner (draft, validate, store as proposed)
crates/web/              API + WS bus + admin branch (IP-allowlisted)
web/                     public + admin SPAs (SvelteKit, adapter-static)
xtask/                   task runner (`cargo xtask ci` = fmt check, clippy, tests)
docs/prompts/            complete log of prompts and responses
AGENTS.md                working agreements for coding agents
LICENSE                  proprietary license, Andrii Co.
```

Build, test, and lint:

```sh
cargo build
cargo test
cargo xtask ci
```

## Run it yourself

One command boots the whole handbook on loopback with synthetic data —
pinned toolchains (`rust-toolchain.toml`, `.nvmrc`), both web builds, the
release binaries, a fresh `.local/` store with a file KEK, loopback
config (public `18318`, admin `18319`), every pack, captures from the
eval gold plus one stub-fixture capture per authority with drafted
proposals, recorded precheck verdicts, an eval grade, and a minted
setup key:

```sh
cargo xtask up --check   # boot, probe /healthz + authorities + setup status, exit
cargo xtask up           # same setup, then serve in the foreground
cargo xtask up --check --from-archive <url|dir> [--country ISO2]  # import a crawl package first
```

`up` prints the public/admin URLs, the one-time setup key (paste it at
`/config` to enrol staff), the loopback-only staff link for `/review`,
and try-this steps. Flags: `--fresh` wipes `.local/` first; `--live`
fetches real watch URLs and drafts/grades through the Meta Model API
with `META_API_KEY` instead of the stub. Reruns are idempotent
(lockfile-hash `npm ci` skip, capture dedup, kept KEK) except
`setup-key-mint`, which re-mints every run.

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
