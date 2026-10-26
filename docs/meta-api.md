# Meta Models API Digest (for this build)

- Source: https://dev.meta.ai/docs/overview.md (fetched 2026-10-26) +
  https://dev.meta.ai/docs/speech-to-text (streaming details).
- Status: decided input (prompt 008 Q1). Exact JSON shapes get pinned
  at implementation against these pages.

## Connection

- Base URL `https://api.meta.ai/v1`, auth `Authorization: Bearer
  $MODEL_API_KEY` (env `MODEL_API_KEY`, never in repo/logs).
- Pricing pay-as-you-go; details at
  https://dev.meta.ai/docs/pricing-rate-limits (transcription:
  $0.18/hr).

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
- **SAM 3.1** (on Model API): text prompt ("face", "license plate")
  → boxes + pixel masks, images and frame-tracked video. Feeds F9
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

- One `ModelClient` trait (see
  [specs/analysis-agent.md](specs/analysis-agent.md) §4) with
  `MetaRestClient` + `StubClient`; add a realtime-STT client
  honoring the handshake-auth gotcha above.
- `StubClient` + fixtures keep `cargo test` and `dev-seed`
  offline-safe; `--dry-run` prints projected calls/tokens.
- Open: exact request/response JSON shapes (pin at implementation);
  Meta API data-handling terms for customer PII (audio/photos) —
  read before sending real customer media.
