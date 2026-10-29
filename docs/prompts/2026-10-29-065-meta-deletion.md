# Prompt 065 — Meta deletion callback + privacy policy

- Prompt timestamp (UTC): 2026-10-29T04:20:00Z
- Response timestamp (UTC): 2026-10-29T05:07:44Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

Meta APP requires [Privacy policy URL + User data deletion
type]. Have a correct "User data deletion" callback to
disable/delete data from our app per user request. All PII
data will be deleted, except for data required for payments /
invocing / accounting. Resolve the issue with Privacy Policy.
Have a final draft for me to review [revisions with legal
partners to follow].

## Response

Done (`ab4d052`, CI 606/0 + 3 E2E, not deployed).
Dashboard URLs for the operator:

- Privacy Policy URL: `https://andrii.ai/privacy`
- Data Deletion Request Callback:
  `https://andrii.ai/auth/meta/delete-request`

Callback (spec §11.9, closes §11.8's deletion half):
verifies `signed_request` HMAC-SHA256 under the app secret
(aws_lc, zero new deps), resolves the Meta sub via
identity_link, purges synchronously (sessions, links,
passkeys, account + submissions/shipments/labels/defaults/
captures/blobs across both DBs), mints a 128-bit hex code,
returns Meta's exact `{url, confirmation_code}`. Unknown
subs → success-shaped `unknown`; bad signatures → 400
single-shape; unconfigured → 503; `md:{ip}` 10/60. Receipts
in new `meta_deletion` table (code-only); the status page is
server-rendered HTML prose (XSS-safe). Kept: append-only
audit (opaque actors) + future payment records (none held
today — the purge exemption is coded as "invoices untouched"
by simply having no invoice table in the purge path).

Privacy page: static HTML at `/privacy` (crawler-safe, no
JS), drafted from a full PII-inventory survey (9 stores,
cookies, sharing, TTLs) — data table, cookie table, sharing
list, retention table, rights + Meta in-Facebook flow,
children, changes, contact. Bracketed owner inputs (legal
name, address, email, dates, invoice years) for counsel
review. Footer link on customer pages.

Finds while building: pristine DBs ship an id-0 capture
sentinel (squashed 005) — the eraser skips id ≤ 0 or FKs
would roll the purge back (caught by the new test).
`auth.origin` is the apex (OAuth/status bases correct);
the wizard staff-step copy calling it the admin origin is
misleading — flagged, untouched.

## Files changed

- `crates/auth/src/meta.rs` (verifier + tests),
  `passkey.rs` (`erase_identity` + test), `lib.rs`
  (`meta_deletion` table), `Cargo.toml` (base64→main)
- `crates/handbook-store/src/db.rs` (`erase_customer_data`,
  report, sentinel guard + test)
- `crates/web/src/oauth_meta.rs` (2 handlers + tests),
  `main.rs` (routes, `meta_delete_status_base`, privacy
  route + tests, 15 fixture updates)
- `crates/web/static/privacy.html` (the draft)
- `web/src/routes/(customer)/+layout.svelte` (footer link)
- `docs/specs/customer-auth.md` (§11.9),
  `docs/auth-playbook.md` (dashboard URLs)

## Commits

- `ab4d052` Meta data-deletion callback + privacy policy page
- (this log; hash on commit)

## Verification

- `cargo xtask ci`: exit 0 (Rust 606/0: +7 new; node 95/95;
  E2E 3/3) — includes signed-request roundtrip/reject,
  identity+blob purge, handler shape, receipt prose, XSS
  guard, privacy route + required-elements tests
- Meta protocol taken from the current developer docs page
  (fetched this turn)
