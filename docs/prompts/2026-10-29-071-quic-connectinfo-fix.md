# Prompt 071 — passkey login 500 over QUIC: missing ConnectInfo

- Prompt timestamp (UTC): 2026-10-29T07:03:00Z
- Response timestamp (UTC): 2026-10-29T07:15:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

There is an error during login with passkey at URL: https://admin.andrii.ai/apostille/api/v1/admin/auth/passkey/login/start
 Missing request extension: Extension of type `axum::extract::connect_info::ConnectInfo<core::net::socket_addr::SocketAddr>` was not found. Perhaps you forgot to add it? See `axum::Extension`.

## Response

Root cause: the browser negotiated HTTP/3, so the request
went through the QUIC h3→`oneshot` bridge (`quic::serve`),
which inserts `PeerCertificates` but never `ConnectInfo`.
TCP listeners inject it via
`into_make_service_with_connect_info`; every auth handler
(`staff_passkey::login_start`, etc.) extracts
`ConnectInfo<SocketAddr>` for rate-limit buckets — so any
auth call over QUIC 500s with exactly this error.

Assumption: prod serves QUIC (`quic.enabled`, Alt-Svc
advertised). The TCP path provably injects `ConnectInfo`
(`main.rs` serve paths), so only the QUIC path can produce
this failure.

Fix (surgical, at the omission point — no handler touched):
`serve_connection` captures `conn.remote_address()` before
the h3 handshake consumes the connection, passes it to
`serve_request`, which inserts `ConnectInfo(remote)` into
the bridged request. Deliberately did not make `ConnectInfo`
optional in the ~20 handlers: TCP always provides it.

Regression test `quic_bridge_injects_connect_info`: serves
a `ConnectInfo` echo route over a real loopback QUIC/h3
connection, asserts 200 + `127.0.0.1`.

## Files changed

- `crates/web/src/quic.rs` — imports, remote capture,
  `ConnectInfo` insert.
- `crates/web/src/main.rs` — regression test in the QUIC
  test block.

## Commits

- `4f0d195` (this work)
- (this log; hash on commit)

## Verification

- New test passes with the fix; with `quic.rs` stashed it
  FAILs (authentic failure reproduced, then fixed).
- `cargo xtask ci` green (exit 0): secrets clean, fmt,
  clippy `-D warnings`, workspace tests (36 ok suites),
  node unit tests, svelte-check 0/0, import gate, both
  builds, smoke, Playwright e2e 3/3, tailwind audit.
  (First CI run stopped at e2e — Playwright browsers were
  never installed on this machine; ran `npx playwright
  install chromium` once per the xtask comment, then full
  green.)

## Follow-ups

- Deploy to prod to fix live staff login over HTTP/3.
  Not pushed (push only on request); branch sits 57+
  commits ahead of origin/main (pre-existing state).
