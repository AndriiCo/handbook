# Single-443 Host-routed admin + native QUIC (goal-026)

Decisions 28–30 ([README](README.md)): the 443-public + 444-admin
split was conservative bring-up, not the design. This spec
restores the original SNI/Host-based selection on ONE TLS
listener and adds native QUIC/HTTP-3 on the same port.

## 1. Part A — single 443

### 1.1 Host-routing design

One TCP+TLS listener on `net.https_port` (443) per bind IP
replaces the 443-public + 444-admin pair. Routing happens AFTER
the TLS handshake, on the HTTP `Host` header (no SNI dispatch:
one cert pair carries both SANs, so SNI selection buys
nothing):

- `Host: admin.andrii.ai` (case-insensitive, optional `:port`
  suffix stripped) → merged admin router: `admin::router` +
  `admin_workers::router` + `admin_voice::staff_router` +
  `admin_voice::webhook_router`, with the existing
  `mtls::inject_verified_cert` fingerprint layer on this branch
  ONLY.
- Anything else (public host, unknown host, missing host) →
  public router. Unknown hosts fall through to public (fail
  open to the harmless branch; the admin branch keeps its own
  host guard + session + cert checks, so misrouting can never
  grant admin access).

The admin branch keeps every existing layer untouched
(host guard, `AdminAuth`, CSRF/Origin): Host routing is a
dispatch, not an auth change. Auth semantics are unchanged
(401 no/expired session, 403 wrong role / missing or
unregistered cert / wrong host inside the admin branch).

Implementation: a `host_router(public, admin)` whose fallback
dispatches by `Host` and `oneshot`s the chosen branch. Before
forwarding to the PUBLIC branch the dispatcher strips the
`PeerCertificates` request extension the acceptor captured, so
the public branch never sees client-cert state even though the
handshake is shared.

Plain-HTTP dev mode (`serve_http`, ports 3000/3001) is
UNCHANGED: two listeners stay for local dev (no certs, no SAN
issues, no reason to churn the dev loop).

### 1.2 TLS handshake shape

The single listener reuses the ADMIN TLS shape for every
connection: `mtls::admin_server_config` + `MtlsAcceptor`:

- Client certs stay OPTIONAL at the handshake
  (`allow_unauthenticated`, as today): no cert → handshake
  still completes; a cert from a wrong CA → handshake failure.
- Presence + registration stay app-layer checks on the admin
  branch (Telnyx webhook holds no cert; cert-less staff gets
  an auditable HTTP 403, not a TLS alert).
- The public branch never reads cert state (extension
  stripped at dispatch; no fingerprint layer on that branch).

### 1.3 One cert pair, both SANs

The single listener serves one certificate for both hosts, so
the pair MUST carry both `andrii.ai` and `admin.andrii.ai`
SANs (the ACME default `acme.domains` already lists both).
Boot-time check: after loading the pair (explicit files or
ACME), parse the leaf with `x509-parser` and emit a LOUD
`eprintln!` warning when the admin SAN is missing. Warning,
not refusal: refusing would brick staging before the reissue
lands (see §5); the missing SAN only breaks browsers on the
admin host (curl `-k` and enrollment still work).

### 1.4 Deprecated key

`net.admin_https_port` is DEPRECATED: the key stays registered
(unknown-key writes would confuse operators mid-rollout), its
value is still parsed, but NO listener is bound to it. When
the row is SET (any value), boot logs a loud deprecation
warning naming `net.https_port` as the replacement. Fresh and
existing DBs that never set the key boot silently.

## 2. Part B — native QUIC

### 2.1 Architecture

- One `quinn` UDP endpoint per bind IP on the SAME port as
  `net.https_port` (UDP 443), reusing `tls.cert_file` /
  `tls.key_file` (or the ACME pair). No new cert material.
- `h3` server on each endpoint; every HTTP/3 request is
  bridged to the SAME merged Host-routed axum `Router` via
  `Router::oneshot` (request bytes in, response bytes out).
- QUIC is gated by the new bool key `quic.enabled`, default
  FALSE. Staging enables first; prod flips after soak (§5).
  Disabled → no UDP socket is bound at all.
- Body policy: request bodies are fully buffered before
  dispatch; response bodies are fully collected before
  writing. Our payloads are small (JSON APIs, SPA shells);
  streaming is a later optimization, not a blocker.
- WebSocket stays TCP-only: h3 requests under
  `/apostille/ws/` get an explicit `501` JSON error naming
  TCP. Browsers open a separate TCP WebSocket; acceptable —
  no WS-over-H3 (RFC 9220 extended CONNECT) in this slice.

### 2.2 mTLS over QUIC

The QUIC crypto config is the same rustls `ServerConfig` the
TCP listener uses (`mtls::admin_server_config` with
`tls.client_ca_file` when set, still `allow_unauthenticated`).
Per connection, the bridge reads the peer identity quinn
captured (`peer_identity` → rustls `CertificateDer` chain)
and inserts it as the SAME `PeerCertificates` extension the
TCP acceptor inserts. From there the path is IDENTICAL: Host
dispatch → admin branch → `inject_verified_cert` →
`AdminAuth` (same 401/403 semantics over QUIC as TCP);
public branch has the extension stripped. No-CA and cert-less
connections behave exactly as on TCP.

