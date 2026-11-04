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

Receiver: `POST /apostille/api/v1/calls/webhook` on the
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
POST /apostille/api/v1/calls/dial
  {to, connection_id?, from?, authority?, questions[], consent_confirmed}
  → 200 {call_id, status} | 422 consent missing / bad number | 503 voice not configured
GET  /apostille/api/v1/calls?limit=
  → {calls: [{call_id, status, authority, started_at, ended_at, recording, questions_asked, questions_total}]}
GET  /apostille/api/v1/calls/{id}
  → {call_id, status, authority, questions, transcript, recording_status, created_at, ended_at}
POST /apostille/api/v1/calls/{id}/hangup            → {status}
POST /apostille/api/v1/calls/{id}/record/start      → {recording: true}
POST /apostille/api/v1/calls/{id}/record/stop       → {recording: false}
POST /apostille/api/v1/calls/{id}/questions/{idx}/asked  → {asked: true}
POST /apostille/api/v1/calls/webhook (Telnyx → us; signature, no session)
POST /apostille/api/v1/calls/{id}/quote (Slice E, "quote to draft")
  {segment_ids: [int...], topic, statement}
  → 200 {fact_id, call_id, topic, status} | 400 malformed | 404 unknown call/segment | 422 bad topic/statement
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
   https://admin.andrii.ai/apostille/api/v1/calls/webhook`
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

## 11. Credential-free validation slice

Goal: prove the whole voice lane end to end — dial →
webhooks → transcript → record → quote-to-draft → review →
playback → sweep — with zero Telnyx credentials on the box.
Every byte Telnyx would send or receive is a checked-in
fixture or a stub assertion; the creds-gated handoff (§11.8)
is config rows only, no code change.

Non-goals: real dialing, real audio fidelity, WS push (OQ-6
stands), bulk quote import, any new PII surface (§6 rules
apply to fixtures too: synthetic numbers and invented office
names only).

### 11.1 Script packs (questions file layout + schema)

Staff read from a prepared pack so every pilot call asks the
same questions in the same order. Packs are repo files, loaded
at dial time; the dial request may still pass inline
`questions[]` (§7), which win over the pack for that call only.

Layout (checked in, synthetic content only):

```
crates/voice/scripts/
  default.toml            # fallback pack, 3 generic questions
  wa-auditor.toml         # per-authority packs, one file each
  wa-secretary-of-state.toml
```

Schema (TOML; one pack per file):

```toml
id = "wa-auditor"                 # required, kebab-case, unique; doubles as filename stem
authority = "WA State Auditor"    # required, display name (public office, never a person)
topics = ["hours", "fees"]        # required, ≥1; each must parse in the frozen 25-enum (§7)
questions = [                     # required, 1–20 items
  "What are your walk-in hours for apostille requests?",
  "What is the per-document fee this quarter?",
]
```

Validation (at load; failure → dial 422, nothing leaves the
box): `id` kebab-case matching the filename stem (grep-able,
no collisions); `authority` 1..128 chars; `topics` ≥ 1, each
in the frozen 25-enum (quote-to-draft needs a parseable
topic); 1–20 questions, each 1..280 chars after trim (bounds
the dial payload and the UI checklist); file parses as UTF-8
TOML under 16 KiB (stays reviewable in a diff).

Default-script fallback: when dial passes neither
`questions[]` nor a pack id, the server loads `default.toml`.
When dial names a pack id that does not exist or fails
validation, dial is refused (422 + refusal audit row per §8
item 3) — never an empty question list on a live call. `GET
.../calls/{id}` echoes the resolved `questions` plus
`script_id` (`"inline"` when the request carried its own
list).

Fixtures only: packs live under `crates/voice/scripts/`,
webhook bodies under `crates/voice/tests/fixtures/`.
Canonical pilot values (real office names, real numbers) never
appear in either — synthetic `+1555…` numbers and invented
offices only, per the cohort fixtures rule.

### 11.2 Webhook fixture corpus + replay harness

Corpus: `crates/voice/tests/fixtures/webhooks/*.json`, one
file per delivery, envelope exactly per §1 (`data.{record_type,
event_type, id, occurred_at, payload}`). Required files (names
are the contract the harness and tests reference):

- `initiated.json`, `answered.json` — happy-path open (§1).
- `transcription-1.json`, `transcription-2.json` — two live
  segments, ascending `seq`.
- `recording-saved.json` —
  `payload.recording_urls.mp3` → stub audio URL (small
  checked-in `.mp3`, < 64 KiB).
- `hangup.json` — normal end → `completed`.
- `duplicate-answered.json` — same `data.id` as
  `answered.json` → 2xx no-op.
- `out-of-order-hangup.json` + `late-answered.json` — hangup
  before answer: state moves forward only, never resurrected
  (§2).
- `replay-recording-saved.json` — same `data.id` as
  `recording-saved.json` → blob not re-downloaded.
- `unknown-leg.json` — well-signed, unknown
  `call_control_id` → 2xx + drop (log ids only).
- `bad-signature.json` — tampered body → 401, no state
  change, nothing persisted.
