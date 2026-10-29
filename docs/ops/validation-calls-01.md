# Validation calls, morning 1 (operator plan)

Goal: 3 pilot calls (WA SOS → USDOS → Egypt DC), each ending in
quote-to-draft the same session. ~10 min apart; stop after 3 if
any runs long. Stretch only: Egypt NY, if DC defers to the post.

## Tonight (prereqs — all required)

Config (setup wizard step 6 "Voice (Telnyx)" or  `/config`):

- `telnyx.api_key` — Mission Control → API Keys (Call Control scope)
- `telnyx.from_number` — E.164 voice number on the account
- `telnyx.connection_id` — Voice → Connections → Call Control ID
- `telnyx.webhook_public_key` — portal signing key (wrong key = all webhooks 401)
- `telnyx.base_url` — leave default unless private interconnect

**Restart the service after setting rows** — config is read
once at boot (no hot-reload); then confirm  `/calls`
loads (a misconfigured box answers dial with 503).

Portal: voice number with voice capability; Call Control
connection with `webhook_event_url =
https://admin.andrii.ai/apostille/api/v1/calls/webhook`
(no `:444`), failover URL set, timeout ~10s; **Outbound Voice
Profile attached** (dials fail without it); budget check
(Call Control + recording + transcription rates).

Reachability: the webhook needs no client cert (host guard +
Ed25519 signature only) but MUST be reachable at
`https://admin.andrii.ai/…` over 443 tonight — send the
portal test-webhook and confirm a 2xx + webhook row (closes
spec OQ-1).

Numbers (public offices, from pack observations — re-verify
against official sites tonight; dial E.164 `+1…`):

- WA SOS: `+13607250377` (opt 3; from the superseded Dec-2024 form — stale until dialed)
- USDOS: `+12024858000`
- Egypt DC: `+12029666342`
- CA/FL/TX/NY-SoS: no numbers on file — operator lookup, out of scope for morning 1.

## Consent (every call, before question 1)

Washington is all-party consent: *"Hello, I'm calling from a
handbook research team. This call is recorded for note-taking
— is that okay?"* If refused: no recording/transcription,
handwritten notes only. Tick `consent_confirmed` in the
console (dial refuses without it). Never log or speak numbers
or transcript text outside the console.

## Question packs (checked in — dial by pack id, inline paste as fallback)

Pack ids in `crates/voice/scripts/` (validated by the
`checked_in_packs` test): `wa-sos`, `usdos-authentications`,
`egypt-embassy-dc` — same six questions per pack as below.

**WA SOS** — PO box? conflicting older addresses, want current.
Courier street address + prepaid USPS return? Per-document fee
+ expedite surcharge? Walk-ins at Town Center 3 Tumwater +
counter hours? Mail turnaround (business days) + same-day
counter? Appointment or daily cutoff?

**USDOS** — Per-document fee + is DS-4194 still required?
Mail-to address + prepaid return envelope? Counter hours at
600 19th St NW + Friday service? Walk-ins without appointment
+ Mon–Thu window? Appointment booking + slot lead time? Posted
turnarounds (mail / walk-in / emergency)?

**Egypt DC** — Per-document fees (personal / commercial / pet)?
Payment methods mail vs in person (cashier's-check-only by
mail)? States served + home-state-post rule? Required envelope
+ return postage? Turnaround mailed personal vs commercial?
Full step order for US docs used in Egypt?

## After each call

Transcript → fix misheard segments → multi-select → quote to
facts (topic + statement) → review queue → accept. Unquoted
transcripts rot — never end a call without quoting.