### 2.3 Crypto provider (HARD RULE)

Workspace TLS rule: aws-lc-rs everywhere, no `ring`-selected
features. `quinn` is pinned with
`default-features = false, features = ["runtime-tokio",
"rustls-aws-lc-rs"]` (NOT the default `rustls-ring`);
`h3-quinn` already depends on quinn with
`default-features = false`, so the workspace controls the
provider. Verified post-build via
`cargo tree -e features -p quinn-proto` (aws-lc-rs present,
ring absent from the quinn/h3 subgraph) and
`cargo tree -i ring` (no NEW edges vs the pre-QUIC baseline:
`ring` stays only under `rcgen 0.13` via `acme`/dev-deps —
pre-existing, untouched by this change).

### 2.4 Alt-Svc

TCP responses carry `Alt-Svc: h3=":<https_port>"` ONLY when
`quic.enabled` is true (advertising h3 with no UDP listener
would black-hole upgrade attempts). One middleware on the
merged TLS router; dev HTTP and QUIC-disabled boots send no
Alt-Svc.

## 3. Key changes (all additive)

| Key | Type | Default | Change |
|---|---|---|---|
| `quic.enabled` | bool | `false` | NEW: bind UDP + serve h3 + send Alt-Svc. |
| `net.admin_https_port` | text (u16) | `444` | DEPRECATED: parsed, warned-on when set, never bound. |

Registry grows 30 → 31 keys (5 bool / 24 text / 2 secret).

## 4. Acceptance criteria

- Host matrix (hermetic, `oneshot`): admin host → admin
  router (401 without session, 403 without cert in strict
  mode, 200 staff+cert); public host → public router;
  unknown/missing host → public router; public branch never
  observes `PeerCertificates` (assert the extension is gone).
- Port 444 is never bound in TLS mode; setting
  `net.admin_https_port` produces the deprecation warning;
  unset stays silent.
- Cert without the admin SAN → loud boot warning naming the
  host; cert with both SANs → silent.
- QUIC e2e (loopback UDP, port 0, rcgen certs): h3 GET
  `/apostille/healthz` and `/apostille/version` → 200 with
  the version+sha body; admin Host branch over QUIC
  (mTLS allow → 200 path, deny → 403/401 path);
  `quic.enabled=false` binds no UDP socket (bind-probe:
  a fresh UDP bind on the target port succeeds iff disabled).
- Alt-Svc present on TCP responses iff `quic.enabled`.
- `cargo xtask ci` green (fmt, clippy `-D warnings`, tests);
  `ring` edge check per §2.3.

## 5. Staging/prod rollout order

1. **Land + CI.** Merge this change; staging/prod DBs need
   NO key changes (defaults preserve behavior except the 444
   listener, which closes — intended).
2. **Staging TLS ports first.** Deploy to staging; confirm
   ONE `listening on https://…:443` line per IP, NO `:444`
   line; confirm the admin SAN warning state matches the
   staging cert (ACME staging pair should already carry both
   SANs via `acme.domains`).
3. **Prod SAN reissue (dependency).** BEFORE the prod flip,
   the prod cert MUST carry the `admin.andrii.ai` SAN
   (in-binary ACME path). Without it, browsers on the admin
   host get a name-mismatch error (loud boot warning fires
   as the backstop signal).
4. **Prod flip.** Deploy; same log assertions; run the
   mTLS enrollment probes from the retired mTLS seam spec §3 against
   `https://admin.andrii.ai/apostille` (no `:444`) —
   NOTE: that doc still shows `:444` URLs and needs a
   parent follow-up edit (out of this change's scope).
5. **QUIC staging soak.** `config-set quic.enabled=true`
   on staging ONLY; verify UDP 443 serves h3 (healthz/
   version 200), Alt-Svc appears on TCP, mTLS-over-QUIC
   allow/deny matches TCP; firewall opens UDP 443.
6. **QUIC prod.** After soak, flip `quic.enabled` on prod;
   same verification; close the 444 firewall opening
   (moot after step 4).

## 6. Open questions

- O1. Should the missing-admin-SAN case REFUSE the boot on
  prod after the reissue lands (fail-closed), or stay a
  warning forever? (Owner: author; blocks nothing — warning
  ships first.)
- O2. WS-over-H3 (RFC 9220) vs TCP-fallback forever: is the
  501 + separate-TCP-WS split acceptable long-term, or
  scheduled? (Owner: author; blocks nothing.)
- O3. QUIC observability: qlog / connection metrics — which
  sink (journald fields vs separate sagrada)? (Owner: parent;
  blocks step 6 only.)
- O4. the retired mTLS seam spec §2.4/§3 `:444` URLs + the §3 step-4
  log line: parent follow-up edit after this merges.
  (Owner: parent; blocks step 4 docs only.)