- `machine-detection.json` — AMD `machine` result →
  "voicemail suspected" hint (§1 event catalog; OQ-5 covers
  live AMD tuning).
- `unknown-event.json` — unlisted `event_type` → 2xx, event
  type logged, no payload logged.

Replay-harness entry: `cargo xtask voice-replay --corpus
crates/voice/tests/fixtures/webhooks --scenario happy`
(scenarios: `happy`, `duplicates`, `out-of-order`, `replays`,
`unknown-leg`, `bad-signature`). The harness boots the app
with the stub Telnyx client (§11.3), dials one synthetic
call, POSTs each fixture body to the real webhook route with
a freshly computed Ed25519 signature over
`{timestamp}|{raw-body}` (test keypair generated per run,
never persisted), and asserts final call status + row
counts. The bad-signature scenario asserts 401 with a
deliberately wrong key. Slow work (the mp3 fetch) runs
against the stub, after the 2xx, same as production (§3).

### 11.3 Stub-Telnyx contract (never real HTTP)

`TelnyxClient` is a struct with a `base_url` override: tests
point it at a port-0 stub listener serving the §1 shapes,
while production with `telnyx.api_key` unset fails closed
(503 staff / 401 webhook per §7) — no ambient "are we
stubbed" flag the app can misread.

Endpoints faked (request/response shapes mirror §1 exactly):

- `POST /v2/calls` → synthetic `call_control_id`
  (`stub-<uuid>`); records `{to, from, connection_id,
  command_id}`; rejects non-E.164 `to` and empty `from`
  like production validation.
- `.../actions/record_start` / `record_stop` → records the
  call id + params; `{ok: true}`.
- `.../actions/transcription_start` /
  `transcription_stop` → records the call id; `{ok: true}`.
- `.../actions/hangup` → records the call id; `{ok: true}`.
- Recording download (`recording_urls.mp3`) → serves the
  checked-in stub `.mp3` bytes; counts fetches per webhook
  id (replay ⇒ count stays 1).

Action-log assertions: every test/harness run gets a fresh
in-memory action log. Tests assert on it, not on real I/O:
dial sent exactly once with the expected `command_id`;
consent refusal (§8 item 3) sent nothing; auto
record+transcription start fired at answer when
`consent_confirmed` was set; hangup sent once. The stub
refuses to construct against `https://api.telnyx.com` (panic
in tests, error in harness) so a misconfigured run fails
loudly instead of dialing. No test in this slice opens a
socket except to the local stub listener.

### 11.4 Segment fix/annotate API deltas

Live segments are drafts: staff fix misheard words and pin
flags before quoting to facts. Semantics reuse
`media/transcript.rs` `apply_fix` exactly — same function
where the types allow, same rules where they don't: fix
keeps index/speaker/timing and rewrites only `text`
(trimmed, blank rejected); every fix appends a `Correction`
annotation quoting `before → after` with author + unix time;
unknown segment and blank author are errors, same variants.

API deltas (§7 block grows two routes; same guard stack —
staff + CSRF, admin listener only):

```
POST /apostille/api/v1/calls/{id}/segments/{seq}/fix
  {text, author?}                      # author defaults to staff:<user> from session
  → 200 {seq, text} | 400 blank text | 404 unknown call/segment
POST /apostille/api/v1/calls/{id}/segments/{seq}/annotate
  {kind: flag|note, note, author?}     # note 1..1024 chars
  → 200 {seq, kind} | 400 bad kind/blank note | 404 unknown call/segment
```

Storage: `voice_transcript` rows gain the corrected text in
place (the `Correction` annotation preserves the before-text,
so nothing is lost); annotations ride a `voice_annotation`
table keyed `(call_id, seq, created_at)` (new table — the
release crossing it carries migration notes per the ≤10
releases rule). Audit shape (extends §6, ids only — never
transcript text): `voice_segment_fix` and
`voice_segment_annotate` rows carrying actor, call id,
segment seq, annotation kind, and timestamp.

### 11.5 Review surfacing (queue-item call-ref, backlink, UI)

Quote-to-draft (§7) already inserts via
`insert_proposed_fact`; this slice makes the voice origin
visible in review instead of grep-only — with zero DDL, by
reading the audit reason the quote already writes
(`call:<call_id>:<seqs>`, the §7 fallback-mark precedent):

- Queue-item call-ref: queue enrichment parses the
  `call:<id>:<seqs>` audit reason and renders `via call
  <short-id>` next to the statement. No new column; a
  future `source_ref` column is a follow-up, not this slice.
- Call→facts backlink: `GET .../calls/{id}` gains a `facts:
  [{fact_id, topic, status}]` array listing every fact quoted
  from this call (derived from the audit-reason prefix
  match). A call with zero quotes shows `facts: []`, never
  null.
- UI contract (admin calls page): each call row links its
  quoted facts to the review queue item; each review queue
  item with a `call:` ref links back to the call detail
  (transcript + recording per §11.6 guards). Plain anchors
  with ids in hrefs — no transcript text in list payloads,
  no audio prefetch.

