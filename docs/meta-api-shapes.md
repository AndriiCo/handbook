# Meta Model API — pinned JSON shapes (implementation input)

- Source: live dev.meta.ai docs, verified 2026-10-28 (prompt 049)
  and re-verified 2026-11-01 against
  `~/Code/meta-models/handbook-review/meta-api/raw/` (F1–F14 pass;
  raw pages fetched 2026-11-01 from https://dev.meta.ai/docs).
- Companion: [meta-api.md](meta-api.md) (model choice + policy).
- Status: pinned input for wiring `MetaRestClient` (readiness B1).
  Re-verify against dev.meta.ai if a call fails after a docs change.
- Doc links: [overview](https://dev.meta.ai/docs/overview),
  [chat completions](https://dev.meta.ai/docs/protocols/chat-completions),
  [Responses](https://dev.meta.ai/docs/protocols/responses),
  [structured output](https://dev.meta.ai/docs/structured-output),
  [reasoning](https://dev.meta.ai/docs/reasoning),
  [error handling](https://dev.meta.ai/docs/error-handling),
  [retry recipe](https://dev.meta.ai/docs/cookbook/error-handling-retry),
  [streaming recipe](https://dev.meta.ai/docs/cookbook/streaming-responses),
  [speech-to-text](https://dev.meta.ai/docs/speech-to-text),
  [SAM segmenting](https://dev.meta.ai/docs/sam/segmenting),
  [SAM output](https://dev.meta.ai/docs/sam/reading-segmentation),
  [file handling](https://dev.meta.ai/docs/file-handling),
  [prompt caching](https://dev.meta.ai/docs/prompt-caching).

## Endpoint + auth

- `POST https://api.meta.ai/v1/chat/completions`
- `Authorization: Bearer $MODEL_API_KEY` (upstream name; repo alias
  `META_API_KEY`). Key format `LLM_{numeric-id}_{secret}`.
- Gotcha: unknown paths return `404` with an **empty body** — branch
  on status, not on an error envelope.

## Request (fields we use)

```json
{"model": "muse-spark-1.3",
 "messages": [{"role": "developer", "content": "<aa-prompt-v1 standing rules>"},
              {"role": "user", "content": "<slug, max_records, doc_ids + evidence>"}],
 "reasoning_effort": "medium",
 "max_completion_tokens": 32000,
 "response_format": {"type": "json_schema",
   "json_schema": {"name": "ProposedRecords",
                   "schema": {"type": "object", "...": "..."}}},
 "prompt_cache_key": "analysis-agent-aa-prompt-v1",
 "stream": true,
 "stream_options": {"include_usage": true}}
```

Notes: `developer` sets standing rules and outranks `user`, but a
lone `developer` message is rejected with HTTP 400 (`` `messages`
must contain at least one message with role `user` or `tool` `` —
reproduced live 2026-11-01). Stable prompt first, volatile
evidence last: caching is automatic prefix matching on the stable
prefix. No `temperature`: Spark is tuned for its defaults
(`temperature: 1.0`, `top_p: 1.0`); leave sampling unset.
`max_tokens` is a deprecated alias, still accepted;
`max_output_tokens` is rejected on this endpoint. `reasoning_effort`:
never `none` (Spark always reasons, `none` → 400); reasoning
tokens count inside `max_completion_tokens` AND bill as output
(a two-character reply already costs ~385 tokens total — size caps
accordingly). Do NOT send: `stop`, `logprobs: true`, `n > 1`,
`top_p: 0`, `modalities`, `audio`, `logit_bias`, `text.format` —
all → 400. `frequency_penalty` / `presence_penalty`: leave unset.
`prompt_cache_key` is a stable use-case string (not per-user).

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

## Response (reference envelope; drafts stream — see below)

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

## Streaming (drafts always stream)

`stream: true` + `Accept: text/event-stream`. SSE `data: {...}`
lines, end `data: [DONE]`. Skip chunks with empty `choices` or
null `delta.content` (keep-alives and the terminal chunk).
`choices[0].delta` carries `{role?, content?, refusal?}`; the
terminal chunk carries `finish_reason` (`stop` ok; `length` =
truncated, never parse partial JSON). `delta.refusal` set = model
refused. With `stream_options: {"include_usage": true}` a trailing
chunk carries `usage` (prompt/completion tokens). Non-streaming
has a server time limit (→ 504 `gateway_timeout`); streaming is
exempt — stream anything large (drafts use a 300 s client budget).
A cut stream (no `[DONE]`) or mid-stream transport failure
restarts the whole request; 429/5xx retry with backoff + jitter
honouring `Retry-After`, terminal 4xx never retry.

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
out tokens (~$22/mo) **plus reasoning tokens, billed as output**
(measure per call: even trivial replies burn ~60; verdict-scale
JSON runs to ~1.5k — all inside the 32k draft budget). Contributor
≈ $0.05/day but trains on your content — **never for customer
PII** (fine for synthetic fixtures); no `-contributor` model is
listed in the example pack. Voice $0.18/audio-hour; SAM $2.50/1k
images.

## Explicitly absent

- **No embeddings endpoint** (both doc URLs 404; no embedding
  model, no pricing). Any vector work must be external or
  self-hosted (see RAG verdicts in the prompt-049 log: vectors
  deferred unless discovery needs them).
- **No Seamless/translation API** (P3 stays conditional).
- Transcription file endpoint `POST /v1/asr/transcribe`:
  multipart with TWO parts — `request` (JSON
  `{model, audioEncoding: "WAV", mode, languageBias?, keywords?}`,
  content type `application/json`) and `audio` (the WAV clip,
  `audio/wav`), header auth. Audio must be RIFF/WAVE mono 16-bit
  PCM at 16 or 24 kHz (`ffmpeg -ac 1 -ar 24000 -c:a pcm_s16le`);
  non-WAV or >10 min → 400, body >32 MB → 413. `mode`:
  `PUSH_TO_TALK` (single turn, `turns` empty), `ENDPOINTING`
  (turn boundaries), `DIARIZATION` (+ speaker labels).
  `languageBias` is a list of language names, not free text.
  Response: `{sessionId, transcript, audioDurationMs, turns[]}`
  with turns `{turnId, startMs, endMs, transcript, speaker?}` —
  turn-level timing only, never word-level. Live
  `wss://.../asr/realtime` with auth in the handshake frame, NOT
  the header.

## Responses API (pinned 2026-10-29, prompt 078; re-verified 2026-11-01)

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
- Structured output: `text = {format: {type: "json_schema",
  name, schema, strict}}` — FLAT (the nested `json_schema`
  wrapper is the Chat `response_format` shape; each endpoint
  ignores the other's param). `status: "incomplete"` +
  `incomplete_details` = truncated, never partial data; a
  `{"type": "refusal"}` content item = model refused.
- Token counting: `POST /v1/responses/input_tokens` with the
  trimmed input — measure before sending.
- Files API: `POST /v1/files` upload (1 GiB) with
  `purpose=user_data` for inference files, sent before the `file`
  part; expiry via `expires_after[anchor]=created_at` +
  `expires_after[seconds]` (3600–2592000; 7 days here).
  Uploaded files NEVER expire unless `expires_after` is set
  (team storage is capped — always set it).
  `DELETE /v1/files/{id}`; inline `file_data` / `file_url`
  capped at 50 MB. (`purpose=batch` JSONL datasets exist but NO
  batch-execution endpoint is documented — no `/docs/batch`,
  no `/v1/batches` shape found.)
- SAM 3.1 on Responses: one bare noun phrase per request in
  `input_text` + image in `input_image` (`POST
  .../v1/responses`, `model: "sam-3.1"`); commands, questions
  and comma/and lists return zero matches. Output is token text
  `<Nf>id<|box;x1;y1;x2;y2;w;h[;c]|><|mask;…|>,…` in the
  `output_text` lane — corners are source pixels (normalize by
  the record's own `w`/`h`), records key by object id (sparse,
  never positional), empty lane = zero matches (normal).
  `metadata: {"include_confidence": "true"}` (string, not
  boolean) adds `c` to every box and mask.
- Responses hygiene: `store: false` on calls never retrieved
  later (background polls stay stored); `safety_identifier`
  carries a hashed deployment id, never a username or email.
- PDF handling (server-side): text + per-page images together;
  ≤50 pages fully covered; longer PDFs give text to page 100
  and images to page 50.
- Output text: `output[]` message items carrying `output_text`
  blocks with a `text` string (verified live 2026-11-01);
  `input_image` takes an `image_url` (public URL or base64 data
  URL). Refusals arrive as `{"type": "refusal", "refusal":
  "..."}` content items.
