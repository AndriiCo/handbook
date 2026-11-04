# Work queue: production work items, plant workers over WebSocket (P392)

Operator decision 2026-11-03: research, evals and scraping run as repo
code on plant workers, calling the Meta Model API (Meta models only).
Production (`andrii.ai`) keeps the pending work in SQLite; workers dial
OUT to production over one WebSocket and check jobs out. Plant never
accepts inbound connections and holds no production credentials beyond
its own worker token.

```text
staff /work page ──POST──▶ andrii.ai  work.sqlite ◀──wss── plant: cargo xtask worker
  (admin listener)          (queue + audit)          (outbound)   one job at a time
                            store blob/<sha> ◀── result upload ──┘
```

## 1. Storage: `work.sqlite`

A separate database next to `handbook.sqlite` and `p2.sqlite`. Its
tables are created at runtime with `CREATE … IF NOT EXISTS` DDL in
`handbook_store::work::migrate` (the `telemetry::migrate_tables`
pattern), never from `crates/handbook-store/migrations/`: the ship
rails refuse any release that edits that directory or a schema-version
constant, so changes here stay additive (new tables or indexes only).
Opened only through `handbook_store::work::open`.

`work_item`: `kind`, `payload_json`, `status`
(`pending | leased | done | failed`), `lease_owner`
(`<worker>:<lease token>` while leased), `lease_expires_at`,
`attempts`, `max_attempts`, the result blob ref (`result_sha256`,
`result_bytes`, `result_content_type`; bytes live content-addressed at
`<store>/blob/xx/yy/<sha>`), `summary_json`, `error`, `created_by`,
`created_at`, `updated_at`.

`audit` (append-only, triggers refuse UPDATE/DELETE) gets one row per
`work_enqueue`, `work_checkout`, `work_complete`, `work_fail`,
`work_requeue`, `work_expire`, `work_retry` and `work_stage`, committed
in the same transaction as the change. Heartbeats are not audited.

Leases: a checkout leases the oldest pending item whose kind the
worker announced, for `LEASE_SECS` (300 s), and bumps `attempts`.
Heartbeats extend an unexpired lease. Every checkout and every admin
list first sweeps expired leases: an item with attempts left goes back
to `pending`; one at `max_attempts` (default 3) becomes `failed`.
Backups: `work.sqlite` is operational state and is not in the nightly
backup set; results that matter (packs) are committed to git by the
builder.

## 2. WebSocket: `GET /apostille/api/v1/worker/ws` (public host)

Auth before upgrade: `Authorization: Bearer <token>`, compared in
constant time (SHA-256 of both sides) with the sealed secret
`worker.token` (`/config`, group "Plant workers"). No row → 503, wrong
or missing token → 401, fewer than 32 characters configured → 503.
At most 8 sockets per peer IP. Frames are JSON text; inbound frames
are capped at 24 MiB; the decoded result at `MAX_RESULT_BYTES`
(16 MiB).

Example frames: `docs/contracts/worker-frames.json` (both sides'
tests parse every sample).

| Direction | `type` | Fields |
|---|---|---|
| worker → | `hello` | `worker` (name, `[a-z0-9-]{1,32}`), `capabilities` (kinds), `version` |
| ← server | `welcome` | `worker`, `lease_secs`, `heartbeat_secs`, `max_result_bytes` |
| worker → | `checkout` | — |
| ← server | `job` | `id`, `kind`, `payload`, `lease`, `lease_expires_at`, `attempt` |
| ← server | `idle` | `retry_after_secs` |
| worker → | `heartbeat` | `id`, `lease` |
| ← server | `lease` | `id`, `lease_expires_at` |
| worker → | `complete` | `id`, `lease`, `content_type`, `sha256`, `result_b64`, `summary` |
| worker → | `fail` | `id`, `lease`, `error`, `retryable` |
| ← server | `ack` | `id`, `status` (item status after the frame) |
| ← server | `error` | `code` (`bad_frame`, `lease_lost`, `too_large`, `sha_mismatch`, `store`), `error`, `id?` |

The first frame must be `hello` within 10 s or the socket closes. The
lease token rides in `lease` so a worker that reconnects mid-job can
still heartbeat and complete (the token, not the socket, owns the
lease). The server pings every 30 s.

## 3. Admin API (admin listener, staff session + CSRF on mutations)

| Method + path | Purpose |
|---|---|
| `GET /apostille/api/v1/admin/work-items?status=&limit=&before_id=` | list + connected workers + kinds |
| `POST /apostille/api/v1/admin/work-items` | enqueue `{kind, payload, max_attempts?}` → 201 `{id, status}` |
| `GET /apostille/api/v1/admin/work-items/{id}` | one item + its audit trail |
| `GET /apostille/api/v1/admin/work-items/{id}/result` | result bytes (`Content-Disposition: attachment`) |
| `POST /apostille/api/v1/admin/work-items/{id}/retry` | failed → pending (attempts reset) |
| `POST /apostille/api/v1/admin/work-items/{id}/stage-packs` | write a done `research.nonparty` bundle's packs to `<store>/packs-inbox/<id>/` |

Shapes: `docs/contracts/admin-work-items.json`. `lease_owner` is the
worker name only; lease tokens never leave the server. Every item
carries `staged` (its inbox folder exists).

