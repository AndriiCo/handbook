# Voice Calling via Telnyx Call Control (spec)

- Status: draft (goal-025 stream 2 of 5, for author review; thin-slice
  implementation alongside in `crates/voice` + `admin_voice.rs`)
- Date: 2026-10-27
- Source: Telnyx developer docs (fetched 2026-10-27) plus secondary
  sources where marked. Claims without a Telnyx-docs link need
  portal/spec confirmation before production use.

Scope: browser-initiated outbound voice calls from the admin UI
(staff calls an authority office, asks prepared questions, records
with consent, reads a live transcript). Inbound calling is out of
scope. No customer PII flows through this lane: the callee is a
public office, and transcripts feed the existing staff fact-draft
flow, never the public site.

## 1. Telnyx primitives (verified)

Base URL `https://api.telnyx.com`, auth `Authorization: Bearer
[REDACTED]` (API key from the [Mission Control
portal](https://portal.telnyx.com/)). Dial needs a Voice API
application (Call Control connection) with a webhook URL plus an
attached Outbound Voice Profile; its `id` is the `connection_id`
passed at dial. `from` must be a number on the account or dial is
rejected (error D35). ([quickstart](https://developers.telnyx.com/docs/voice/programmable-voice/quickstart))

| Need | Call | Notes |
|---|---|---|
| Dial | `POST /v2/calls` `{to, from, connection_id}` | Returns `data.call_control_id`, the handle for every later command ([quickstart](https://developers.telnyx.com/docs/voice/programmable-voice/quickstart)) |
| Record start/stop | `POST /v2/calls/{id}/actions/record_start` `{format, channels}` / `record_stop` | Recording also stops at hangup. No command webhooks; readiness arrives as `call.recording.saved` ([v1 record_start](https://developers.telnyx.com/api/v1/mission-control/call-control-record-start), [Symbl walkthrough](https://symbl.ai/developers/blog/create-powerful-automations-on-your-telnyx-calls-with-symbl-ai-and-pipedream/)) |
| Hangup | `POST /v2/calls/{id}/actions/hangup` | ([hangup ref](https://telnyx-openapi-ng.s3.us-east-1.amazonaws.com/call-control-commands/hangup.yml)) |
| Live transcription | `POST /v2/calls/{id}/actions/transcription_start` / `transcription_stop` | Segments arrive as `call.transcription` webhooks ([event table](https://developers.telnyx.com/docs/voice/programmable-voice/voice-api-webhooks)) |

Idempotency: send `command_id` with commands; Telnyx ignores a
duplicate within 60 s. ([receiving webhooks](https://developers.telnyx.com/docs/voice/programmable-voice/receiving-webhooks))

### Webhooks (verified)

Delivered to the Voice API application's `webhook_event_url`
(`POST`, `User-Agent: telnyx-webhooks`), failover URL on failure.
A 2xx response acknowledges; anything else fails delivery.
Webhooks are unordered, near-simultaneous, and may duplicate —
handlers must be idempotent on the webhook `data.id` and must
answer 2xx fast, doing slow work (audio download) afterwards.
([webhook reference](https://developers.telnyx.com/docs/voice/programmable-voice/voice-api-webhooks),
[receiving webhooks](https://developers.telnyx.com/docs/voice/programmable-voice/receiving-webhooks))

Signature: every webhook carries `Telnyx-Signature-Ed25519` +
`Telnyx-Timestamp`; verification is Ed25519 over
`{timestamp}|{raw-body}` (ASCII pipe, exact body bytes), with the
signature and the Mission Control public key both strict padded
base64. Verified 2026-10-28 against Telnyx's own Node SDK
(`src/lib/webhooks.ts`, `TelnyxWebhook.verify`) and the messaging
webhook docs ("computed over `{timestamp}|{json_payload}`") — an
earlier `{timestamp}.{body}`/hex guess was wrong and is fixed in
`crates/voice/src/webhook.rs` (OQ-1 closed; a regression test pins
the pipe construction and rejects the dotted one). Headers are
case-insensitive; timestamps must be digits-only within 300 s.

Event envelope: `data.{record_type, event_type, id, occurred_at,
payload}`, `payload.{call_control_id, connection_id, call_leg_id,
call_session_id, client_state, from, to, direction, state}`.

Events this slice handles (full table in the [webhook
reference](https://developers.telnyx.com/docs/voice/programmable-voice/voice-api-webhooks)):

| Event | Meaning | Our handler |
|---|---|---|
| `call.initiated` | Leg created | Mark call `ringing` (idempotent on webhook id) |
| `call.answered` | Remote picked up | Mark `in_progress`; auto `record_start` + `transcription_start` when consent was confirmed at dial |
| `call.hangup` | Call ended | Mark `completed`; store hangup cause/source; stop polling |
| `call.recording.saved` | Recording ready; `payload.recording_urls.{wav,mp3}` | Download one format (mp3) server-side with the API key, store as content blob, link to call |
| `call.transcription` | One live transcript segment | Append segment row (seq per call); UI polls |
| `call.machine.detection.ended` | AMD result (if enabled at dial) | Store result; UI shows "voicemail suspected" hint |
| anything else | — | Log event type only (no payload), keep 2xx |

`call.recording.transcription.saved` (post-call full transcript in
`transcription_text`) is reported by a [secondary
source](https://lowlatencyclub.ai/blog/posts/build-ai-voicemail-transcription)
only — confirm in the portal event catalog before relying on it
(OQ-2). Until then the transcript is the ordered `call.transcription`
segments plus the recording itself.

## 2. Our call states

Local state machine (stored on `voice_call.status`); Telnyx is the
source of truth for transitions, the UI polls our rows:

```
queued → ringing → in_progress → completed
   |         |          |
   v         v          v
 failed    failed    failed      (terminal; `status_detail` keeps the cause)
```

- `queued`: dial accepted by Telnyx, no `call.initiated` yet.
- `ringing`: `call.initiated` seen.
- `in_progress`: `call.answered` seen.
- `completed`: `call.hangup` seen (normal end, either party).
- `failed`: dial rejected, or hangup with a failure cause before
  answer (busy / no-answer / failed), or AMD `machine` + staff
  hangup. Never resurrected: webhooks for terminal calls only
  attach artifacts (recording/transcript), never move state.
- Out-of-order/duplicate webhooks: every webhook row is keyed by
  Telnyx webhook `data.id` (`INSERT OR IGNORE`); state moves only
  forward along the diagram above.

## 3. Webhook handlers (ours)

Receiver: `POST /apostille/api/v1/admin/calls/webhook` on the
ADMIN listener (admin host guard applies; no staff session —
Telnyx authenticates by Ed25519 signature, verified in-handler).
CSRF does not apply (server-to-server, no session); signature
failure → 401, unknown call → 2xx + drop (log ids only), parse
failure → 400. Every accepted webhook is persisted before its
effect runs, so a retried delivery is a no-op.

Handler order per webhook: verify signature → parse envelope →
`INSERT OR IGNORE` webhook id → load call by `call_control_id` →
apply event (§1 table) → 2xx. The `call.recording.saved` download
runs after the 2xx on a background task (timeout 30 s, one retry);
a failed download leaves `recording_status=pending` for a later
staff retry — no unbounded queue in this slice.

## 4. Recording storage + retention

- Recordings download to the store blob dir (`Store::write_blob`,
  content-addressed sha256, same as media intake) and link from
  `voice_call.recording_sha`. The Telnyx-hosted URL is never
  persisted (it is a bearer URL — anyone holding it can fetch).
- Retention: recordings are working notes for fact drafting, not
  archives. Keep 90 days, then delete blob + transcript text,
  keeping only the call metadata row (who/when/duration/consent
  record) for audit. Sweep rides the existing retention sweep
  pattern (`retention_sweep`; voice sweep is a follow-up — OQ-3).
- Access: staff role only, admin listener only. No recording or
  transcript bytes in logs (§6).

## 5. Transcript source

Primary: live `call.transcription` webhook segments, appended to
`voice_transcript(call_id, seq, speaker, text, at)`. The admin UI
polls `GET .../calls/{id}` every 2 s while the call is
non-terminal (a WS push on the bus is a follow-up; polling keeps
this slice off the shared `auth::ws` topic registry). Fallback:
the recording itself; post-call full-text webhook per OQ-2.

## 6. Consent + audit requirements

- Washington is an all-party-consent state (RCW 9.73.030) and the
  pilot is Washington: recording or transcribing without telling
  the callee is unlawful. The dial request REQUIRES
  `consent_confirmed: true` (staff attests they will announce
  recording at the start of the call); without it dial is refused
  (422). Whether to also auto-play a spoken disclosure via
  `speak` before `record_start` is OQ-4.
- Every call-affecting action audits to the `audit` table:
  `voice_dial` (actor, call id, authority; never the number),
  `voice_hangup`, `voice_record_start/stop`, webhook receipts at
  debug level only. Audit rows carry ids + timestamps, never phone
  numbers, transcript text, or audio.
- Logs: call ids, event types, byte counts, durations. Phone
  numbers, names spoken on the call, transcript text, and the API
  key/secret must never appear in logs, error messages, or WS
  payloads. Record-start/stop is staff-initiated from the UI (or
  automatic at answer when consent was confirmed — same thing,
  attested at dial).

## 7. Our API (admin listener, staff + CSRF except webhook)

```
POST /apostille/api/v1/admin/calls/dial
  {to, connection_id?, from?, authority?, questions[], consent_confirmed}
  → 200 {call_id, status} | 422 consent missing / bad number | 503 voice not configured
GET  /apostille/api/v1/admin/calls?limit=
  → {calls: [{call_id, status, authority, started_at, ended_at, recording, questions_asked, questions_total}]}
GET  /apostille/api/v1/admin/calls/{id}
  → {call_id, status, authority, questions, transcript, recording_status, created_at, ended_at}
POST /apostille/api/v1/admin/calls/{id}/hangup            → {status}
POST /apostille/api/v1/admin/calls/{id}/record/start      → {recording: true}
POST /apostille/api/v1/admin/calls/{id}/record/stop       → {recording: false}
POST /apostille/api/v1/admin/calls/{id}/questions/{idx}/asked  → {asked: true}
POST /apostille/api/v1/admin/calls/webhook (Telnyx → us; signature, no session)
POST /apostille/api/v1/admin/calls/{id}/quote (Slice E, "quote to draft")
  {segment_ids: [int...], topic, statement}
  → 200 {fact_id} | 400 malformed | 404 unknown call/segment | 422 bad topic/statement
```

Quote-to-draft inserts ONE proposed fact via the standard
`insert_proposed_fact` path (same queue, same approval gate —
voice drafts get no shortcut). Voice has no capture row, so the
fact is unattributed at rest (`capture_id` 0-style, created_by
`staff:<user>`); the call+segment refs ride the audit row
(grep-able reason string, zero DDL — the magic `fallback` mark
precedent). Staff-only with the file's standard guard stack;
statement 1..1024 chars; topic must parse in the frozen
25-enum. No auto-accept, no bulk import in v1.

`to` accepts E.164 (`+1…`) only in this slice. `connection_id` /
`from` default from sealed DB rows (`telnyx.connection_id`,
`telnyx.from_number`); request overrides exist for the stub tests
and multi-number futures, and are ignored unless non-empty. When
`telnyx.api_key` is unset the staff endpoints answer 503 (fail
closed); the webhook answers 401 (no key to verify with).

## 8. Acceptance criteria

1. Staff dials a synthetic number against the stub Telnyx server:
   call row `queued`, audit `voice_dial`, no real HTTP leaves the box.
2. Stub emits initiated → answered → transcription ×N →
   recording.saved → hangup: UI shows live segments, recording
   blob stored under its sha, call `completed`.
3. Dial without `consent_confirmed` → 422, no Telnyx call, audit
   row for the refusal.
4. Webhook with bad signature → 401 and no state change; replayed
   webhook id → 2xx no-op; unknown `call_control_id` → 2xx drop.
5. Non-staff session → 403; wrong host → 403; mutation without
   Origin/CSRF → 403; nothing under `/calls` on the public listener.
6. Logs for a full call contain no phone number, transcript text,
   or key material (grep gate in tests where feasible).
7. `cargo xtask ci` green; `npm run check`, `test`, `build:admin`,
   `smoke` green.

## 9. Telnyx account needs (author checklist)

1. Telnyx account + API key (`telnyx.api_key`, secret — sealed
   config row via admin UI, never in repo/env files).
2. One voice number on the account (`telnyx.from_number`, E.164).
3. Voice API application (Call Control connection) with
   `webhook_event_url =
   https://admin.andrii.ai/apostille/api/v1/admin/calls/webhook`
   (single-443, no port — the old `:444` form is dead) (+
   failover URL), `webhook_timeout_secs` ~10; its id is
   `telnyx.connection_id`.
4. Outbound Voice Profile attached to the application (outbound
   billing + destinations).
5. Webhook signing public key from the portal for signature
   verification (`telnyx.webhook_public_key`).
6. Budget note: Telnyx prices Call Control from $0.002/min
   ([source](https://telnyx.com/resources/what-is-call-control));
   confirm current voice + recording + transcription rates in the
   portal before the pilot.

## 10. Open questions

- OQ-1 CLOSED 2026-10-28: `{timestamp}|{body}`, base64 sig +
  key (Node SDK source + messaging docs; see §Signature). A live
  portal test webhook during the Phase-0 proof call is still the
  final confirmation.
- OQ-2: is `call.recording.transcription.saved` available on our
  API version for post-call full text? Else segments + recording only.
- OQ-3: 90-day recording retention OK? And who runs the sweep —
  fold into `retention_sweep` or a voice-owned xtask?
- OQ-4: auto-play a spoken recording disclosure (`speak`) before
  `record_start`, or is staff's verbal announcement enough?
- OQ-5: AMD on outbound dials (`answering_machine_detection`) —
  enable from day one or after the pilot?
- OQ-6: WS push for transcript segments (new bus topic) vs
  polling — polling ships in this slice; revisit if staff hate it.
