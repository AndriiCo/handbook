# Atlas harness (convention-source research worker)

Parent question: who scans the Apostille Convention's own sources
(HCCH status table, footnotes, per-country restrictions, objections,
bilateral non-requirement agreements) and turns them into
jurisdiction data + slugs? Today nothing does: `xtask hcch-poll`
only change-detects the status table, `ingest::hcch` only routes
from already-known rows. This spec defines the **harness** that
runs such workers — rerunnable, debuggable, model-backed. Slugs,
prompts, and jurisdiction content are task-pack data, not this
spec and not this code.

## 1. Non-goals

- No slug/content definitions here (packs carry them).
- No store writes: the harness proposes versioned JSON under
  `out/` + a JSONL ledger. Promotion into captures/facts is a
  separate, reviewed step (same rule as the S2 runner: never
  auto-accept).
- No new crypto, no PII: Contributor-mode models are opt-in per
  task and forbidden for customer media (see §5).

## 2. Pipeline

```text
sources.toml ──▶ fetch+cache ──▶ extract ──▶ chunk ──▶ summarize ──▶ task ──▶ out/*.json
  (pack)          (etag, sha)     (text/html/   (budgeted,   (map-reduce,   (structured   + ledger.jsonl
                                   pdf/svg/     offset maps)  cached)       + verify)
                                   audio)
```

Every stage is content-addressed and resumable: re-running with
unchanged inputs reuses cache and prints what it skipped. `--dry-run`
prints projected calls/tokens/cost without touching the network
(except explicit `--check`).

## 3. Model surface (Meta Model API, pinned 2026-10-29)

- **Fast**: sync `POST /v1/chat/completions`, `json_schema`
  structured output, `prompt_cache_key`, `reasoning_effort`
  (never `none` on Spark), retry 429/5xx with backoff. Shapes:
  `docs/meta-api-shapes.md`.
- **Batch**: managed bulk over `POST /v1/responses` with
  `background: true` (ack `{status: queued}`), poll `GET
  /v1/responses/{id}`, cancel, 1 MiB creation cap, 600
  submissions/min/team. Requests persist as local JSONL datasets
  (`purpose=batch`-compatible) so a run resumes after Ctrl-C.
  No native batch-execution endpoint is documented upstream; the
  `BatchTransport` trait reserves the slot for one.
- **Files**: `POST /v1/files` upload (1 GiB), `file_id` refs,
  `DELETE` after the run. Inline `file_data`/`file_url` under
  50 MB. Server-side PDF handling: text + per-page images
  (≤50 pages full; text to 100, images to 50 beyond).
- **Voice notes**: `POST /v1/asr/transcribe` (multipart,
  turn-level transcript, no word timestamps) for local audio;
  or `input_audio` blocks on Responses for short clips.
- **Budgets**: estimate locally (chars/4), then
  `POST /v1/responses/input_tokens` to measure before sending
  when live. Context window, RPM/TPM, and $/1M come from the
  pack's model table — never hardcoded.

## 4. Task packs (TOML)

A pack names sources, models, effort, and tasks. One task =
selector + instructions + JSON schema + effort + model +
protocol + verify policy. Example skeleton (prompts are pack
data; the harness never invents them):

```toml
[run]
out = "out/atlas"
effort = "standard"          # low | standard | high | max

[[models]]
name = "spark"               # task-local alias
id = "muse-spark-1.3"        # -contributor suffix selects Contributor mode
mode = "standard"            # standard | contributor (trains on content!)
context_window = 1048576
rpm = 3000
tpm = 4000000
input_per_1m = 0.0           # $/1M; fill from pricing page
output_per_1m = 0.0

[[sources]]
name = "hcch-status"
url = "https://www.hcch.net/…/status-table"
kind = "html"                # text | html | pdf | svg | audio | image
refresh = "weekly"

[[tasks]]
name = "membership-scan"
model = "spark"
protocol = "fast"            # fast | batch | auto
effort = "standard"
instructions = "…pack-owned prompt…"
schema = "schemas/membership.json"
verify = "none"              # none | self | second-model
allow_contributor = false
```

Effort presets (overridable per task): `low` (8k-token chunks,
one summary level, no verify), `standard` (16k, two levels),
`high` (32k + verify), `max` (verify on the strongest model).
`protocol = auto` picks batch past a request-count threshold.

## 5. Safety rules

- Contributor mode only when the task sets
  `allow_contributor = true`; default false. Customer PII
  never leaves on contributor calls (same bar as the analysis
  agent).
- API key from sealed config (`meta.api_key`) or `META_API_KEY`
  env; redacted in Debug, ledger, and logs.
- External extractors (PDF CLI fallback) run with argv from the
  pack, piped stdio, byte caps, and timeouts — never a shell.

## 6. CLI surface

```text
cargo xtask atlas --config packs/x.toml --task membership-scan [--effort …]
                  [--model spark] [--protocol fast|batch|auto] [--out …]
                  [--refresh] [--limit N] [--dry-run] [--resume <run-id>]
cargo xtask atlas --batch-poll <job-id> [--config …]   # resume a batch job
cargo xtask atlas --check                               # key + GET /models
```

Exit codes: 0 green, 1 failed (reason on stderr), 2 usage/config
error. `--dry-run` is network-free.

## 7. Outputs

- `out/<run-id>/result.<task>.json` — schema-validated proposal.
- `out/<run-id>/ledger.jsonl` — one line per model call (model,
  mode, effort, protocol, input sha, tokens, cached tokens,
  $ estimate, ms, output path).
- `out/<run-id>/debug/` — fetched bytes meta, chunks, summaries,
  requests, raw responses (keyed by content hash).
- `out/<run-id>/status.html` + `status.svg` — rendered status
  table + SVG grid from result rows (world-map geometry is pack
  content, not harness code).

## 8. Testing

Unit tests run offline via `StubTransport` (canned HTTP) and
`StubClient` (canned model): chunk budgets, summary merge,
batch resume, config parse, extractor goldens, renderer
goldens. Live API is exercised only by `--check` and explicit
`--live` runs, never by `cargo test`.
