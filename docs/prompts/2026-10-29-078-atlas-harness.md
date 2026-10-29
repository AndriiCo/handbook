# Prompt 078 — Atlas harness: rerunnable convention-source research worker

- Prompt (UTC): 2026-10-29
- Response (UTC): 2026-10-29
- Author: Andrii (operator)
- Agent: Muse Code (solo inline; lookups + build, no subagents)

## Prompt

```text
Do we have code for worker that scans the source of Apostille conversion members countries, their footnoes and restrictions of each country membership (dates, objects to others countries, maybe bi-lateral agreemnts that make apostille not required, etc.). Can this worker be tasked to also define slugs for all jurisdictions (like usa to have states, federal and for some states to have county level offices / slugs. etc). Can it be a code that possible to rerun and debug, not one-off from this session, but that code to utilize Meta Modela API to reason about data based on inputs. Make consirations for max context size by using summarization (or others techologies).  You task to is code  harness for this type of work, not to define slugs / content. Make it configurable to be able comfigure various effort levels, models (contributor vs. not), batch vs. fast APIs, etc.  Code to be canonical Rust. Use best practiced from Workshops from https://dev.meta.ai/docs/overview  Make sure it's multi-modal and supports PDF, voice notes, SVG maps for drawring status table code.
```

## Response

Answer to "do we have this": no. `xtask hcch-poll` only
change-detects the status table, `ingest::hcch` only routes from
already-known rows; nothing extracts footnotes/objections/
bilaterals, defines slugs, or runs multimodal model jobs. Built
the harness as new crate `atlas` (+ `xtask atlas`), spec-first
per repo rules. No slugs/content defined — packs carry those.

What was built (spec: `docs/specs/atlas-harness.md`):

- `crates/atlas`: `config` (TOML packs: models/effort/sources/
  tasks, Contributor opt-in enforced), `source` (fetch + etag
  disk cache, reruns skip network), `extract` (text/HTML-table/
  SVG-labels locally; PDF API-native via `input_file` with
  external-tool fallback; audio/image sniffed for native
  paths), `chunk` (budgeted, byte-offset citations), `summarize`
  (map-reduce + content-addressed file cache), `model`
  (MetaClient: sync chat `json_schema`, Responses incl.
  `background` + `text.format`, Files upload/delete,
  `input_tokens` measure-before-send, `/asr/transcribe`,
  retry/backoff per shapes doc), `batch` (persisted JSONL
  datasets + paced submit/poll/resume over background
  Responses), `task` (orchestrator: transcribe → chunk →
  summarize → budget gate → fast/batch extract → verify →
  result + debug dumps + ledger), `ledger` (JSONL cost/
  provenance), `render` (result `{columns,rows}` → HTML table
  + SVG grid).
- `xtask atlas`: `--config --task` with `--effort/--model/
  --protocol/--out/--refresh/--limit/--live/--dry-run/
  --run-id/--pdf-tool`, `--batch-poll`, `--check`. Stub port
  by default; live key from sealed `meta.api_key` or
  `$META_API_KEY`.
- Example pack + synthetic HTML fixture + schema
  (`crates/atlas/packs/`), runnable offline via `--dry-run`.
- `docs/meta-api-shapes.md`: pinned Responses background,
  `text.format`, Files, `input_tokens`, PDF handling
  (verified 2026-10-29 from dev.meta.ai).

Doc-grounded decisions: "contributor vs not" = Standard/
Contributor mode via `-contributor` model ids (default deny,
per-task opt-in; never customer PII); "batch vs fast" =
background Responses jobs vs sync chat (no native batch-
execution endpoint is documented — `/docs/batch` 404s — so
batch is harness-managed with a reserved `BatchTransport`
trait); PDF understanding is server-side (`input_file`,
text+images to 50 pages); voice via the pinned transcribe
endpoint (turn-level, no word timestamps); context via
estimate→measure→chunk→cached-summarize→verify-fit.

Assumptions: TOML packs (dep resolved offline); no store
writes (proposals + ledger under `out/`, promotion is a
separate reviewed step); world-map geometry is pack content,
not harness code; image/audio Responses block fields flagged
re-verify at first live use.

Files changed: `crates/atlas/**` (new, 11 modules + packs),
`xtask/src/atlas.rs` (new), `Cargo.toml`, `Cargo.lock`,
`xtask/Cargo.toml`, `xtask/src/main.rs`,
`docs/specs/atlas-harness.md` (new),
`docs/meta-api-shapes.md`.

Verification: `cargo test -p atlas` 33 pass (offline stubs);
example pack `--dry-run` prints plan (98 est tokens, 2 calls,
$0.00); stub end-to-end writes result + ledger + debug dumps
under `/tmp/atlas-stub/demo/`; full `cargo xtask ci` exit 0
(fmt, clippy `-D warnings`, 38 suites incl. atlas, playwright,
tailwind). No live API calls made (no key in this env).

## Commits

- `5c103d9` Atlas harness: rerunnable convention-source research worker
  (crate, xtask, spec, shape pins; this log rode along)
- (this hash fill-in, committed immediately after)

## Follow-ups

- First live run: `atlas --check`, then `--live` on the
  example; re-verify `output_text` walk, `image_url` field,
  upload `purpose` value against real responses.
- Pack authorship (content, not harness): membership-scan
  prompts/schema, slug-definition task, HCCH source URLs.
- Native batch endpoint: re-check upstream docs; slot behind
  `BatchTransport` when documented.
- Promotion path: reviewed step from `out/*.json` into
  captures/proposed facts (out of scope here).
