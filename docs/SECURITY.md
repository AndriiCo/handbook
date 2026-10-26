# Security Baseline (draft for review)

- Status: draft (prompt 007, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child
  session log). Researcher read in full: `README.md`, `AGENTS.md`,
  `docs/design.md`, `docs/web-stack.md`, prompt-001 log, all current
  Rust sources and the migration. External:
  `https://github.com/SnailSploit/claude-red` — full tree via GitHub
  API (76 `SKILL.md` files across 22 categories) plus full contents of
  the 14 most stack-relevant skills: `offensive-sqli`,
  `offensive-xss`, `offensive-idor`, `offensive-file-upload`,
  `offensive-ssrf`, `offensive-api-security`, `offensive-api-abuse`,
  `offensive-jwt`, `offensive-oauth` (head), `offensive-race-condition`,
  `offensive-business-logic`, `offensive-supply-chain`,
  `offensive-cicd-secrets`, `offensive-tls-attacks`,
  `offensive-deserialization`.
- Findings F1–F8 below are the pre-API must-fix list for the
  implementation plan.

## 1. Auth-shape verdicts (confirm or correct, with reasons)

1. **Opaque session cookies — CONFIRM.** Correct choice over JWT for
   this stack. The `offensive-jwt` skill catalogs what you avoid
   entirely by going opaque: `alg=none`, RS256→HS256 confusion,
   `kid`/`jku` injection, weak-secret brute force, un-expiring tokens.
   Opaque random tokens (≥256-bit, CSPRNG) verified by DB/hash lookup
   have none of these classes. Revocation is a row delete, not a
   deny-list bolted onto a stateless design.
2. **`__Host-hb_pub` / `__Host-hb_adm` split — CONFIRM with one
   correction.** Two cookies enforcing two session namespaces
   (public/customer vs admin) is good defense-in-depth: theft of a
   customer session never yields admin access, and each can carry its
   own lifetime/flags. The `__Host-` prefix is the right call: it
   forces `Secure`, `Path=/`, and no `Domain`, which kills subdomain
   cookie-tossing. **Correction:** `__Host-` cookies require `Path=/`,
   but the app serves under `/apostille` — that is fine (cookie path
   `/` still sent to sub-path), just do not try to scope the cookie
   path to `/apostille`. Also set `HttpOnly` + `SameSite=Lax` on both;
   `SameSite` alone is not CSRF protection (the
   `offensive-api-security` skill notes top-level GET CSRF and
   Lax-bypass timing), so keep the token layer below.
