# Apostille Handbook

Every apostille fact carries its quote, source and date — drafted by Meta AI, approved by a human.
Citations are off by default; turn them on with Alt+C (⌥C on a Mac) or the Citations button. Live at <https://andrii.ai/apostille>;
the Washington guide with citations on: <https://andrii.ai/apostille/authority/usa/washington?cite=1>.

> Demo video: _no video yet — operator replaces this line with the video URL on 2026-11-03._

Built by Andrii Co. for the
[Meta AI Developer Hackathon](https://cerebralvalley.ai/e/meta-ai-developer-hackathon.md)
(Startup Track), 2026-10-26 to 2026-11-04 with Muse Code.

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

## Eval numbers (2026-11-02, production, live key)

Model muse-spark-1.3, written to the live store, shown on the home page:

| Metric | What it measures | Latest |
|---|---|---|
| `quote_found_rate` | Share of drafted records whose quote is found verbatim on the captured page | **1.0** |
| `value_in_quote_rate` | Share of records whose fact value appears inside its own quote | 0.985 |
| `topic_accuracy` | Share of records filed under the correct frozen topic | 0.632 |

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
their own gates. Real cases come next: `cargo xtask gold-export
--store <backup copy>` turns accepted facts cited on official pages
into cases in the same shape (`evals/authorities-real/`), graded on
plant by an `eval.gold` job with `{"live": true, "gold":
"authorities-real"}`. Not run yet. The live-model rates above were measured
2026-11-02T17:32Z on all 30 cases (26 English, 4 Spanish/French; 68
drafted records). `topic_accuracy` understates by construction: each
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

What is live at <https://andrii.ai/apostille> today (production
`0.1.0+20abd03`, deployed 2026-11-03):

- **Every Hague party in the index:** search the country-grouped
  authority index of **510 authorities**: the 130 HCCH-party packs
  (501 competent authorities from the HCCH status table, six merged
  into the curated US dossiers) plus the curated packs, loaded from
  admin **Packs** on 2026-11-03.
- **Dossiers:** a step-by-step guide, cited evidence, a "what
  changed" panel per watched source (SA-14), and notice banners
  (`service_notice`), all updating live.
- **Sign-in and orders:** sign in with a passkey, run the photo
  precheck, save an order draft.
- **Staff tools:**
  - Approve every fact in the review queue and watch captures in the
    trace console; approvals reach the public dossier live.
  - Diff any watched source's captures.
  - Load packs.
  - Queue research and eval jobs for plant workers on the Work queue
    page (cancel, retry backoff, per-kind time limits). No job has run
    on plant yet: the worker is installed by the operator.
  - Buy a FedEx label on the staff path (live keys are operator-owned).
  - Restart the service from `/config`.
- **Caching:** every Meta API call and every downloaded page is
  recorded on disk and replayed on reruns (`docs/ops/call-cache.md`);
  only live calls count toward the daily cap.

What is demo-only: document photos are synthetic samples, and no
customer has been charged.

## Promised in our application → delivered

Promise words are quoted from the hackathon application text
(2026-10-26). "Live" means served by the production SHA; "(repo)"
means merged and shipped but waiting on an operator step or data.
Production is `0.1.0+56f0fbb`, scoreboard **8/10** measured
2026-11-04T07:47Z (`node xtask/ops/scoreboard.mjs`). The red rows:
- GP3: brand logos not placed.
- S1: demo video link and repo visibility (operator-owned; the licence part passes).