Permissions follow the existing gates: staff see both
directions; non-staff get 403 on either endpoint (§8
item 5 unchanged).

### 11.6 Playback blob-serve guards + 90-day sweep

Playback serves the stored blob (§4), never the Telnyx URL
(which we never persist):

```
GET /apostille/api/v1/calls/{id}/recording
  → 200 audio/mpeg (Range supported) | 404 no recording yet | 410 swept | 403 non-staff
```

- Staff + admin-listener only; `Content-Disposition:
  inline`, no caching headers that would park audio in
  shared caches.
- Byte-range support so the UI player can seek without
  re-downloading.
- `recording_status=pending` (failed download, §3) → 404
  with a staff retry affordance, not a 500.
- Swept calls (§4 retention) → 410 with the metadata row
  intact (who/when/duration/consent record survive; blob +
  transcript text are gone).

90-day sweep: `retention_sweep` gains a voice arm that
deletes blobs + transcript text older than 90 days and flips
the call to `swept` (metadata kept for audit). NOTE — OQ-3
owner decision needed: whether the voice arm folds into the
existing `retention_sweep` or ships as a voice-owned xtask
determines the exact wiring; this slice specifies the
behavior (what is deleted, what is kept, the 410 contract)
so either wiring passes the same acceptance test (§11.7.6).

### 11.7 Acceptance criteria (credential-free)

1. `cargo xtask voice-replay --scenario happy` passes with
   no `telnyx.*` config rows set: dial → initiated →
   answered → 2 segments → recording.saved → hangup leaves
   the call `completed`, two transcript rows, one blob
   addressed by its sha, and a stub action log showing dial
   + record_start + transcription_start + zero real HTTP.
2. `--scenario duplicates`, `out-of-order`, `replays`:
   terminal state never moves backward, duplicate webhook
   ids are 2xx no-ops, the stub mp3 fetch count stays 1.
3. `--scenario unknown-leg` → 2xx + drop (ids only in
   logs); `--scenario bad-signature` → 401 with no row
   written.
4. Script packs: dial with no pack and no inline questions
   uses `default.toml`; dial naming a broken pack → 422 +
   refusal audit, no stub dial recorded; a 21-question or
   281-char pack fails validation the same way.
5. Fix/annotate: fix rewrites text and appends a
   `Correction` quoting before → after; annotate stores
   flag/note; audit rows carry ids only; blank text /
   unknown seq / blank note return the §11.4 codes;
   transcript text never appears in audit rows or logs
   (grep gate).
6. Playback + sweep: `GET .../recording` streams the stub
   mp3 with ranges for staff, 403 for non-staff, 404 while
   pending, 410 after a forced sweep; the sweep keeps the
   metadata row and drops blob + transcript text.
7. Review surfacing: quoting two segments yields one
   proposed fact with a `call:<id>:<seqs>` audit reason,
   visible as `via call` in the queue; the call detail lists
   the fact and its status; the fact approves through the
   standard gate with no voice shortcut.
8. No-credentials CI gate: `cargo xtask ci` passes on a
   runner with no Telnyx keys, no `telnyx.*` rows, and no
   network route to `api.telnyx.com` (the suite fails if any
   test attempts real HTTP — localhost only). `npm run
   check`, `test`, `build:admin`, `smoke` green per §8
   item 7.

### 11.8 Creds-gated handoff checklist (config rows only)

When the author approves the slice above, going live is
sealed config rows plus the §9 account needs — no code, no
migration, no fixture edits:

1. `telnyx.api_key` — sealed secret row via the admin UI
   (never repo/env); presence flips staff endpoints from
   503 to live (§9 item 1).
2. `telnyx.from_number` — E.164 number on the account (§9
   item 2).
3. `telnyx.connection_id` — Call Control application id
   (§9 item 3).
4. `telnyx.webhook_public_key` — portal signing key (§9
   item 5); presence flips the webhook from 401-open to
   verifying.
5. Portal webhook URL — confirm it equals the §9 item 3
   single-443 form (no `:444`), failover URL set, timeout
   ~10 s.
6. Proof call: dial one synthetic number, confirm the portal
   test webhook verifies (OQ-1 final confirmation), then
   hang up and confirm the sweep math (90-day clock starts
   at `ended_at`).
7. Rollback: blanking `telnyx.api_key` returns every
   endpoint to fail-closed (503/401) with stored calls,
   transcripts, and facts untouched.

## Fake calls (W-3, `telnyx.mode = fake`)

Loopback and staging boots only (`docs/specs/config-db.md` §9). A dial
places no call: no Telnyx key or connection is needed and no phone
rings. The call row is created as usual (consent, script pack and
audit unchanged, `fake-…` control ids), a recorded WAV — two seconds
of a synthetic 440 Hz tone, mono 16-bit PCM at 16 kHz, built in code
so no audio file rides in the repo — becomes its recording, the call
completes, and the same transcribe step a real hangup triggers runs
on it (Meta transcription through the call cache). As for a real
call, it stays untranscribed with no Meta key or recorded answer, or
while `media.terms_read` is off. Hangup and record commands do not
apply to a fake call (it is already over).
