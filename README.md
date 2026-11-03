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
| `facebook/sam3` | plant RTX 3060, home venv (`ops/open-weights/sam3_boxes.py`) | Open-weights SAM 3 drop-in for the A2 recorder (`--sam-backend open-weights`, optional `--mask faces,license plates`); emits the same box shape as the hosted call, Muse Spark still judges; weights gated behind the Meta licence, never committed |
| `muse-voice-transcribe-1.0` | Meta Model API (`https://api.meta.ai/v1`) | Transcribes call recordings after hangup (multipart `request` JSON + `audio` WAV); without the data-handling terms read, calls stay untranscribed |

The eval gold set is graded by deterministic checks (quote found,
value in quote, topic match) — no model grades anything.

## What is live and what is replayed

Live: drafts and eval run against the Meta API; approvals are staff
decisions in /review (deterministic quote checks only — no model call).
The public precheck endpoint serves recorded verdict JSONs verbatim —
zero marginal model cost, no key on the serving path. Replays and fixtures:
`evals/authorities/` (30-case gold set), `web/e2e/fixtures/`, crate unit
tests.

## Eval numbers (2026-11-01, production, live key)

Model muse-spark-1.3, written to the live store, shown on the home page:

| Metric | What it measures | Latest |
|---|---|---|
| `quote_found_rate` | Share of drafted records whose quote is found verbatim on the captured page | **1.0** |
| `value_in_quote_rate` | Share of records whose fact value appears inside its own quote | 0.969 |
| `topic_accuracy` | Share of records filed under the correct frozen topic | 0.641 |

Planted faults (`cargo xtask eval --planted`; deterministic, no
model; re-measured 2026-11-02 on the 30-case set): **181/182
caught** after the verifier gates (121/182 before). Per class over
the 30 gold cases: value swap 30/30, span moved 30/30, digit change
30/30, empty value 30/30 (before 0/30), truncated value 30/30
(before 0/30), dropped "not"/"only" 1/2, wrong topic 30/30. The one
miss is `dropped_negation:vn-04`:
dropping trailing "only" from "8 Le Van Huu, Hanoi, mornings
only" leaves a string still contained in the quote, so no
substring gate can see it — that edit needs a human (or a judge
model) to catch.

Honest note: the gold set is 30 synthetic passages in
`evals/authorities/`, not live government pages — it grades the
drafter in isolation, before the quote-checker and human review add
their own gates. The live-model rates above were measured 2026-11-01
on the then-26 English-only cases; four Spanish/French cases landed
2026-11-02 and live rates on the full set are not yet re-measured. `topic_accuracy` understates by construction: each
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

## Promised in our application → delivered

Promise words are quoted from the hackathon application text
([001 — Initial scaffolding](docs/prompts/2026-10-26-001-initial-scaffolding.md),
2026-10-26). No separate "review §14" promise file exists in the
repo — the push-plan's §14 list points back to this application
text, so it is the source used here. Statuses marked "live" are
served by the production SHA; "(repo)" means merged, not claimed
live. Production is `0.1.0+2b4bc8d`, scoreboard **5/10** measured
2026-11-02T17:32Z (`node .agents/scoreboard/scoreboard.mjs`).
GP2-data is red: Vietnam 0/8, usa-wa-sos 1/8 accepted facts —
approvals are operator-owned.

