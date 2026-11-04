# Worker enrolment: self-registration, staff approval, per-worker tokens

Built 2026-11-04. It extends the plant work queue
(`docs/specs/work-queue.md`). Code: `handbook_store::work_enrol`
(store), `crates/web/src/worker.rs` (endpoints, auth, settings),
`xtask/src/worker/` (enrol and settings on the worker), and the Work
queue page's Workers tab.

Today a worker reads two secrets from a hand-written
`~/.config/apostille/worker.env`:

- `APOSTILLE_WORKER_TOKEN`: the shared `worker.token`, or a named `w1.`
  token derived from it;
- `META_API_KEY`.

It also reads the settings its job kinds need from that file. Setting up
a worker means copying secrets by hand, and revoking one worker means
rotating `worker.token`, which cuts off every worker.

This spec replaces that with:

1. **Self-registration.** A worker with no token asks to enrol and
   shows a pairing code in its log.
2. **Staff approval.** The Work queue page's **Workers** tab shows who
   is asking (user, host, build, start time, IP). Staff approve by
   typing the code.
3. **A per-worker token.** The worker receives a random token once and
   stores it 0600. Staff can revoke one worker from the Workers tab
   without touching the others.
4. **Everything else from the server.** `welcome` carries the Meta key
   (production's own `meta.api_key`) and every setting the job kinds
   need, held in memory only. The worker reads no `.env` file.

## Operator decisions (2026-11-04)

1. Workers use the same Meta key as production (`meta.api_key`); there
   is no separate worker key.
2. Staff revoke worker tokens on the **Workers** tab of the Work queue
   page.
3. A worker's name is `username@hostname`. Staff can add the approval
   date on the America/Los_Angeles calendar, as in
   `chandra@plant 2026-11-04`.
4. The worker takes no values from a `.env` file: not the token, not
   the Meta key, not any setting.

## 0. Non-goals

- No change to the job protocol after `welcome` (checkout, leases,
  heartbeats and results stay as in work-queue.md §2).
- No root on plant. The token is stored in a 0600 file; `systemd-creds`,
  TPM sealing and the persistent kernel keyring all need root.
- No asymmetric worker identity (a keypair plus a signed challenge). A
  private key in a 0600 file is no harder to steal than a token there,
  so bearer tokens over `wss://` are enough.
- Production never dials the worker; plant still only dials out.

## 1. Flow

```text
worker (no token)                      andrii.ai (public host)             staff (admin host)
  │ POST /worker/enrol  ──────────────▶ pending row, code hash
  │ ◀── 202 {id, code, poll_after_secs}
  │ log: "enrol code K7QF-2M"
  │ POST /worker/enrol/{id}/poll ─────▶ {status: pending}           Workers tab: pending row
  │   … every poll_after_secs …                                     Approve: code, name,
  │                                                                  [add date], step-up
  │ POST /worker/enrol/{id}/poll ─────▶ mint token, store hash,      ──▶ approved
  │ ◀── {status: approved, worker, token}   row → delivered
  │ write ~/.config/apostille/worker.token (0600)
  │ wss /worker/ws  Bearer w2.…  ──────▶ welcome {…, settings}
```

## 2. Worker names

A worker's name is `username@hostname`, optionally followed by a space
and a `YYYY-MM-DD` date: `chandra@plant` or `chandra@plant 2026-11-04`.

- The worker proposes `<username>@<hostname>`, both lowercased, with the
  hostname cut at its first `.` (so `plant.lan` becomes `plant`).
- On approval, staff can edit the name and tick **Add date**. That
  appends the approval day on the America/Los_Angeles calendar (the same
  conversion as `evidence.rs`), not UTC and not the browser's zone.
- The rule is `[a-z0-9._-]{1,32}@[a-z0-9._-]{1,63}( [0-9]{4}-[0-9]{2}-[0-9]{2})?`.
  It has no `:`, because `lease_owner` is `<name>:<lease token>`
  and is split on the first `:`.
- The name must be free among active credentials, so re-enrolling the
  same machine after a revoke works, and adding the date tells two
  enrolments of the same machine apart.

Today's `[a-z0-9-]{1,32}` rule and the `--name` flag (default `plant`)
go away, because the token carries the name (§4).

## 3. Public endpoints (public host, unauthenticated)

Both endpoints sit next to the WebSocket route and take JSON bodies of
at most 4 KiB.

### `POST /apostille/api/v1/worker/enrol`

Request:

| Field | Rule |
|---|---|
| `username` | the OS user (`$USER`, else `id -un`), the name rule's user part |
| `hostname` | the host's short name, the name rule's host part |
| `version` | `<semver>+<git sha>`, the same as `hello.version` |
| `started_at` | unix seconds when the worker process started |
| `capabilities` | job kinds, the same as `hello.capabilities` |
| `poll_hash` | hex SHA-256 of a 32-byte random poll secret the worker keeps |

Response `202`: `{id, code, poll_after_secs, expires_at}`.

- `id`: 16 random bytes, as hex.
- `code`: 6 Crockford base32 characters from the server's RNG, shown
  as `XXXX-XX`. Only its SHA-256 is stored, and the admin side never
  shows it: staff must read it from the worker (§5).
- `poll_after_secs` is 5; `expires_at` is now plus 15 minutes.

Refusals:

- `429` when 5 requests are already pending in total, or when the peer
  IP has made 3 requests in the past hour;
- `422` for a field that breaks its rule;
- `503` when enrolment is off (`worker.enrolment` = `off`, §7).

The server records the peer IP, from the same source as the
WebSocket's per-IP cap.

### `POST /apostille/api/v1/worker/enrol/{id}/poll`

Request: `{poll_secret}` (hex). It goes in the body, never the URL, so
it stays out of access logs. The server compares SHA-256 of the secret
with `poll_hash` in constant time. A mismatch or an unknown `id` gets
`404`.

| Row state | Response |
|---|---|
| `pending` | `200 {status: "pending", expires_at}` |
| `approved` | mint the token (§4), store its hash, row → `delivered`; `200 {status: "approved", worker, token}` |
| `denied` | `410 {status: "denied"}` |
| `delivered` | `410 {status: "delivered"}` (a token is never sent twice) |
| expired | `410 {status: "expired"}` |

The token's plaintext exists only in that one response. It is minted
at delivery, not at approval, so it is never stored. If the response is
lost in transit, the worker enrols again.

## 4. Tokens and the WebSocket

The token format is `w2.<credential id>.<secret>`: the credential id
is 16 random bytes and the secret 32 random bytes, both in hex. The name is not in the
token, because a name can contain `.` and spaces.

Authentication at upgrade (work-queue.md §2) becomes:

1. Parse `w2.<id>.<secret>` and look up the credential by `id`.
2. Compare SHA-256 of the whole token with `token_hash` in constant time.
3. Refuse with `401` when there is no match. Refuse with `401` and code
   `revoked` when the credential is revoked.
4. Read the session's worker name from the credential.
   `hello.worker` must equal that name, else the server answers
   `bad_frame` and closes.

The per-ping re-check also closes a socket whose credential was revoked
since the last ping.

**Legacy tokens are removed.** These go away:

- the shared `worker.token` row and its `/config` field;
- `w1.` named tokens and `POST /apostille/api/v1/admin/workers/token`;
- the Work page's "Token for …" control.

The worker no longer reads `worker.env` (decision 4), so nothing can
present a legacy token any more. The plant queue worker is not
installed today (checked 2026-11-04), so removing them breaks nothing
that runs.

## 5. Admin: the Workers tab

The Work queue page (`/work`) gets two tabs: **Jobs** (today's page)
and **Workers**.

### Pending requests

Each row shows:

- the proposed name (`username@hostname`) and the peer IP;
- the worker's start time, and the request's age and expiry;
- the build, flagged "other build" when its SHA differs from the
  server's;
- the capabilities.

The name, build and capabilities came from the worker, so they are
untrusted and rendered as text only.

Actions:

- **Approve** opens a form with:
  - the pairing code (required);
  - the name (prefilled, editable);
  - an **Add date** checkbox (§2);
  - a preview of the final name.

  Approving needs a fresh step-up and CSRF. A wrong code gets `422`,
  and after 3 wrong codes the request becomes `denied`. Typing the code
  ties the approval to the machine whose log staff can read; an
  impostor's request looks the same in the list.
- **Deny** needs CSRF only.

### Enrolled workers

Each row shows:

- the name, and when and by whom it was enrolled;
- when it was last seen, and whether it is connected now;
- its build and the job it is running, if any.

Action: **Revoke** (fresh step-up, CSRF). It sets `revoked_at` and
closes any open socket for that credential. The worker's lease runs
out and the job requeues, as work-queue.md describes for a lost worker.

The confirm dialog says that the worker had production's Meta key in
memory. If the machine may be compromised, rotate `meta.api_key` in
`/config` too.

Revoked credentials stay listed, greyed out, under a **Show revoked**
toggle.

### API

Admin listener, staff session, CSRF on mutations.

| Method + path | Purpose |
|---|---|
| `GET /apostille/api/v1/admin/workers/enrolments` | pending requests and credentials (`?revoked=1` includes revoked) |
| `POST /apostille/api/v1/admin/workers/enrolments/{id}/approve` | `{code, name, add_date}` → `200 {status: "approved", name}` |
| `POST /apostille/api/v1/admin/workers/enrolments/{id}/deny` | → `200 {status: "denied"}` |
| `POST /apostille/api/v1/admin/workers/credentials/{id}/revoke` | → `200 {id, revoked_at}` |

The server builds the final name. With `add_date`, it appends the
America/Los_Angeles date at the moment of approval, then checks the
name rule and that the name is free (`409` when it is taken).

## 6. Settings in `welcome` (no `.env`)

`welcome` gains `settings`: an object of strings with every value the
job kinds read, taken from server config at the time of the connect,
resolved the way the server resolves them for its own calls
(`$META_API_KEY` before `meta.api_key`, the call-cache environment
overrides, the effective `hf.mode`). The table
shows what the worker reads today and where it comes from instead.

| `settings` key | Today (from `worker.env`) | Source on the server |
|---|---|---|
| `meta_api_key` | `META_API_KEY` | `meta.api_key` (decision 1: production's key) |
| `meta_base_url` | `META_BASE_URL` | `meta.base_url` |
| `meta_cache_mode` | `META_CACHE_MODE` | `meta.cache_mode` |
| `meta_daily_call_cap` | `META_DAILY_CALL_CAP` | `meta.daily_call_cap` |
| `hf_mode` | `HF_MODE` | `hf.mode` |
| `glimmer_model` | `GLIMMER_MODEL` | new row `worker.glimmer_model` |
| `glimmer_url` | `GLIMMER_URL` | new row `worker.glimmer_url` |

An unset row is left out of `settings`, and the job kind keeps its
current default or its "needs …" error. Error messages say "set
`<row>` in /config" instead of "in worker.env".

The daily call cap is counted against each cache directory, so every
worker gets its own cap of that number, separate from production's.

On the worker:

- **No `.env` file.** `worker.env` is not read, and neither is the
  process environment, for these settings or for the token. Flags such
  as `--workdir`, `--cache-dir` and `--connect` stay as flags.
  `--run-local` (no server) takes a `--settings <json file>` with the
  same keys, so one-off runs need no `.env` either.
- **Memory only.** Settings are never written to disk and never put in
  the process environment with `set_var`. A handler that starts a child
  process passes only the values it needs in that child's environment.
  Frame logging redacts `settings.meta_api_key`.
- **Changes.** A changed row reaches each worker on its next `welcome`.
  When the digest of `settings` changes, the per-ping re-check closes
  idle sockets. A running job finishes with the values it started with.

## 7. Storage and audit (`work.sqlite`, additive)

Two new tables, created by `handbook_store::work::migrate`
(`CREATE … IF NOT EXISTS`):

- **`worker_enrolment`**: `id`, `username`, `hostname`, `version`,
  `started_at`, `capabilities_json`, `peer_ip`, `poll_hash`,
  `code_hash`, `code_tries`, `status`, `expires_at`, `created_at`,
  `decided_by`, `decided_at`, `credential_id`.
  - `status` is `pending`, `approved`, `denied` or `delivered`. Expiry
    is computed from `expires_at`, not stored.
- **`worker_credential`**: `id`, `name`, `token_hash`, `enrolment_id`,
  `created_at`, `created_by`, `last_seen_at`, `last_version`,
  `revoked_at`, `revoked_by`.
  - A unique index on `name` where `revoked_at IS NULL` allows one
    active credential per name.

New audit rows, written in the same transaction as the change:

- `worker_enrol_request`;
- `worker_enrol_approve`, with the final name;
- `worker_enrol_deny`, by staff or after 3 wrong codes;
- `worker_token_delivered`;
- `worker_revoke`.

Expired requests stay (they still count toward the per-IP limit) and
are pruned 30 days after they were filed. Their audit rows stay.

Config rows:

- Added: `worker.enrolment` (`on`/`off`, default `on`; `off` refuses
  new requests with `503`, and pending ones can still be decided),
  `worker.glimmer_model` and `worker.glimmer_url`.
- Removed: `worker.token` (§4).

## 8. Worker side (`xtask worker`)

- **Token.** The worker reads `~/.config/apostille/worker.token`. If it
  is missing, the worker enrols. A token file whose mode allows group
  or other access is refused.
- **Enrolment state.** `~/.config/apostille/worker.enrol` (0600) keeps
  `{id, poll_secret, expires_at}`. Cron starts the worker every minute,
  so a restart resumes polling the same request instead of opening a
  new one. A `410`, or a passed `expires_at`, deletes the file, and the
  next start opens a fresh request.
- **Log line.** `enrol code XXXX-XX for chandra@plant: approve it on
  Work queue → Workers`, written once per request and again every 5
  minutes.
- **Lock.** The worker holds `worker.lock` while it enrols, so a
  workdir has one open request at most.
- **Saving the token.** On approval the worker:
  1. writes the token to a temp file in `~/.config/apostille`
     (directory 0700, file 0600) and fsyncs it;
  2. renames it into place;
  3. deletes `worker.enrol`;
  4. connects.
- **Revoked.** On `401 revoked` the worker renames the token file to
  `worker.token.revoked` and enrols again. Staff see a fresh request
  and can deny it. A worker that keeps being denied enrols at most 3
  times an hour per IP (§3); setting `worker.enrolment=off` stops it
  entirely.
- **URLs.** The enrol URLs come from `--connect`: `wss://` becomes
  `https://` on the same host, with the
  `/apostille/api/v1/worker/enrol` path. The loopback-only `ws://` rule
  covers `http://` too.

Docs to update when this is built:

- `ops/plant-pipeline/README.md` § Work-queue worker: no `worker.env`
  step; approve the code from `worker.log`.
- `worker-install.sh`: drop the "missing worker.env" note.
- `docs/ops/operator-today.md` steps 2–3: no token minting and no
  `GLIMMER_MODEL` in `worker.env`; set `worker.glimmer_model` in
  `/config` instead.
- `docs/contracts/worker-frames.json` and
  `docs/contracts/admin-work-items.json`.

## 9. Threats

| Threat | Mitigation |
|---|---|
| Anyone on the internet floods enrol requests | 5 pending in total, 3 per IP per hour, 15-minute expiry, 4 KiB bodies, `worker.enrolment=off` |
| An impostor's request approved by mistake | Staff must type the code from the real worker's log; the admin UI never shows it; 3 wrong codes deny |
| Token intercepted on delivery | `https` only (loopback excepted); polling needs the poll secret that only the requester holds; the token is sent once |
| Token stolen from plant | 0600 file in a 0700 directory; per-worker revoke; only hashes on the server |
| `work.sqlite` leaks | It holds only hashes of tokens, codes and poll secrets |
| Production's Meta key leaks through a worker (decision 1) | Held in memory only and never logged; sent only to approved, unrevoked credentials; the revoke dialog prompts a `meta.api_key` rotation when compromise is possible. The cost of decision 1: a compromised worker exposes production's key, not a worker-only one |
| XSS through username or hostname | Rendered as text; the name rule limits characters at intake |

## 10. Acceptance criteria

1. A worker with no token enrols, logs a code, and, once staff approve
   it with that code, connects with a `w2.` token without a restart.
2. Killing and restarting the worker during enrolment keeps the same
   request (one pending row).
3. Three wrong codes deny the request. The admin UI never shows the
   code.
4. The proposed name is `username@hostname`. **Add date** yields
   `chandra@plant 2026-11-04` on the America/Los_Angeles date, including
   after 16:00 PST or 17:00 PDT, when UTC is already on the next day.
5. A second poll after delivery gets `410`. `work.sqlite` holds no
   plaintext token.
6. **Revoke** on the Workers tab closes the worker's socket within one
   ping interval, and the next connect with that token gets
   `401 revoked`.
7. `welcome.settings` carries `meta.api_key` and the other rows in §6,
   but only to an authenticated `w2.` session. The key appears in no
   log, frame dump or worker file.
8. A worker with a `worker.env` present ignores it: the values come
   from `welcome` only. A test plants a conflicting `worker.env` and
   checks that the job sees the server's values.
9. Shared and `w1.` tokens get `401`, and the token-mint route returns
   `404`.
10. Limits: the 6th pending request, and the 4th from one IP within an
    hour, get `429`. `worker.enrolment=off` gives `503`.

## 11. Open questions

1. **Credential expiry.** Should credentials expire (e.g. after 180
   days) and need re-approval, or last until revoked? Proposed: until
   revoked.
2. **Settings for staging.** Staging on plant runs its own server with
   its own `/config`, so a staging worker gets staging's values. Is
   that wanted, or should staging workers be refused? Proposed: allowed,
   since staging is how this gets tested.
