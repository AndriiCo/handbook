# Admin workers management (goal-025 stream 1)

Draft for author review. Staff-only UI + API to manage the URL
download/analysis ingest paths: list/retry captures, worker status,
manual watch-sweep, per-authority ingest state. Spec-first per the
stream brief; implementation follows §5.

## 0. Non-goals (frozen for this stream)

- No new worker runtime: no background threads, no scheduler, no
  queue tables. Every fetch runs synchronously inside the staff
  request that triggered it (in `spawn_blocking`, since
  `ingest::fetch` is blocking and `rusqlite::Connection` is `!Send`).
- No new WS topics: the topic enum is unioned and frozen (specs
  README resolution 1/5). All state here is request/response.
- No server-side watchlist/schedule: sweep items are explicit per
  call. A persisted watchlist + cadence is a follow-up, not this
  stream.
- No changes to `xtask fetch-url`: the admin API reuses the same
  `ingest::{fetch,capture}` + `handbook-store` primitives; the small
  orchestration (ensure-source, record-failure) is duplicated, not
  extracted, to keep the diff surgical. Follow-up: extract a shared
  `ingest::watch` module and port `xtask fetch-url` onto it.
- Fixtures are synthetic only (`example.com` / `synthetic-NN`
  shapes); canonical values never leak into facts.

## 1. API routes

All on the ADMIN listener only, under `/apostille/api/v1/admin`,
behind the existing recipe (spec: `spa-separation.md` §1.4;
code: `crates/web/src/admin.rs`): admin host guard, staff-role
`AdminAuth` session (+ mTLS seam), CSRF Origin + double-submit on
mutations. Error shape is `{"error": "..."}`; auth failures are
401/403 from the shared guards, never per-handler.

| Method + path | Auth | Purpose |
|---|---|---|
| `GET /workers/status` | staff | §2 worker status |
| `GET /workers/captures` | staff | §3 capture list |
| `POST /workers/captures/{id}/retry` | staff + CSRF | §3 re-fetch one capture's source URL |
| `POST /workers/fetch` | staff + CSRF | §4 one-off fetch of an explicit URL |
| `GET /workers/authorities` | staff | §5 per-authority ingest state |
| `POST /workers/sweep` | staff + CSRF | §6 manual watch-sweep (≤ 10 URLs) |

`{id}` is the integer capture id. Unknown ids → 404; a capture
with no source row → 409 (nothing to re-fetch).

## 2. Worker status

`GET /workers/status` → 200:

```json
{
  "mode": "manual",
  "fetch": {"backend": "ureq", "timeout_secs": 20,
            "max_body_bytes": 8388608},
  "queue": {"captures": 12, "sources": 10,
            "open_fetch_failures": 2},
  "last_sweep": {"at": 1800000100, "actor": "staff-1",
                 "summary": "..."} 
}
```

- `queue.open_fetch_failures`: live (`open`/`batched`/
  `in_progress`) `validate_fact` tasks titled `fetch failed: …`.
- `last_sweep`: latest `audit` row with `action='watch_sweep'`,
  or `null` when no sweep has ever run.

## 3. Captures: list + retry

`GET /workers/captures?authority=&kind=&trust=&limit=&before_id=`
→ 200 `{"captures": […], "count": n}`. Newest first (`id DESC`);
`before_id` pages (`id < before_id`); `limit` defaults 50, max
200. Row shape:

```json
{"id": 7, "authority_slug": "usa-wa-sos",
 "source_uri": "https://example.com/synthetic/1",
 "captured_at": 1800000100, "kind": "warc",
 "bytes_raw": 128, "sha256": "…", "trust": "official",
 "notes": "…"}
```

`source_uri` is `null` when the capture is unattributed.
`authority` must be a known slug (unknown → 404), `kind`/`trust`
filter verbatim.

`POST /workers/captures/{id}/retry` re-drives the existing
fetch path for the capture's source URL: robots re-check for
`website_thirdparty` sources (verdict persisted on the source row,
F1 rule), `fetch_url` (20 s timeout), `ingest_bytes` with the
capture's kind/trust, actor = staff user id. Audited as
`worker_retry` (`before_ref` = retried id, `after_ref` =
`created|duplicate:{id}`). Responses:

- 200 `{"outcome": "created"|"duplicate", "capture_id": n,
  "status": 200}` — fetch failures are never silent: on fetch
  error the handler records/refreshes the `fetch failed: …` task
  (same dedup key as `xtask fetch-url`) and answers 502
  `{"error": "…", "task": "recorded"}`.
- 200 `{"outcome": "skipped", "reason": "robots disallow"}` when
  the robots gate blocks (task recorded, as in `xtask`).

## 4. One-off fetch

`POST /workers/fetch`, body (unknown fields rejected):

```json
{"slug": "usa-wa-sos", "url": "https://…",
 "source_kind": "website_official", "trust": "official"}
```

