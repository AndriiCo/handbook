# Meta Models API Digest (for this build)

- Source: https://dev.meta.ai/docs/overview.md (fetched 2026-10-26) +
  https://dev.meta.ai/docs/speech-to-text (streaming details).
- Status: decided input (prompt 008 Q1). Re-verified 2026-10-28
  (prompt 049): base URL + Bearer auth confirmed; upstream env name
  is `MODEL_API_KEY` (`META_API_KEY` stays a project-local alias);
  structured output confirmed; NO embeddings endpoint exists (404).
  Re-verified 2026-11-01 (F1–F14): every call site below matches
  the live docs; verification notes with file:line in
  `~/Code/meta-models/handbook-review/meta-api/findings.md`.
  Pinned request/response shapes: [meta-api-shapes.md](meta-api-shapes.md).

## Connection

- Base URL `https://api.meta.ai/v1` everywhere (one canonical
  value: Rust `META_BASE_URL` / `BASE_URL` / `META_API_BASE`
  consts, `xtask a2-record --base`). Auth `Authorization: Bearer
  $META_API_KEY` (never in repo/logs). Key lookup order: sealed
  `meta.api_key` row first, `$META_API_KEY` env fallback for
  operator one-shots (`xtask analyze/eval --live`); upstream env
  name is `MODEL_API_KEY` (`META_API_KEY` stays a project-local
  alias).
- Pricing pay-as-you-go; details at
  https://dev.meta.ai/docs/pricing-rate-limits (transcription:
  $0.18/hr; reasoning tokens bill as output).

## Models we use

- **Muse Spark** (`muse-spark-1.3`, also 1.2/1.1; `-contributor`
  discounted variants): analysis-agent drafting (F4/F5), call-console
  follow-up questions. 1M-token context, parallel/streamed tool
  calls, reasoning across turns. Structured-output and prompt-caching
  APIs exist — use both (citations schema; frozen prompt versions).
- **Muse Wonder** (`muse-wonder-1.0`): same requests as Spark;
  reserve for the hardest jobs (contradiction arbitration drafts);
  default to Spark on cost.
- **Muse Voice Transcribe** (`muse-voice-transcribe-1.0`):
  - Live: `wss://api.meta.ai/v1/asr/realtime` — WebSocket.
    **Authenticate in the handshake frame; the `Authorization`
    header is ignored.** Session via `?sessionId=`. `mode` is fixed
    once a session starts; `languageBias` + `keywords` are set at
    start and immutable. Turn detection, speaker diarization,
    endpointing/VAD in-model; 25 languages + code-switching.
  - Files: `POST /v1/asr/transcribe` (multipart upload), with an
    event-stream option following the same turn pattern.
  - Output: transcript text with **turn-level** timing (+ optional
    speaker labels). **No word-level timestamps.** STT only — no
    TTS, no speech-to-speech API.
  - Feeds the P1 call console (`call:{id}` WS) and stored
    `capture(kind='transcript')`.
- **SAM 3.1** (on Model API via `POST .../v1/responses`,
  `model: "sam-3.1"`): ONE bare noun phrase per request
  (`"face"`, then a separate call for `"license plate"` —
  commands, questions and comma/and lists return zero matches)
  → a box and a mask per match, images and frame-tracked video.
  Output grammar: `<Nf>id<|box;x1;y1;x2;y2;w;h[;c]|><|mask;…|>,…`
  in the `output_text` lane (corners are source pixels, records
  key by object id, empty lane = zero matches). Feeds F9
  redaction for both customer-side and admin-side use (see
  [specs/app-features.md](specs/app-features.md) F9). Verify recall
  on fixtures; a self-hosted detector is the fallback, not the
  plan.
- **Muse Glimmer**: open-weight, Apache 2.0, **self-host only**
  (vLLM/llama.cpp) — not on Model API. Feeds F10 pre-check with no
  customer pixels leaving the server.

## Explicitly NOT on Model API (per overview)

- **Seamless / SeamlessStreaming**: absent from the overview. P3
  live translation stays conditional — recheck the docs before
  designing it; P1 depends on nothing from it.

## Client design consequences

- Clients (one per surface, all blocking + stubbed for tests):
  `MetaRestClient` (chat drafting, streamed SSE) + `StubClient`
  in the analysis agent; atlas `ModelPort` (`MetaClient` live,
  `StubPort` scripted: chat, background Responses, Files,
  Transcribe); media `VoiceFileClient` (`request` JSON + `audio`
  multipart) and `MetaSamClient` (one Responses call per concept)
  behind the `media.terms_read` gate; the A2 recorder
  (`xtask a2-record`, same SAM body and box parser) for replay fixtures. Add a
  realtime-STT client honoring the handshake-auth gotcha above.
- `StubClient` + fixtures keep `cargo test` and `dev-seed`
  offline-safe; `--dry-run` prints projected calls/tokens.
