# Meta Model API — pinned JSON shapes (implementation input)

- Source: live dev.meta.ai docs, verified 2026-10-28 (prompt 049,
  researcher report; full text in that turn's child session logs).
- Companion: [meta-api.md](meta-api.md) (model choice + policy).
- Status: pinned input for wiring `MetaRestClient` (readiness B1).
  Re-verify against dev.meta.ai if a call fails after a docs change.

## Endpoint + auth

- `POST https://api.meta.ai/v1/chat/completions`
- `Authorization: Bearer $MODEL_API_KEY` (upstream name; repo alias
  `META_API_KEY`). Key format `LLM_{numeric-id}_{secret}`.
- Gotcha: unknown paths return `404` with an **empty body** — branch
  on status, not on an error envelope.

## Request (fields we use)

```json
{"model": "muse-spark-1.3",
 "messages": [{"role": "developer", "content": "<aa-prompt-v1 + evidence>"},
              {"role": "user", "content": "<capture excerpt + live facts>"}],
 "temperature": 0.1,
 "max_completion_tokens": 2000,
 "response_format": {"type": "json_schema",
   "json_schema": {"name": "ProposedRecords",
                   "schema": {"type": "object", "...": "..."}}},
 "prompt_cache_key": "analysis-agent-aa-prompt-v1",
 "stream": false}
```

Notes: `developer` role preferred over `system` (highest
precedence). `max_tokens` is a deprecated alias, still accepted;
`max_output_tokens` is rejected. `reasoning_effort`: never `none`
(Spark always reasons, `none` → 400). Do NOT send: `stop`,
`logprobs: true`, `n > 1`, `top_p: 0`, `modalities`, `audio`,
`logit_bias`, `text.format` — all → 400. `frequency_penalty` /
`presence_penalty`: leave unset. `prompt_cache_key` is a stable
use-case string (not per-user); caching itself is automatic prefix
matching — order stable prompt first, volatile evidence last.

## Structured output

`response_format: {"type": "json_schema", "json_schema": {"name",
"schema", "strict?"}}`. `name`: `[a-zA-Z0-9_-]`, ≤64 chars,
required. Decoding is schema-constrained (not post-processing);
result arrives as a JSON **string** in
`choices[0].message.content` — parse it, then run our own
byte-exact citation validator on top (the API cannot check
citations). Schema limits (400 if exceeded): depth 10, 5000
properties, 120k string chars, 1000 enum values; recursive/`$ref`
cycles → 400.

## Response (non-streaming)

```json
{"id": "chatcmpl-abc123", "object": "chat.completion", "created": 1714502400,
 "model": "muse-spark-1.3",
 "choices": [{"index": 0,
   "message": {"role": "assistant", "content": "<JSON string per schema>",
     "refusal": null, "tool_calls": [], "reasoning_content": null},
   "finish_reason": "stop", "logprobs": null}],
 "usage": {"prompt_tokens": 25, "completion_tokens": 19, "total_tokens": 44,
   "prompt_tokens_details": {"cached_tokens": 0},
   "completion_tokens_details": {"reasoning_tokens": 0}}}
```

`finish_reason`: `stop|length|tool_calls|content_filter|function_call`.
`usage` is optional — record when present (cost ledger in audit).
`finish_reason: length` → treat as truncated output (validator
failure path, not a partial write).

## Streaming (long generations only)

`stream: true` + `Accept: text/event-stream`. SSE `data: {...}`
lines, end `data: [DONE]`. Chunks share `id`/`created`;
`choices[0].delta` carries `{role?, content?, reasoning_content?,
tool_calls?[{index!, ...}]}`; final chunk carries `finish_reason`.
When `stream_options.include_usage` is set, the FINAL chunk has
empty `choices` and carries `usage` instead. Non-streaming has a
server time limit (→ 504 `gateway_timeout`); streaming is exempt
(first-event deadline only) — stream anything large. Mid-stream
failure = terminal SSE `error` event or `response.failed` → retry
the whole request.

## Errors + retry

```json
{"error": {"message": "`top_p`: The number must be `<= 1.0`.",
 "type": "invalid_request_error", "param": "top_p", "code": null}}
```

`type`: `invalid_request_error` (4xx), `authentication_error`
(401, `code: invalid_api_key`), `billing_error` (402),
`rate_limit_error` (429, `code: rate_limit_exceeded`, honor
`Retry-After`), `server_error` (5xx). Populated codes:
`model_not_found`, `payload_too_large` (413), `gateway_timeout`
(504), `server_shutting_down` / `service_overloaded` /
`backend_unavailable` (503, all retryable, `Retry-After` present).
Retry 429/500/503 with exp backoff + jitter (3–5 tries max);
never retry 400/401/402/403/404/413 unchanged. Rate-limit
headers on token responses: `x-ratelimit-limit-tokens`,
`x-ratelimit-remaining-tokens`, `x-ratelimit-limit-requests`,
`x-ratelimit-remaining-requests`.

## Rate limits + cost (batch-job scale)

Per-team, per model: Spark/Wonder 3000 RPM / 4,000,000 TPM
(Contributor 150 RPM / 3M TPM). ~500 short calls/day ≈ 0.35/min —
limits are irrelevant. Spark Standard ≈ $0.74/day at 500 in / 200
out tokens (~$22/mo); Contributor ≈ $0.05/day but trains on your
content — **never for customer PII** (fine for synthetic
fixtures). Voice $0.18/audio-hour; SAM $2.50/1k images.

## Explicitly absent

- **No embeddings endpoint** (both doc URLs 404; no embedding
  model, no pricing). Any vector work must be external or
  self-hosted (see RAG verdicts in the prompt-049 log: vectors
  deferred unless discovery needs them).
- **No Seamless/translation API** (P3 stays conditional).
- Transcription file endpoint `POST /v1/asr/transcribe`
  (multipart, header auth); live `wss://.../asr/realtime` with
  auth in the handshake frame, NOT the header.

## Responses API (pinned 2026-10-29, prompt 078)

- `POST https://api.meta.ai/v1/responses` with `model` +
  `input`; returns `id` + `status`. Retrieve `GET
  /v1/responses/{id}`; cancel `POST .../cancel`; stream a
  background job `GET ...?stream=true`.
- `background: true` runs async: ack `{background: true,
  status: queued}` (600 submissions/min/team); cannot combine
  with `stream: true` (400); creation body capped at 1 MiB
  (413 `payload_too_large` — upload media via Files first).
- `input`: `{role, content[]}` with typed blocks `input_text`,
  `input_image`, `input_file` (`file_data` base64 / `file_url`
  https / `file_id`), `input_video`; replay assistant turns
  with `output_text`. `instructions` is developer-level but
  per-request only (resend each turn); `previous_response_id`
  chains server-managed history; `reasoning: {effort}`.
- Structured output: `text.format = {type: json_schema,
  json_schema: {name, schema, strict}}` (NOT `response_format`
  — a Chat param; each endpoint ignores the other's).
- Token counting: `POST /v1/responses/input_tokens` with the
  trimmed input — measure before sending.
- Files API: `POST /v1/files` upload (1 GiB, purpose field;
  `purpose=batch` JSONL datasets exist but NO batch-execution
  endpoint is documented — no `/docs/batch`, no `/v1/batches`
  shape found); `DELETE /v1/files/{id}`; inline `file_data` /
  `file_url` capped at 50 MB. Uploaded files never expire.
- PDF handling (server-side): text + per-page images together;
  ≤50 pages fully covered; longer PDFs give text to page 100
  and images to page 50.
- Output text: `output[]` items carrying `output_text` blocks
  (defensive walk in `atlas::model`; re-verify item shape at
  first live use). `image_url` block field and upload
  `purpose` value are conventional, same caveat.