| Promised (application words) | Status | Reason / where it stands |
|---|---|---|
| Watch 500+ cited URLs, HCCH feed, inbox for authority notices | Partial | Delivered for web pages: 478 world watch URLs + 24 curated are loaded on production. The sweep keeps every download in the fetch cache, advances past dead URLs, honours weekly cadence, and spaces requests per host (P396, P402). Notices on a watched page are captured even when tiny (`gained_lines`). The email inbox is built and tested offline but off in production: no notice has gone end to end yet (Gmail credentials and the instruct timer are operator steps). |
| Muse Spark turns only changes into cited records I approve | Delivered | Changed web captures draft from their added lines only (P394, every 15 min, under `meta.daily_call_cap`). Staff approve in /review; approvals publish live. |
| Reading English, Spanish or French pages and forms | Partial | Runner drafts from es/fr pages; gold set 30 cases including 4 es/fr. Live rates on all 30 cases: see Eval numbers. Each approved fact queues es/fr translations; staff render them from /review → Translations, and the public API serves them (`/authorities/{slug}/translations?lang=es`; production has one es and one fr row for Washington so far). |
| Flag contradictions like Washington's two PO Boxes | Delivered | Conflict banner on the dossier (`crates/web/src/approval.rs`). |
| One consular legalization chain (Egypt in USA) with pre-filled authority forms | Partial | Chain view plus DS-4194 prefill. Official PDFs with fillable fields are filled and flattened from accepted form maps (`crates/web/src/forms_map.rs`). Step validation: V-19 is done (no copy routes the twelve late Apostille parties through consular legalization; a test guards it, P436); V-1/V-2 pages are watched and wait for staff-verified captures; V-3…V-18 are phone verifications (operator). |
| Walk-in reports (route photos, parking tips, rejections) update the handbook | Partial | Signed-in report form (≤3 photos, EXIF stripped). Staff turn reports into cited facts. |
| SAM 3.1 masking faces and plates | Delivered | Hosted SAM 3.1 on the precheck path, plus an open-weights SAM 3 drop-in on plant. |
| Self-hosted Muse Glimmer pre-checks document photos | Delivered (repo) | `precheck.glimmer` worker kind on plant's GPU plus `ops/open-weights/glimmer-serve.sh` (P404). Weights download from the operator-named Hugging Face repo into `/srv/scratch/apostille/models`; the kind is announced once `GLIMMER_MODEL` is set in `worker.env` (P433). Not run yet. |
| Approved changes reach the handbook and MCP server | Partial | Accepted-only publish reaches the handbook live. The read-only MCP tool `lookup_authority` is a localhost-only server (`crates/mcp`): run it beside a local store; production does not expose it. |
| A new country means new sources, not new code | Delivered | 130 country packs as data (`packs/world/`), validated in ci, loaded on production from admin Packs with no code change (510 authorities live). Non-party countries come from the `research.nonparty` worker job (P392) into `packs/world-nonparty/`. |
| One source for every country, starting 106 US/CA/MX; prepaid labels and envelopes to your door | Partial | Every Hague party is live (510 authorities). Non-parties come from the research job (not yet run). The staff FedEx path now keeps the label PDF, and the request fields are checked against FedEx's docs (P393). The first live label waits on the operator's production keys. Envelope kits are not delivered. |
| Success criterion: Viet Nam request → completed form + FedEx label, envelopes tomorrow | Partial | The form fill works and the staff label path is ready. No end-to-end Viet Nam order has been run with a real label yet. |
| Source-changed view (SA-14) | Delivered | Admin capture history plus a line diff; the public "What changed" panel shows added and removed lines per source. It needs two captures of a source, which the daily sweep produces. |

Number provenance (measured 2026-11-03 unless noted):

- Production `0.1.0+20abd03`, 8/10: measured with the scoreboard
  before and after the deploy. The count of 510 authorities is from
  the live `GET /api/v1/authorities`.
- 501 world authorities in 130 packs, 478 watch URLs: parsed from
  `packs/world/` (pinned by `world_rows_merge_into_curated_dossiers`).
- Staging with every pack loaded serves 510 authorities and passed
  9/10 (only S1) plus the demo 3/3 on each release SHA.
- Eval rates: live `/api/v1/quality`, run 2026-11-02T17:32:47Z, 30
  gold cases.

## What's next

- Run `research.nonparty` for the non-party countries and load them
  from Packs.
- Buy the first live FedEx label on the staff path, then the
  customer-paid path.
- Run Glimmer on plant once the weights are named.
- Grow the eval gold set past 30 synthetic passages.

## How we built it with Muse Code

