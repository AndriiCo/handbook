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
