# Call cache: Meta calls and URL fetches (P391)

Operator decision 2026-11-03: every URL fetched and every Meta API call
is saved to disk and reused on reruns, so retries and revisions never
re-spend budget. Code: `crates/callcache` (Rust) and the matching
`MetaCache` class in `xtask/ops/a2-record.py` (same key, same layout).

## Where it lives

| | Plant (`chandra`) | Production (`andrii-ai`) |
|---|---|---|
| Meta calls | `/srv/scratch/apostille/meta-cache` | `/var/lib/apostille/meta-cache` |
| URL fetches | `/srv/scratch/apostille/fetch-cache` | `/var/lib/apostille/fetch-cache` |

Resolution, lowest to highest precedence:

1. default under the store dir: `<store>/meta-cache`, `<store>/fetch-cache`
   (production's store is `/var/lib/apostille`, so production needs no
   setting; the unit's `ReadWritePaths` already covers it);
2. sealed config: `paths.meta_cache_dir`, `paths.fetch_cache_dir`,
   `meta.cache_mode`, `meta.daily_call_cap` (read per call: no restart);
3. environment: `META_CACHE_DIR`, `FETCH_CACHE_DIR`, `META_CACHE_MODE`,
   `FETCH_CACHE_MODE`, `META_DAILY_CALL_CAP`;
4. xtask flags: `--meta-cache`, `--fetch-cache`, `--cache-mode`,
   `--fetch-cache-mode`, `--daily-call-cap`.

On plant, either export the env pair once per shell or pass the flags:

```sh
export META_CACHE_DIR=/srv/scratch/apostille/meta-cache
export FETCH_CACHE_DIR=/srv/scratch/apostille/fetch-cache
```

Directories are created owner-only (0700): cached answers include model
output from customer material (drafts, voice transcripts). Whichever
user first writes a cache dir owns it, so run CLI jobs on production as
the service user, or give them their own `--meta-cache`. The A2 ops
wrapper (`xtask/ops/ops-a2-record.sh`, run as `hal`) uses
`$HOME/apostille-cache/meta-cache` for that reason.

## Modes

| mode | hit | miss |
|---|---|---|
| `record` (default) | stored bytes, no network | call live, store the answer |
| `replay-only` | stored bytes | error; nothing touches the network |
| `refresh` | ignored: always call live | call live; overwrite |

`--cache-mode` / `meta.cache_mode` sets both caches; `--fetch-cache-mode`
overrides the fetch cache alone. Watch sweeps want the fetch cache in
`refresh` (archive every fetch, still see changes); a rerun from cache is
`replay-only`.

## Meta call cache

Key: hex sha256 of `meta-cache/v1\n<METHOD> <url path>\n<canonical body>`.
The canonical body is the request JSON with sorted keys and no
whitespace (Python: `json.dumps(v, sort_keys=True, separators=(",", ":"),
ensure_ascii=False)`); it carries the model id. Multipart requests hash
their text fields (wire order) plus the file part's field, content type
and sha256; the filename is left out so temp names still hit. The host is
left out, so a staging stub and production share recordings.

```
meta-cache/
  v1/<k[0..2]>/<key>.body    raw response bytes (SSE or JSON) as read
  v1/<k[0..2]>/<key>.json    {v, key, method, path, model, status,
                              stored_at, sha256, len} (written last)
  ledger/<YYYY-MM-DD>.jsonl  one line per live attempt (UTC day)
```

Only 2xx answers are stored: 429/5xx are transient and a 4xx is worth
re-asking once the request or key is fixed. Parsers run unchanged on a
hit (the stored bytes are the wire bytes).

Seams: `MetaRestClient::post_chat` (every web pipeline, `xtask analyze`,
`eval`), the atlas `CachedTransport` (chat, `/responses`, token counts,
`/files` uploads for at most their server-side expiry minus an hour,
`/asr/transcribe`; terminal `GET /responses/{id}` polls), the media
`CachedTransport` (voice transcribe, SAM), the web translate client,
`xtask world-index --live-model` (curl), `xtask ask`, and
`a2-record.py`. `xtask atlas` and `xtask instruct` get it through
`xtask::atlas::live_client`, so any worker built on that client does too.

### Daily cap

`meta.daily_call_cap` (default 400) is enforced at the transport for
every path above: each live attempt (retries included) is admitted
against and appended to `ledger/<day>.jsonl`; cache hits never write a
line and never produce a `model_call` row. With N calls in flight the
day can overshoot by at most N. Web handlers keep their own
`model_call`-row gate as well.

### Replay corpus (`evals/replay/drafts-spark.jsonl`)

`handbook_agent::replay` keys now cover the model id and the sha256 of
the prompt template (`replay-key/v2`). New lines carry `"kv": 2`; older
lines are re-keyed on load from their own `model` and `prompt` columns,
so the committed corpus keeps working.

## URL fetch cache

```
fetch-cache/
  blobs/<sha[0..2]>/<sha256>  body bytes, once per digest
  manifest.jsonl              {url, fetched_at, status, headers: [[k, v]…],
                               sha256, len[, head_sha256, request_sha256]}
```

Newest line per URL wins. `record` serves the newest answer unless it was
transient (429/5xx) or a bodiless 304; error statuses such as 404 are
answers and replay as such. Users: `ingest::fetch::fetch_url_cached` /
`fetch_url_conditional_cached` (for the sweep), `xtask crawl` (every hop
with raw request and response head, plus robots.txt; robots are always
re-checked live outside `replay-only`), `xtask world-index --live`, and
atlas sources (`meta.json` now also records `url`, `status`, `headers`).

## Not covered

- Calls made outside these seams (none known; grep for `api.meta.ai`).
- The watch sweep (`crates/ingest/src/sweep.rs`) still calls
  `fetch_url_conditional`; switching it to
  `fetch_url_conditional_cached` is the builder's one-line change.