3. **Roles anonymous/customer/staff/admin — CONFIRM, enforce server-side
   per object.** Four roles fit the design (public handbook readers,
   uploading customers, validating staff, admins). Per
   `offensive-idor`/`offensive-api-security` (BOLA is OWASP API #1):
   roles must be checked on every object access, never inferred from
   IDs, hidden fields, or JS-bundle routes (`offensive-business-logic`
   explicitly hunts buried admin routes in client bundles — SvelteKit
   route groups `(public)/(customer)/(admin)` are a build-time
   convenience, not an authorization boundary; the axum API re-checks
   role on every route).
4. **CSRF double-submit + Origin checks — CONFIRM, with ordering.**
   Defense-in-depth is right: strict `Origin`/`Referer` allowlist check
   first (cheap, kills cross-site POST/WS), then double-submit token
   comparison (covers same-origin-adjacent edge cases and non-browser
   clients that omit Origin). Reason for both: `fetch()` same-origin
   always sends Origin in modern browsers, but some native/upload
   clients may not — the token covers those. Use constant-time
   comparison, rotate the CSRF token on privilege change (login, role
   escalation).
5. **WS upgrade validates session first — CONFIRM, and this is the
   highest-priority auth item.** `offensive-api-security`'s WebSocket
   section tests exactly the failure mode: missing Origin validation →
   cross-site WebSocket hijacking (CSWSH), where the victim's browser
   carries cookies to `wss://` and the attacker rides the socket. Rule:
   on upgrade, validate (a) session cookie, (b) Origin allowlist, (c)
   CSRF token (subprotocol header or first-message proof — cookies
   alone are insufficient because the browser attaches them cross-site),
   and only then subscribe. Reject unauthenticated upgrades with 403
   before spawning send/recv tasks.
6. **Per-topic authz (`authority:{slug}`, `call:{id}`) — CONFIRM,
   mandatory.** Broadcast topics are IDOR-shaped: `offensive-idor`'s
   first move is swapping IDs in requests; here that means subscribing
   to `call:{other-id}`. Each subscribe/unsubscribe must re-check: role
   permits the topic class (call console = staff/admin only) AND the
   principal is entitled to that specific instance (operator assigned
   to that call; customer only to their own order topics). Never let
   the client declare its own topic list without server-side filtering;
   log denied subscriptions (they are attacker-shaped probes per the
   skill's Defender View).
7. **`admin_cert` table + mTLS on admin host — CONFIRM with scoping.**
   mTLS for the admin host is strong (kills credential-stuffing and
   password-spray against the highest-value surface;
   `offensive-jwt`'s mitigations section explicitly recommends mTLS
   token-binding as top-tier). But: (a) mTLS must be terminated where
   you control the client-CA — Caddy can do this (`tls { client_auth {
   mode require_and_verify … } }`), forwarded to axum as a verified
   header the app trusts only from localhost; (b) keep login *in
   addition* (cert = device/operator binding, login = identity/audit
   actor) — a shared admin workstation cert alone destroys attribution
   in the `audit` table; (c) `admin_cert` table should store
   fingerprint + operator + issued/revoked timestamps; check revocation
   on every admin request, not just at TLS handshake (Caddy validates
   chain, app checks revocation). Do not put mTLS on the
   public/customer host — friction with zero threat-model gain.
8. **Audit everything — CONFIRM, extend slightly.** Design §6 already
   appends `audit` rows for content changes; extend to auth/security
   events: logins (success/fail), logouts, role changes, cert
   revoke/issue, denied WS subscriptions, denied topic publishes,
   rate-limit trips, trust promotions, retention deletions. Keep the
   append-only rule (no UPDATE/DELETE path — enforce with a SQLite
   trigger too, cheap and survives code bugs). Ship audit rows off the
   single server periodically; a local-only audit log dies with the box
   it is meant to explain.

## 2. Day-1 SECURITY BASELINE (distilled from claude-red for our stack)

**Auth/session.** Opaque 256-bit tokens, `__Host-` prefix,
`Secure/HttpOnly/SameSite=Lax`; server-side session store with absolute
+ idle timeouts (customer idle 24h ok; admin idle ≤30 min); rotate
session id and CSRF token on login and on role change; single-flight
refresh semantics; constant-time token compare; lockout/backoff on login
(see rate limits). Customer methods (decided prompts 008–009):
passkey, Apple, Google, Meta, magic-link fallback — full portal
checklist, linking
rules, and revocation-signal handling in
[auth-playbook.md](auth-playbook.md). Linking invariants: identities
table (one account ↔ many verified identities); link on
verified-email equality only; Apple private-relay never matches by
string; no silent auto-merge (explicit link flow with step-up);
first-login creates a fresh account (kills pre-account takeover);
last method cannot be unlinked; every link/unlink/signal writes an
audit row.

**Input validation.** Every axum extractor validates: reject unknown
JSON fields (`serde(deny_unknown_fields)` on mutating DTOs — kills mass
assignment, `offensive-api-security` BFLA/mass-assignment section),
whitelist enums for all `kind`/`status`/`topic` strings (the schema has
~15 such vocabularies), length caps on every string (statement, notes,
comment), integer ranges on bytes/ids, slug regex
`^[a-z0-9-]{1,64}$`. Validate WS message envelopes the same way —
`offensive-sqli` explicitly lists WebSocket messages as an injection
vector. Never trust client-computed totals, prices, quantities,
currency (business-logic skill: negative/zero/float quantities,
skipped checkout steps — the FedEx-label/prepay flow must be a
server-side state machine; replay of a one-time action requires
idempotency keys).

**SQL safety.** rusqlite with bound parameters everywhere (current code
already does this — keep the rule absolute); no `format!`-built SQL, no
dynamic ORDER BY/sort strings from clients (`offensive-sqli`
REST-filter probe: `"sort": "name'; DROP TABLE users--"`); FTS5
queries need `quote()`/parameterized MATCH — FTS5 has its own
syntax-injection (query operators) distinct from SQLi; enable `PRAGMA
foreign_keys=ON` when FKs land (see findings); error responses never
include SQLite messages (error-based detection relies on leaked syntax
errors).

**Secrets.** Env-only (`STORE_PATH`, `BIND_ADDR`, `PORT`, `BASE_PATH`,
`META_API_KEY`, session secret, client-CA material); never in
repo/logs/errors; `offensive-cicd-secrets` patterns to block: no
`/debug/env` endpoints, no error pages echoing env, structured logs
with a PII/secret allowlist rather than free-form Debug dumps.
Single-server note: secrets live in a root-only env file (0600) read
by the service unit, not in shell history. Rotate Meta API key on any
suspected log leak.

**Headers (via tower-http + Caddy).** `Strict-Transport-Security`
(preload once stable), `Content-Security-Policy` (SvelteKit SPA:
`default-src 'self'`, no inline scripts beyond Svelte's hashed output,
`connect-src 'self' wss:` same-host), `X-Content-Type-Options:
nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`,
`X-Frame-Options: DENY` (or `frame-ancestors 'none'`),
`Permissions-Policy` minimal, `Cross-Origin-Opener-Policy:
same-origin`. Serve uploaded/customer content with
`Content-Security-Policy: sandbox` + `Content-Disposition: attachment`
where possible and never from the app origin if avoidable (file-upload
skill: separate domain/CDN for user content kills stored-XSS
escalation; at minimum `X-Content-Type-Options` + no-sniff +
non-executable store dir).

**CSRF/WS-Origin.** API mutations: Origin allowlist
(`https://andrii.ai`, admin host) + double-submit token. WS: same
Origin check + auth-before-subscribe + per-topic authz (§1.5–1.6). No
`Access-Control-Allow-Origin: *`, no reflected-Origin CORS; SvelteKit
SPA is same-origin served by axum so CORS should be deny-by-default
with an explicit allowlist if a second origin ever appears.

**Dependencies.** `Cargo.lock` committed (already done); `cargo audit`
(or `cargo-deny`) added to `cargo xtask ci`; Node side (SvelteKit
build): lockfile committed, `npm audit`/Dependabot on, `postinstall`
scripts forbidden in our own packages and reviewed in deps
(`offensive-supply-chain`: postinstall hooks are the standard npm
persistence). Pure-Rust rule (web-stack §1) already shrinks the
native-dependency attack surface — keep it. Pin the Caddy binary
version + checksum at install.

**Logging/PII.** Structured JSON logs; log actor+action+entity for
mutations (mirrors `audit`), but never log: session tokens, CSRF
tokens, cookies, Authorization headers, document photos content, full
emails, transcripts verbatim (log capture ids + hashes + offsets
instead). Instruction photos become Andrii Co. property via an
upload-time grant (decided prompt 008); both customer and admin can
redact faces/plates with Meta models (SAM 3.1 via Model API) before
anything publishes — unredacted originals stay PII-vault
(staff-role-only reads, every read audited) until a redacted
derivative exists. Meta Model API is therefore an approved processor
for customer media (transcription, redaction) — read its
data-handling terms before real media flows; no other third party
ever receives customer media. EXIF-stripping for derivatives stands
(design §2).
Transcript `ignore` ranges may themselves contain PII — same handling.

**Rate limits.** Login + WS-upgrade + upload + comment/submit endpoints
all limited per IP *and* per account (per-IP alone lets one IP rotate
accounts; per-account alone lets a botnet spray one account).
Suggested day-1 values: login 10/min/account + 60/min/IP with
exponential backoff; WS upgrades 30/min/IP; uploads 20/hour/customer;
public API reads generous (600/min/IP) with 429 + `Retry-After`.
Race-condition skill adds: rate-limit *check and decrement must be
atomic* (SQLite `UPDATE … WHERE quota>0` in one statement, or a token
bucket behind a mutex — never read-then-write across awaits).

**Uploads/media.** Customer photos/videos, scans, comments are the top
attacker-controlled input. Apply the file-upload skill's remediation
list: allowlist extensions AND magic-byte sniffing (extension lies,
content-type lies); hard size caps (photo ≤25 MB, video ≤500 MB)
enforced by streaming reader, not after full buffering; random
server-side filenames (content-addressed sha256 already does this —
good); store outside web root with no-execute (already the design:
`store/blob/…` served only through axum handlers, never `ServeDir`
directly); re-encode/strip EXIF for derivatives (decided); run
image/video converters sandboxed; filename never interpolated into
shell/SQL/paths (command-injection-via-filename and traversal:
`blob_path()` takes only hex digests — keep that invariant, see
finding F3). Quarantine-until-validated: community uploads stay
`trust=community`, unreachable from public pages until promotion.

**Supply chain.** Beyond deps: build on the server via `git pull`
means the server's git remote + deploy path is the supply chain —
protect with signed commits (or at minimum push restrictions), deploy
from a tag, record `code_git_commit` in every `build`/`audit` row
(already designed). Deploy access uses the operator's local SSH key
(key stays on the author's machine, never in repo; server host via
env, e.g. `APOSTILLE_HOST`); no deploy credentials in the tree. No CDN JS (already decided: vendor
htmx-equivalents; SvelteKit bundle is self-built). `robots.txt` gate
for crawls (design §3) doubles as legal hygiene. WARC ingest of
third-party sites is SSRF-shaped: the fetcher must block
private/metadata IPs, pin DNS per-fetch (anti-rebinding), cap redirect
hops and response sizes — the SSRF skill's bypass list
(decimal/octal/hex IP encodings, DNS rebinding via `rbndr.us`-style
double-resolution, redirect to `169.254.169.254`) is the test plan for
`xtask fetch-url` when it lands.

## 3. Code review findings (all bodies read; paths exact)

**F1 — `retention_sweep` is not atomic (medium).**
`crates/handbook/src/db.rs:59-96` loops per-capture: `remove_file` →
`DELETE` → `UPDATE` → `INSERT audit`, with no transaction. A crash
mid-loop leaves blob deleted but row present (or row deleted but
session link dangling, or deletion unaudited). Fix: wrap the loop in
`conn.transaction()` / immediate transaction; audit row inside the same
txn. Also `cutoff = now - 365*86400` can underflow on absurd `now` —
`saturating_sub`.

**F2 — Path traversal via `capture.path` on delete (medium).**
`crates/handbook/src/db.rs:76`: `store.root().join(path)` where `path`
comes from the DB. Today only trusted code writes it, but any future
write path (import tools, API) that stores `../../etc/x` turns the
sweep into an arbitrary-file delete (`full.exists()` +
`remove_file`). Fix: canonicalize and assert
`starts_with(store.root())`, or delete by `blob_path(sha256)` (already
hex-constrained) instead of the stored `path`.

**F3 — `blob_path` panics on short/malicious input (low).**
`crates/handbook/src/store.rs:49-56`: `&sha_hex[..2]`/`[2..4]`/`[4..]`
panics if `len<4` (only a `debug_assert` guards) and slices on byte
indices. Any caller passing user-influenced strings (future read-by-hash
API) gets a panic = 500/DoS. Fix: validate `len==64 && all hex`,
return `Result`/`Option`.

**F4 — `write_blob` TOCTOU + non-atomic write (low).**
`crates/handbook/src/store.rs:60-71`: `path.exists()` then `fs::write`
— two processes can race (benign: same bytes, same hash,
last-writer-wins identical content), but `fs::write` truncates in
place, so a concurrent reader sees a torn file. Fix day-1-cheap: write
to temp + `rename` (atomic on both macOS APFS and ext4).

**F5 — No FK enforcement + no CHECK constraints (low now, grows with
API).** `crates/handbook/migrations/001_init.sql:3` explicitly skips
FKs ("checked in code") — but no code checks them yet:
`capture.authority_id`, `fact.capture_id`, `task.batch_id`,
`call_session.*_capture_id` can all dangle; `kind`/`status`/`trust`/
`consent`/`visibility` accept any string (an API typo or attacker value
like `trust='official'` on a community upload bypasses the tier model
in §3). Fix: add `CHECK (…)` constraints for the ~15 fixed vocabularies
now (cheap, no framework needed); add real FKs (with `PRAGMA
foreign_keys=ON` in `open()`) before the API writes these tables.

**F6 — `flag()` CLI parsing takes the last occurrence silently +
`--actor` unaudited default (info).** `xtask/src/main.rs:47-55,144`:
`find_map` over windows returns the *first* match actually (find_map
stops at first) — duplicate flags are silently ignored rather than
rejected; `--actor` defaults to `"xtask"`, so unattributed sweeps write
`actor='xtask'` audit rows. Fix: reject duplicate flags; require
`--actor` (or inject `$USER`) for any command writing `audit`.

**F7 — `lib.rs` surface is clean; `db::open` needs hardening flags
(info).** `crates/handbook/src/lib.rs:1-23` is docs + re-exports only,
no findings. `db.rs:17-19` `Connection::open` uses SQLite defaults:
set `PRAGMA journal_mode=WAL` (single writer/WS readers concurrency),
`PRAGMA busy_timeout=5000` (avoid SQLITE_BUSY under WS fan-out),
`PRAGMA synchronous=NORMAL`, and `foreign_keys=ON` (with F5). Also
`migrate()` runs `execute_batch` outside a txn — wrap it so a
half-applied 001 can't strand the DB.

**F8 — Test-only issue, no prod impact:** `db.rs:200`
`.to_str().unwrap()` on test paths — fine in tests, but the
production import path must handle non-UTF8 filenames explicitly (store
`path` as given vs normalized — decide before `import-file` lands).

No SQLi (all queries parameterized), no secrets, no unsafe, no network
code yet — the current tree is a clean foundation; F1–F5 are the
pre-API must-fix list.

## 4. Open questions

1. **Meta Models API spec unknown** — transcription endpoint, auth
   scheme, streaming vs batch, PII/data-handling terms?
2. **Content-repo location** — where does the separate content repo
   live; who can push; how do `content_git_commit` hashes get verified
   at build?
3. **Photo retention undecided** — only voice (365d) decided; retention
   for customer photos/videos/scans?
4. ~~Admin host typo~~ — decided prompt 008: `admin.andrii.ai`.
5. ~~Customer login methods~~ — decided prompts 008–009
   (passkey/Apple/Google/Meta + magic-link fallback; see
   [auth-playbook.md](auth-playbook.md)). Staff method still open:
   passkey or password+TOTP (+ mTLS)?
6. **Customer PII boundary** — which uploads are PII-vault (ID
   documents?) vs public-derivable (route photos); who may read
   originals?
7. **WS fallback scope** — is SSE fallback in scope for day 1, or
   WS-only with documented degradation?
8. **Backup/DR for the single server** — SQLite + `store/` backup
   schedule, encryption, restore test; audit-log offsite shipping?