- Open: exact request/response JSON shapes (pin at implementation);
  Meta API data-handling terms for customer PII (audio/photos) —
  read before sending real customer media.

## Official rules vs this code (checklist, 2026-11-01)


Every Meta API call in this repo follows these rules. The official pages were fetched 2026-11-01 from
https://dev.meta.ai/docs. A citation `raw/<name>.md` means the page
`https://dev.meta.ai/docs/<name with _ as />.md` (e.g.
`raw/protocols_chat-completions.md` →
`https://dev.meta.ai/docs/protocols/chat-completions.md`). The operator's
Mac keeps copies under `~/Code/meta-models/handbook-review/meta-api/raw/`.
They stay out of this repo, which goes public. Code references are to
`4b711c4` (P182). Each finding was checked against both the page and the
code line. A finding is fixed when the code follows the rule and a
contract test pins it.

Status: the fixes for F1-F14 were merged to `main` on 2026-11-01 (P207).

## Call sites

| Path | Endpoint | Proven live? |
|---|---|---|
| Analyzer, A5 eval, admin draft: `crates/handbook-agent/src/model.rs` | `POST /v1/chat/completions`, `muse-spark-1.3` | yes, staging 10-31 (`docs/ops/deploy.md:155-166`) |
| Atlas + instruct worker: `crates/atlas/src/model.rs`, `crates/instruct/src/{draft,trigger}.rs` | chat, `/v1/responses`, `/v1/files`, `/v1/asr/transcribe` | no; but `apostille-instruct.service` already runs `--live` |
| A2 recorder: `xtask/src/a2_record.rs` (was `xtask/ops/a2-record.py` until P411) | SAM on `/v1/responses`; Spark vision on chat | yes, staging and production (recorded verdicts) |
| Media crate: `crates/media/src/{redact,transcribe}.rs` | invented `/v1/sam/segment`; `/v1/asr/transcribe` | no, called only from tests |

## Findings

**F1. `temperature` is set everywhere; the docs say leave it unset.**
Code: `handbook-agent/src/model.rs:349` (0), `a2-record.py:156` (0),
`atlas/src/task.rs:350,406,654` (0.1; `atlas/src/model.rs:384` only
forwards `req.temperature`, and `:960` is a test fixture),
`instruct/src/draft.rs:117,144` and `trigger.rs:164` (0.1). Official: Spark "is tuned to run at the default
sampling settings (`temperature: 1.0`, `top_p: 1.0`)"
(`raw/protocols_chat-completions.md:401,421`;
`raw/cookbook_quickstart-chat-completions.md:234`). Keep the change only
if the A5 eval is no worse.

**F2. Both speech-to-text clients send the wrong multipart body.**
Code: `atlas/src/model.rs` (≈:538, form field `model`) and
`media/src/transcribe.rs:87` (form field `model`). Official: two parts,
`request` (JSON `{model, audioEncoding:"WAV", mode, languageBias?,
keywords?}`, content type application/json) and `audio`. Audio must be
mono 16-bit WAV at 16 or 24 kHz; over 10 minutes or non-WAV returns 400,
over 32 MB returns 413 (`raw/api-reference_voice_transcribe.md:5,39,47,63`).
For calls, set Telnyx recording to WAV mono or transcode before upload.

**F3. Responses structured output is nested; it must be flat.**
Code: `atlas/src/model.rs:431-435` sends
`text.format = {type, json_schema:{name,schema,strict}}`. Official:
`text:{format:{type:"json_schema", name, schema, strict}}`. Chat keeps
`response_format:{type:"json_schema", json_schema:{…}}`
(`raw/structured-output.md`, `raw/cookbook_structured-output.md:152-163`).
`docs/meta-api-shapes.md:142-144` carries the same wrong shape.

**F4. SAM 3.1 prompts and the dead SAM client.**
- A2 sends `"Locate: …"`. Official: one bare noun phrase per request.
  "Commands … are unreliable", and lists or second phrases return zero
  matches (`raw/sam_segmenting.md:3-17`). This likely explains the "0 boxes
  on flat synthetics" result.
- Output is token text `<|box;x1;y1;x2;y2;w;h|>` plus masks, keyed by
  object id. Empty output means zero matches. Metadata values are strings,
  e.g. `include_confidence:"true"` (`raw/sam_reading-segmentation.md`).
- `media/src/redact.rs:19,24,168` joins base `…/v1` to path
  `/v1/sam/segment`, producing `/v1/v1/…`. It uses an invented body
  (`imageBase64`), and tests at `:344` lock that in.
- `docs/meta-api.md:43` puts "face" and "license plate" in one request;
  they must be separate requests.