One builder session shipped numbered slices (`P196`, `P197`, … in the
commit subjects); Meta-model work happened in lane sessions on their own
worktree branches, merged back by the builder. The release checks are a
script, not an opinion: `node xtask/ops/scoreboard.mjs` scores ten rows
(home, dossier data and page, login, order, precheck, eval, submission)
against production and staging, and every deploy needs its green stamp
from the plant build gate (`docs/ops/deploy.md`).

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
docs/ops/deploy.md       build, stage, ship, roll back, watch
LICENSE.md               PolyForm Noncommercial 1.0.0 (source code)
LICENSE-CONTENT.md       handbook content, all rights reserved
docs/licenses/           third-party BOM inputs for andrii.ai/licenses.html
```

Build, test, and lint:

```sh
cargo build
cargo test
cargo xtask ci
```

## Run it yourself

Judges: `ops/judge/preflight.sh && cargo xtask judge` on a fresh clone
(macOS or Linux, a C toolchain, Rust via rustup, Node 24, no keys). The
preflight checks what cargo needs before it compiles anything; the
doctor then checks the rest, each with one exact fix per problem. It
boots everything below with recorded Meta responses and prints a
five-minute tour. Without Rust or Node, `docker compose up --build`
serves the same boot at http://localhost:18318/apostille. Details:
[docs/judges.md](docs/judges.md).

One command boots the whole handbook on loopback with synthetic data —
pinned toolchains (`rust-toolchain.toml`, `.nvmrc`), both web builds, the
release binaries, a fresh `.local/` store with a file KEK, loopback
config (public `18318`, admin `18319`), every pack, captures from the
eval gold with drafted proposals (replayed from the recorded Muse Spark
responses; only the offline stub, without that corpus, adds one
stub-fixture capture per authority), recorded precheck verdicts, an eval
grade, a minted setup key, and a local `xtask worker` connected to the
server with a fresh token in `.local/worker.env` (0600):

```sh
cargo xtask up --check   # boot, probe /healthz + authorities + setup status, run one worker job to done, exit
cargo xtask up           # same setup, then serve in the foreground (server and worker)
cargo xtask up --check --from-archive <url|dir> [--country ISO2]  # import a crawl package first
```

`up` prints the public URL, the one-time setup key with "open `/config`
first" (paste it there to enrol staff), the review queue URL (a plain
link: it signs nobody in), and try-this steps. Flags: `--fresh` wipes
`.local/` first; `--live` fetches real watch URLs and drafts/grades
through the Meta Model API with `META_API_KEY` instead of the replay,
and refuses to start without the key; `--prebuilt` skips the toolchain
checks and builds and runs the binaries beside `xtask` (the Docker
image). Reruns are idempotent (lockfile-hash `npm ci` skip, capture
dedup, kept KEK) except two one-time values: the worker token is
re-minted every run, and so is the setup key until someone enrols (the
last key printed is the one that works). Once a passkey is enrolled, a
rerun mints no setup key: it says "staff already enrolled" and points
to `/staff-login`.
`cargo xtask setup --target local` runs the same boot with a setup
file's values on that target's `ports`, and `cargo xtask setup wizard`
opens on a fresh clone without sops or age: it keeps a plain,
local-target-only `.local/setup.yaml` (0600) until you install them for
staging and production.

## Design

See [docs/design.md](docs/design.md) — the proposed evidence-store design
(Step 1: collection infrastructure). Feature specs, visual design, and
security baseline live in [docs/specs/](docs/specs/),
[docs/site-theme.md](docs/site-theme.md), and
[docs/SECURITY.md](docs/SECURITY.md) (all drafts for review).

## License

The source code is source-available under the
[PolyForm Noncommercial License 1.0.0](LICENSE.md): free for
noncommercial use (personal, research, education, nonprofit and
government); commercial use needs a separate license. Anyone, including
a company, may clone, build, run and test it to evaluate it (hackathon
judging included). The licensing notes at the top of
[LICENSE.md](LICENSE.md) state that permission and explain that terms
for new licensees may change, that other agreements (click-wrap or
otherwise) may alter your rights, and how to get a mutual or
timestamped acceptance: email license@andrii.ai with the full license
text and your or your company's details.

The Apostille Handbook content (handbook text, data, facts, forms,
images and documentation) stays all rights reserved, except that you
may clone, build and run it with the code to evaluate it; see
[LICENSE-CONTENT.md](LICENSE-CONTENT.md).

Copyright (c) 2026 Andrii Co.

Third-party software, with versions, copyright notices and licence
texts: <https://andrii.ai/licenses.html>, generated by
`cargo xtask licenses` ([docs/licenses/README.md](docs/licenses/README.md)).

Brand font: headings use Vogun Medium (Yukita Creative), a proprietary
face licensed to Andrii Co. It is staged at build time from
`$BRAND_FONT_DIR` (see `web/scripts/brand-fonts.mjs`) and never enters
this repo. Public builds fall back to a metric-matched Source Serif 4
face (OFL, vendored in `web/static/fonts/fallback/`).
