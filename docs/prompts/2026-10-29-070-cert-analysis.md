# Prompt 070 — installed cert still not accepted: analysis

- Prompt timestamp (UTC): 2026-10-29T07:10:00Z
- Response timestamp (UTC): 2026-10-29T07:30:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

Got the client certiicate - error is the same. Analyze why
it's the case. "Could not connect — this page needs your
staff client certificate. Install it, then reload."

## Response

Server side proven correct; the browser is completing the
TLS handshake WITHOUT presenting any client cert:

- `openssl s_client` to prod: server advertises
  `Acceptable client certificate CA names:
  CN=handbook-admin-ca` — the issuing CA. `tls.client_ca_file`
  is right.
- `openssl verify` on prod: the issued client cert verifies
  `OK` against the on-disk CA.
- Prod audit (immutable-mode sqlite, last 2h):
  `staff_login_refused|no cert` ×3 and `admin_cert_missing`
  ×17 — i.e. handshakes completing certless (a presented
  bad cert would die at the handshake with NO audit row).
- QUIC listener reuses the same `admin_server_config` +
  CA, so no TCP/H3 split.

Same message covers both modes (403 `client cert required`
via `loginErrorCopy`, or handshake-failure TypeError), so
the user could not tell them apart — the audit split them.

Remaining causes are all browser-side (ranked): Firefox
(own NSS store, Keychain invisible); identity incomplete
in Keychain (cert without private key); cert-selection
prompt dismissed or private-key ACL denied; TLS session
resumption reusing a pre-install connection (quit +
reopen, not just reload).

Gave the author a splitting probe: curl with the .p12 at
`login/start` (+ real user_id, faked Origin) — 200 with a
challenge proves the cert verifies and isolates pure
browser selection; handshake failure means the .p12
itself. Asked which browser + whether a cert prompt ever
appeared.

## Files changed

None (analysis only).

## Commits

- (this log; hash on commit)

## Verification

- Live `s_client`, on-server `openssl verify`, and prod
  audit rows as above. No code changes to test.