**F5. `developer` role: the docs prefer it, but staging got a 400.**
Official: `developer` sets standing rules and outranks `user`
(`raw/protocols_chat-completions.md:21-31`). Staging saw "endpoint 400s
otherwise" and switched to a lone `user` message
(`docs/ops/deploy.md:163`). Error bodies are cut to 120 characters
(`handbook-agent/src/model.rs:222`), which may have hidden the real cause
(a field in the same request). Reproduce with a minimal request and the
full error body before settling.

**F6. Token caps are too small for a reasoning model.**
Both caps count reasoning plus visible text, and `length` can come back
with empty content (`raw/reasoning.md`). Code: instruct and atlas use 4096
(`crates/instruct/packs/general.toml:42`), atlas summaries 2048
(`atlas/src/task.rs:653`), A2 4000 (`a2-record.py:157`). The analyzer needed 32k on staging. No call sets
`reasoning_effort` (values: minimal, low, medium, high, xhigh, max). Size
caps from P177's measured usage.

**F7. The prompt layout defeats prefix caching.**
`crates/handbook-agent/prompts/aa-prompt-v1.md:7-15` puts slug, max_records,
doc_ids and the evidence before the rules, so almost nothing at the start
is stable. Official: caching is automatic on a stable prefix;
`prompt_cache_key` is one key per shared prefix; check `cached_tokens` in
usage (`raw/prompt-caching.md`). Responses, verify (`atlas/src/task.rs:408`
`cache_key: None`), refine and condense calls send no cache key.

**F8. No streaming, and 32k-token calls under a 90 s timeout.**
`handbook-agent/src/model.rs:125,295`. Official: non-streaming requests
hit a server time limit (504 `gateway_timeout`). Don't retry unchanged;
stream, or use Responses `background:true` (body ≤1 MiB)
(`raw/error-handling.md`).

**F9. Retries.**
The analyzer sleeps 2 s then 8 s, with no jitter and no Retry-After
(`model.rs:88,171-186`). The runner retry, the model retry and the HTTP
retry stack on one draft. A2 has none. Official: 3–5 attempts with jitter
honoring Retry-After on 429/500/503; never retry 400/401/402/403/404/413
unchanged (`raw/error-handling.md`, `raw/cookbook_error-handling-retry.md`).

**F10. The A2 Spark call doesn't check truncation or refusal.**
`a2-record.py` parses `content` without checking `finish_reason` or
`refusal`, and its failure path uses `"choice" in dir()`, which is fragile
(`:173`).

**F11. The Responses path ignores refusals and `status:"incomplete"`.**
`atlas/src/model.rs` (≈:669). An incomplete response falls into "unknown
status". Official: truncation shows as `status:"incomplete"` with
`incomplete_details` (`raw/protocols_responses.md`).

**F12. Data handling.**
- `crates/atlas/packs/example-membership.toml:26-27` declares a
  `-contributor` model (its task keeps `allow_contributor = false`, :51).
  Contributor models are trained on your content, so no personal or
  confidential data may go to them (ToS §6.2); the gate must stay off on
  any path that can carry customer data.
- Atlas transcription and attachments skip the `media.terms_read` gate
  that the media crate enforces.
- Raw model output is written to `debug/extract.txt` (atlas `task.rs:382`).
- Uploaded Files never expire unless `expires_after` is set.
- Use `store:false` and a hashed `safety_identifier` on Responses.
  (`raw/file-handling.md`, dev.meta.ai legal pages.)

**F13. Config sprawl.**
- The base URL const is in 4+ places. Rust bases include `/v1`; Python's
  doesn't; the media crate doubles it.
- The env-key path ignores `meta.base_url`, and `web/draft.rs` ignores it
  on every path.
- `model.rs:93` says the key is "never env", but the code reads env first.
- Model ids are hardcoded in the analyzer and A2, and the voice model id
  is duplicated (`atlas/src/model.rs:30`, `media/src/transcribe.rs:22`).

**F14. The project's own Meta docs are out of date.**
- `docs/meta-api-shapes.md`:
  - `:23`: temperature 0.1.
  - `:24`: a 2000-token cap.
  - `:109-113`: the cost estimate leaves out reasoning tokens, which are
    billed as output.
  - `:142-144`: the Responses format shape.
  - `:151`: says files never expire.
- `docs/meta-api.md`:
  - `:43`: SAM phrases in one request.
  - Still says "key from env only" and "one ModelClient trait".

## Meta capabilities the app doesn't use yet (optional, after F1-F14)

- **Responses API** (the documented default) for drafting, with
  `reasoning.effort` and `store:false`.
- **Tool calling**, with quote verification as a tool, as in the
  workshop's `quote_on_page`.
- **muse-wonder-1.0** (1M context) for whole-document analysis.
- **Realtime speech-to-text** for live calls: auth goes in the first
  frame; PCM 16 or 24 kHz.
- The **trace console** already shows usage (P177). Add `cached_tokens`
  and reasoning tokens per call so judges can see correct usage.