`source_kind` ∈ `website_official | website_thirdparty`
(default `website_official`); `trust` ∈ `official |
staff_verified | community` (default `official`); `url` must be
`http(s)` (else 422); unknown slug → 404. Behavior mirrors
`xtask fetch-url`: find-or-create the `source` row, robots gate
for third-party, fetch, ingest, audited as `worker_fetch`.
Same 200/502/skipped shapes as retry (§3).

## 5. Per-authority ingest state

`GET /workers/authorities` → 200
`{"authorities": […], "count": n}`, ordered by slug:

```json
{"slug": "usa-wa-sos", "name": "WA Secretary of State",
 "sources": 3, "captures": 5, "last_capture_at": 1800000100,
 "open_fetch_failures": 1, "last_analysis_at": 1800000200,
 "last_prompt_version": "v3"}
```

`last_capture_at` is `null` with zero captures;
`last_analysis_at`/`last_prompt_version` come from the
`watermark` row (`null` when never analyzed).

## 6. Manual watch-sweep

`POST /workers/sweep`, body (unknown fields rejected):

```json
{"items": [{"slug": "usa-wa-sos",
            "url": "https://example.com/synthetic/1"}]}
```

1–10 items (empty or > 10 → 422); each item validated like §4
(first bad item fails the call, 404/422, nothing fetched).
Items run sequentially in one blocking task (20 s timeout
each, worst case ~200 s + ingest — staff-only, no client
timeout in the admin shell). Per-item result:

```json
{"slug": "…", "url": "…", "outcome": "created"|"duplicate"|
 "failed"|"skipped", "capture_id": 7, "detail": "…"}
```

`capture_id` is present only for `created`/`duplicate`;
`detail` carries the failure/skip reason otherwise (fetch
failures record tasks, same as §3). The run writes one
`worker_sweep`-family audit row: action `watch_sweep`, reason =
`ok/total` summary. Response 200:

```json
{"results": […],
 "summary": {"total": 2, "created": 1, "duplicate": 0,
             "failed": 1, "skipped": 0}}
```

## 7. WS topics

None. The frozen topic enum gains no variant; sweep/retry
results return inline in the POST responses. If a future stream
needs progress events, it must go through the topic-freeze
exception process (specs README resolution 5), not this API.

## 8. UI screens

One staff page (admin build only): `/workers`
(`web/src/routes/(admin)/workers/+page.svelte`), linked from
the staff nav as “Workers”. Sections, top to bottom:

1. **Status** — mode, fetch backend/timeout/body cap, queue
   counts, last sweep (or “never run”).
2. **Authorities** — per-authority ingest table (§5).
3. **Captures** — filterable list (§3) with a Retry button per
   row; retry result shown inline per row.
4. **Fetch** — form (slug, URL, source kind, trust) driving §4.
5. **Sweep** — textarea of `slug | url` lines (≤ 10) driving §6;
   per-item outcomes listed after the run.

All mutations send `credentials: same-origin` + the
double-submit CSRF headers (`lib/config-client.js`
`csrfHeaders()`), and surface 401/403 as “staff session
expired — log in again”. No secrets or raw bytes rendered;
sha256 shown in full (public dedup key, not sensitive).

## 9. Acceptance criteria

1. Staff `GET status` returns §2 shape; `last_sweep` is null
   before any sweep.
2. Captures list filters/paginates per §3; unknown authority →
   404.
3. Retry of a capture whose URL serves bytes creates (or
   dedups to) a capture and audits `worker_retry`.
4. Retry/fetch of a dead URL answers 502, records (or
   refreshes) the `fetch failed: …` task, audits nothing as a
   capture.
5. Robots-blocked third-party retry answers `skipped` and
   persists the verdict on the source row.
6. One-off fetch validates slug/URL/enums (404/422) and mirrors
   `xtask fetch-url` outcomes.
7. Authorities endpoint returns §5 shape incl. watermark data.
8. Sweep validates items (empty/>10/bad → 422/404, nothing
   fetched), runs sequentially, audits one `watch_sweep` row,
   returns per-item + summary shapes.
9. Guards: no session → 401; non-staff role → 403; wrong host →
   403; mutation without Origin/CSRF pair → 403. All from the
   shared recipe, not per-handler checks.
10. Admin bundle contains the Workers page and API markers;
    public bundle contains neither (smoke gate).
11. No new WS topics; no background runtime; no `xtask`
    behavior change.

## 10. Open questions

- OQ-W1: should sweep grow a persisted server-side watchlist +
  cadence (cron/systemd timer), or stay manual? (Blocks nothing
  here; decision needed before any scheduler work.)
- OQ-W2: extract shared `ingest::watch` orchestration with
  `xtask fetch-url` now or after sibling streams land? Current
  call: after, to avoid cross-stream churn.
