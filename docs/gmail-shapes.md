# Gmail API + OAuth2 — pinned shapes (implementation input)

- Source: live Google docs, verified 2026-10-28 (prompt 052,
  researcher report; full text in that turn's child session logs).
- Consumer: `crates/mail` (`client` + `poll`), `xtask mail-poll`.
- Status: pinned input. Re-verify against Google docs if a call
  fails after a docs change.

## Endpoint + auth

- Gmail base `https://gmail.googleapis.com/gmail/v1`, every call
  `Authorization: Bearer <access_token>`, `userId=me`.
- OAuth2 token endpoint `https://oauth2.googleapis.com/token`,
  `Content-Type: application/x-www-form-urlencoded` (never JSON).
- Scope (restricted): exactly
  `https://www.googleapis.com/auth/gmail.readonly`.

## Calls (in poll order)

1. Refresh: `POST {token_url}`
   `client_id=..&client_secret=..&refresh_token=..&grant_type=
   refresh_token` → `{"access_token": "ya29…", "expires_in":
   3599, "token_type": "Bearer"}`. `400 invalid_grant` = revoked
   or expired refresh token → re-consent, never loop.
2. Seed/reseed: `GET /gmail/v1/users/me/profile` (1 unit) →
   `{"emailAddress", "historyId": "<uint64 string>"}`.
3. Poll: `GET /gmail/v1/users/me/history?
   startHistoryId=<H>&historyTypes=messageAdded&maxResults=500`
   (2 units) → `{"history": [{"messagesAdded": [{"message":
   {"id", "threadId"}}]}], "nextPageToken?", "historyId"}`.
   `history` may be absent (nothing new) — still advance to the
   top-level `historyId`. Expired watermark → **404**.
4. Fetch: `GET /gmail/v1/users/me/messages/<id>?format=raw`
   (60 units) → `{"id", "threadId", "labelIds",
   "internalDate": "<ms string>", "raw": "<base64url, no pad>"}`.
   `raw` is the entire RFC 2822 message; sha256 over the DECODED
   bytes. `internalDate` is Gmail receipt time (unspoofable);
   the `Date:` header is sender-claimed.
5. Backfill (404 path only): `GET /gmail/v1/users/me/messages?
   q=newer_than:7d&maxResults=500` (5 units, paged), then step 4
   per id, then step 2 again (re-profile AFTER the backfill so
   mid-run arrivals re-deliver instead of skipping).

## Errors

Gmail envelope: `{"error": {"code", "message", "errors":
[{"reason"}], "status"}}`. Branch: `401` → refresh once, retry
once, then hard error; `404` on history → reseed; `429` / `403
rateLimitExceeded|userRateLimitExceeded` → back off, hold the
watermark; `403 insufficientPermissions` → wrong scope, fail
loud (re-consent).

## Flow rules (implemented in `poll`)

- Refresh per run (one extra call per 15 min beats expiry math).
- Page `history.list` fully, then ingest; advance the watermark
  only over fully processed ids (cap 200 fetches/run, then hold
  — the next tick re-lists, sha-dedup absorbs re-delivery).
- First run seeds from `getProfile` (no backfill); backfill only
  on the 404 path, bounded by `newer_than:7d` + the fetch cap.
- Trust: sender domain ∈ `domain(status='official')` and
  Gmail's stamp authenticates that domain → `official` + that
  authority. Authenticated means `dmarc=pass` with
  `header.from` equal to the parsed From domain, or any
  `dkim=pass` clause (every clause is kept; signer from
  `header.d`, else `header.i=@domain`, Gmail's shape) whose
  signer equals the From domain or is its parent or child. SPF
  alone never counts. Unauthenticated official-domain mail keeps
  its authority, capped at `community`; unknown → `community` +
  `mail-inbox`; fake → `rejected` (kept, never analyzed). Only
  the `mx.google.com` `Authentication-Results` stamp is trusted
  (topmost wins). The capture's `authenticity` records
  `verified_by` (`dmarc` / `dkim` / `none`) and the sender.
- Sender: quoted display names and comments are dropped before
  reading the From mailbox, then the last `<…>` wins; a From
  with several mailboxes has no sender (inbox, `community`).
- Body (`mime`): trust-bearing headers come from our own raw
  header scan; the readable text comes from `mail-parser`
  (nested multiparts, every charset, RFC 2047 subjects, forwarded
  `message/rfc822`, text attachments) plus the lopdf text layer
  of PDF attachments (10 MiB, 50 pages, 100k characters each;
  300k per message). A store-root marker (`mail-poll-parsing`)
  names the message being parsed: if a PDF takes the process
  down, the next run reads that message without its PDFs.
- Sidecar privacy: the text file (what the instruction model
  reads, what follower quotes are cut from) says `From: sender at
  <domain>`, drops To and display names, and passes
  `notify::pii::redact` (`[email]`, `[phone]`, `[ssn]`: the
  patterns `xtask pii-sweep` refuses). The raw message stays the
  capture blob, staff-only.
- Bounces and complaints (`report`): a `multipart/report`
  (`delivery-status`, `feedback-report`) whose sender Gmail's
  stamp authenticates suppresses every permanently failed or
  complaining recipient (`notify::suppress_address`, one audit
  row each, no address), marks a bounced outreach email
  `failed`, and is not captured. An unauthenticated one
  suppresses nobody and is captured as `rejected`. Gmail's own
  bounces come from `mailer-daemon@googlemail.com`: check on the
  first real one that its stamp reads `dkim=pass
  header.i=@googlemail.com`.
- Health: a clean run clears `mail_poll.last_error` and moves
  `last_poll_at`; a held run (rate limit, fetch cap) records why
  and moves it; a failed run (dead refresh token, Gmail refusal)
  records why and leaves `last_poll_at` alone. `/readyz` reports
  `mail`: `ok` (clean within 45 minutes), `failing`, `stale`, or
  `disabled` (no row: never ran); only `failing` and `stale`
  fail readiness. The admin mail page shows the error text.
- Outreach sends carry `Reply-To: <mail_poll.email>`, so an
  office's answer lands in the polled mailbox and threads back.
- Quotas (6,000 units/min/user): steady state is 2 units/poll;
  a 50-mail burst is ~3,000 units. Non-issue; sequential fetch.
- No MTA-STS check in the poller (API path, not SMTP MX).

## AUTHOR-ACTION items (Cloud Console, not code)

1. New Cloud project → enable Gmail API → OAuth client ID,
   type **Desktop app** (loopback redirect, no pre-registration).
2. Consent screen: request `gmail.readonly` (restricted).
   DECIDED (prompt 054, D42): publish + verify once (no Testing-
   mode weekly re-consent).
3. One-time loopback PKCE authorization per mailbox
   (`access_type=offline&prompt=consent` yields the refresh
   token); store client id/secret + refresh token in sealed
   config (`mail.client_id`, `mail.client_secret`,
   `mail.refresh_token`). Dead/rotated token → `invalid_grant`
   runbook = repeat this step.
4. Use a dedicated low-volume mailbox, not a personal inbox.
5. Send (`mailer.username`, SMTP) from that same Gmail account,
   so bounces of digests and outreach arrive where the poll reads
   them. Every SMTP send counts against one rolling 24-hour
   budget (`notify::OVERALL_DAILY_SEND_CAP`, 400) under Gmail's
   own daily limit.
6. A custom `mailer.from` domain needs its own SPF, DKIM and
   DMARC records (DNS, operator).