| Promised (application words) | Status | Reason / where it stands |
|---|---|---|
| Watch 500+ cited URLs, HCCH feed, inbox for authority notices | Partial | Thin slice live: `packs/` define 24 source URLs across 15 authorities (9 packs, HCCH pack included); 3 authorities loaded in production. Full 500-URL crawl + F3 inbox are roadmap (Week 3+). |
| Muse Spark turns only changes into cited records I approve | Delivered (repo) | Draft → validate → store-as-proposed; staff approve in /review with audit + conflict banner; approvals publish live. Production approvals pending (see GP2-data above). |
| Reading English, Spanish or French pages and forms | Partial | Runner drafts from es/fr pages (SA-16); gold set grew 26 → 30 with 4 es/fr cases. Es/fr machine translation after approval is merged (SA-20) but roadmap until wired — not claimed live. Live-model rates below were measured on the 26 English-only cases; full-30 re-measure pending. |
| Flag contradictions like Washington's two PO Boxes | Delivered (repo) | Conflict policy is the banner (`crates/web/src/approval.rs`); canonical PO-Box fixtures seed the case in tests. |
| One consular legalization chain (Egypt in USA) with pre-filled authority forms | Partial (repo) | Read-only chain view (`ChainView`, `needs_review` chip per step, `packs/egypt-consular.json`) + DS-4194 prefill from the synthetic order (SA-18). Step validation (V-1…V-19) is roadmap. |
| Walk-in reports (route photos, parking tips, rejections) update the handbook | Partial (repo) | Signed-in report form (≤3 photos, EXIF stripped) + report tables merged; staff turn reports into cited facts (SA-21). |
| SAM 3.1 masking faces and plates | Delivered | Hosted SAM 3.1 via the Meta Model API on the precheck path; open-weights SAM 3 drop-in on the plant RTX 3060 for the A2 recorder (+ optional face/plate mask). Weights licence-gated, never committed (SA-6). |
| Self-hosted Muse Glimmer pre-checks document photos | Descoped | Honest substitute: the public precheck serves recorded verdict JSONs verbatim — zero marginal model cost. Glimmer self-hosting not built. |
| Approved changes reach the handbook and MCP server | Delivered (repo) | Accepted-only publish; read-only MCP stub with one tool (`lookup_authority`, `crates/mcp`). |
| A new country means new sources, not new code | Delivered (repo) | 106 authorities staged as data (`packs/world/US.json` 59 + `MX.json` 33 + `CA.json` 14, all `needs_review`); validated by `cargo xtask packs-validate` (SA-17). |
| One source for every country, starting 106 US/CA/MX; prepaid labels and envelopes to your door | Partial | Authority data staged (above); 3 authorities loaded in production, more load as packs verify. No real mail, fees, or carrier charges anywhere. |
| Success criterion: Viet Nam request → completed form + FedEx label, envelopes tomorrow | Roadmap | Live order timeline, form PDF fill and carrier postage are roadmap. Live FedEx endpoints merged behind `labels.live` (off by default; caps 2/day, $60 max, ZIP allowlist, staff-only, audited — SA-19); credentials + allowlist are operator-owned. |
| Source-changed view (SA-14) | Parked | Needs ≥2 captures of one source; production holds at most one per source (carried from the slice report, not independently verified from this worktree). |

Number provenance (measured this turn vs carried):

- Production `0.1.0+2b4bc8d`, 5/10 — measured (`scoreboard.mjs` full run, 2026-11-02T17:32Z).
- Eval rates — measured from live `/api/v1/quality` this turn: model `muse-spark-1.3`, run `2026-11-01T04:29:10Z`, 26 gold cases at run time, `quote_found_rate` 1.0, `value_in_quote_rate` 0.96875 (shown 0.969), `topic_accuracy` 0.640625 (shown 0.641). Match the README's cited measurement.
- Gold set now 30 (13 vietnam + 13 usa-wa-sos + 2 mexico-sre + 2 france-legalisation) — counted this turn.
- 106 authorities = 59 + 33 + 14 — parsed from the pack files this turn.
- FedEx caps (2/day, $60, off by default) — read from `crates/config/src/lib.rs` defaults this turn.
- Planted faults 181/182 — carried from the README's cited 2026-11-02 re-measurement; not re-run this turn.
- S1 submission row stays honest: the source code is source-available under PolyForm Noncommercial 1.0.0 (not an OSI licence, so the row's MIT/Apache check stays red), repo visibility is the operator's call, and the video line is operator-owned.

## What's next

The live order timeline, the form PDF, and the carrier label are roadmap.
More authorities load as their packs verify; the eval gold set grows past
the first 30 synthetic passages.

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
LICENSE                  PolyForm Noncommercial 1.0.0 (source code)
LICENSE-CONTENT.md       handbook content, all rights reserved
```

Build, test, and lint:

```sh
cargo build
cargo test
cargo xtask ci
```

## Run it yourself

Judges: `cargo xtask judge` on a fresh clone (macOS or Linux, Rust +
Node 24, no keys). It checks your prerequisites and prints one exact fix
per problem, boots everything below with recorded Meta responses, and
prints a five-minute tour. `cargo xtask judge --doctor` runs only the
checks. Details: [docs/judges.md](docs/judges.md).

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

The source code is source-available under the
[PolyForm Noncommercial License 1.0.0](LICENSE): free for
noncommercial use (personal, research, education, nonprofit and
government); commercial use needs a separate license. The licensing
notes at the top of [LICENSE](LICENSE) explain that terms for new
licensees may change, that other agreements (click-wrap or otherwise)
may alter your rights, and how to get a mutual or timestamped
acceptance: email license@andrii.ai with the full license text and your
or your company's details.

The Apostille Handbook content (handbook text, data, facts, forms,
images and documentation) stays all rights reserved; see
[LICENSE-CONTENT.md](LICENSE-CONTENT.md).

Copyright (c) 2026 Andrii Co.

Brand font: headings use Vogun Medium (Yukita Creative), a proprietary
face licensed to Andrii Co. It is staged at build time from
`$BRAND_FONT_DIR` (see `web/scripts/brand-fonts.mjs`) and never enters
this repo. Public builds fall back to a metric-matched Source Serif 4
face (OFL, vendored in `web/static/fonts/fallback/`).