Stage as packs: only a `done` `research.nonparty` item with a result
stages (else 409). The result must be a `packs.v1` bundle whose packs
each have a unique file-safe `pack` id (`[A-Za-z0-9_-]{1,64}`, else
422); each becomes `<store>/packs-inbox/<id>/<pack>.json`, written in a
temp folder and renamed into place, so re-staging replaces the folder
whole. Full pack rules run when the staff Packs page loads the folder
(`xtask packs-load`), not here. Unknown kinds,
non-object payloads → 422; payloads over 64 KiB → 413.

## 4. Job kinds (`xtask/src/worker/`)

One file per kind implementing `JobHandler` (`kind`, `gpu`, `about`,
`example`, `run`). GPU kinds take `~/apostille-pipeline/gpu.lock`
(blocking `flock`) for the whole run, so pipeline GPU stages and
worker GPU jobs never overlap.

| Kind | Payload | Result |
|---|---|---|
| `research.nonparty` | `{countries?: [ISO2], limit?: n, refresh?: bool}` | `application/json` packs bundle (§5) |
| `discovery.bootstrap` | `{recorded?: bool, crosscheck?: bool, countries?: [ISO2] \| "all", batch_size?: 5..25, max_batches?: n}` | `application/json` discovery report (§6) |
| `scrape.urls` | `{urls: [https…], politeness_ms?: n}` | `application/json` `{pages: [{url, status, sha256, bytes, body_b64}]}` |
| `eval.gold` | `{live?: bool, planted?: bool, prompt?: "v1"\|"v2"}` | `application/json` `quality.json` |
| `precheck.glimmer` | builder-defined | stub: not announced in `hello`, so jobs wait `pending` |

## 5. `research.nonparty`

Atlas pack `crates/atlas/packs/nonparty-sources.toml` (sources, model,
two tasks with schemas). Countries = UN member states
(`evals/world-nonparty/un-member-states.html`) minus HCCH 1961 parties
(`evals/world/status-table-cid41.html`), plus Holy See and State of
Palestine. Per country:

1. `find-sources` (Spark): candidate US DOS Judicial Assistance
   country page and MFA/embassy legalization pages.
2. Fetch every candidate with `ingest::fetch` (plus the conventional
   DOS URL and the DOS page's own embassy/MFA links). A URL survives
   only when it answers 2xx and its page names the country; the DOS
   page must be on `travel.state.gov`.
3. `extract-chain` (Spark) over the surviving pages: chain steps and
   the legalizing authority, each with a verbatim quote and the URL it
   came from.
4. Quote check: every value's quote must occur in the fetched bytes of
   the URL it cites (entity-decoded, tag-stripped, lowercased,
   whitespace-collapsed, ≥ 12 chars). Anything else is dropped and
   listed in the report.
5. Pack `packs/world-nonparty/<ISO2>.json`: one `kind: country`
   authority (`<iso2>-legalization`), `unverified` domain rows, weekly
   watch seeds, `needs_review` chain steps (validation `W-8`), and
   observations for the quoted authority. Every pack passes
   `packs::validate`; slugs are checked against `packs/*.json` and
   `packs/world/*.json`.

Result bundle: `{"format": "packs.v1", "run": …, "packs": [Pack…],
"report": [{iso2, country, dos_url, mfa_urls, steps_kept,
steps_dropped, dropped: […]}]}`. `cargo xtask worker --unpack
<bundle> --into packs/world-nonparty` writes and validates the files.

## 6. `discovery.bootstrap`

First slice of from-scratch discovery. Atlas pack
`crates/atlas/packs/discovery-bootstrap.toml` (two tasks with
schemas). Writes nothing to the store or to packs: the result is a
report, and every value in it still needs staff review.

1. Countries, no model: UN member states (recorded page) plus Holy See
   and State of Palestine, joined by ISO code with the HCCH 1961 status
   table, fetched live (`recorded: true` reads the recorded copy). The
   EIF column makes each party a `party` (in force today) or a
   `future_party` (joined; legalization applies until the date).
   Status-table names this build cannot map to an ISO code are listed
   as `unmapped_parties`, never dropped. Fewer than 100 parsed parties
   fails the job (the page changed shape).
2. `hague-crosscheck` (Spark, from memory, unless `crosscheck: false`):
   the party list with EIF dates and the status-table URL, diffed
   against the table: `missing_in_model`, `extra_in_model`,
   `eif_mismatch`, `eif_unsure`. Never used as data; a model error is
   reported and does not fail the job.
3. `find-urls` (Spark, only with `countries`): world-pack authorities,
   `batch_size` per call, at most `max_batches` calls (default 15 and
   10). Fragment names (address, contact and heading lines, lower-case
   descriptions) are skipped and listed. Each proposed URL is fetched;
   it is kept only over https when its text names the authority (2+ of
   its 5+-letter words) or an apostille term, with an `official_hint`
   (`listed_by_hcch` or `government_domain`). Bad country codes fail
   before any model call; if every batch fails at the model, the job
   retries.

Result: `{"format": "discovery.v1", "today", "hcch": {url, recorded,
sha256}, "countries": [{iso2, name, status, entry_into_force}],
"future_parties", "unmapped_parties", "crosscheck", "authorities":
[{authority, unknown, urls: [{kept, candidate, evidence |
reason, official_hint}]}], "names_skipped", "summary"}`. Offline run:
`cargo xtask worker --run-local <job.json>` with `META_API_KEY` in the
env file.
