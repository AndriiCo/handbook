# Security review — 2026-10-27 (parent-filed)

- Base: 7535194. Agent: read-only child, rust-review skill. Parent spot-checked cited files (SECURITY.md, demo token insert, eab-prod.txt ref) — grounded.
- Status: findings UNFIXED as of filing; H1/H2/H4 block PII on prod. Track fixes as follow-up work, not in this commit.

---

SECURITY REVIEW: Apostille Handbook @ 7535194 (read-only; rust-review skill loaded)

Verdict: clean foundation (parameterized SQL, CHECK/FK in migration 005, typed config envelope, per-topic WS authz), but NOT prod-ready for PII: live demo auth, missing WS Origin check, SSRF-open fetcher, and no rate limits are the top blockers. cargo audit: 0 vulns, 2 unmaintained warnings (bincode 1.3.3, rustls-pemfile 2.2.0). cargo deny: no deny.toml, default-config run fails on all licenses (gate unusable).

HIGH

H1 — WS bus has no Origin/CSRF check (CSWSH). crates/web/src/ws_bus.rs:69-98 never reads Origin or x-csrf, unlike the admin csrf_guard (crates/web/src/admin.rs:179-216). SECURITY.md section 1.5 explicitly requires session+Origin+CSRF before upgrade.
Exploit: victim logged in as customer visits attacker page; page opens wss://andrii.ai/apostille/ws/bus?topic=billing:cust-victim (or submission/shipment IDs — IDOR-shaped per ws.rs). Browser attaches cookies; server authorizes by cookie and streams billing events to attacker JS. Anonymous authority topics are also readable cross-site (by design, but still unwanted).
Fix: call auth::csrf::origin_allowed on upgrade; require double-submit token via subprotocol header or first-message proof within N seconds; 403 before subscribe. Log denials.

H2 — SSRF in fetch-url + robots. crates/ingest/src/fetch.rs:54-99 GETs any operator URL with no scheme/IP allowlist, no redirect cap ("no redirects policy beyond ureq defaults", line 5), no DNS pinning; robots fetch (lines 125-152) reads body unbounded (read_to_string, line 139). xtask/src/main.rs:439-490 passes --url straight through.
Exploit: `cargo xtask fetch-url --url http://169.254.169.254/...` from the server (deploy.md says fetches originate ONLY from prod IPs) reaches cloud metadata; redirect or DNS-rebinding (rbndr-style) pivots to localhost/internal services; huge robots.txt OOMs the job. Robots verdict fails open on every error (lines 127-146) so the gate never stops a fetch.
Fix: allow http/https only; resolve once, reject loopback/private/link-local/reserved/multicast (check decimal/octal/hex forms); pin resolved IP; max 3 redirects re-checked per hop; cap robots body (e.g. 64 KiB); keep fail-open only for robots-parse, never for fetch.

H3 — Upload intake: body limit disabled + trusts Content-Type. crates/web/src/admin.rs:240 disables DefaultBodyLimit; lines 484-493 buffer the whole part (field.bytes().await) before the 32 MiB check in crates/media/src/upload.rs:88-92. classify() (upload.rs:112-119) matches on client Content-Type only, no magic-byte sniff (SECURITY.md requires both).
Exploit: 2 GB upload OOMs the worker before rejection (memory DoS, unauthenticated except staff session). Polyglot (HTML/SVG labeled image/jpeg) persists under blob/; when a serve path lands without sandbox/CSP it becomes stored XSS. Stored content_type is client-supplied verbatim.
Fix: replace disable() with ~33 MiB RequestBodyLimitLayer; stream with cap; sniff magic bytes (infer crate), reject mismatch, store canonical MIME; serve future originals with Content-Disposition: attachment + sandbox CSP from non-app path.

H4 — Demo auth live on prod path. crates/web/src/main.rs:211-249 mints any customer_ref with no credential; token stored PLAINTEXT (crates/handbook-store/src/db.rs:338-351); cookie at main.rs:239 lacks Secure (and __Host- prefix); sweep comment (line 233) admits "per-IP rate limit is still Day-7 work".
Exploit: DB/file leak = all demo sessions hijacked (compare magic-link hashes in crates/auth/src/login.rs:225-227). No rate limit: unlimited mint-as-anyone + token guessing + table growth. Cookie without Secure leaks over the port-80 path. customer_ref is attacker-chosen identity with no ownership proof.
Fix: hash tokens (SHA-256 min, compare constant-time), add Secure + __Host-, add login rate limit (10/min/account + 60/min/IP, atomic decrement), and gate demo routes behind an env flag OFF in prod.

H5 — admin_cert_valid ignores expiry. crates/auth/src/lib.rs:341-350 checks revoked_at only; expires_at never read. xtask verify (xtask/src/admin_cert.rs:186-212) DOES enforce expiry — enforcement differs by path.
Exploit: expired admin client cert authenticates forever on the admin listener (crates/web/src/admin.rs:85-105), defeating the 90-day default (admin_cert.rs:119-124).
Fix: SELECT expires_at, revoked_at; require revoked_at IS NULL AND now < expires_at; pass now into admin_cert_valid; add regression test.

H6 — mTLS seam unwired; dev bypass shippable. crates/web/src/admin.rs:24-34: no layer inserts VerifiedClientCert, so strict mode fails closed (403 all admins) while ADMIN_MTLS=off (crates/web/src/main.rs:425-430) skips certs AND widens Origin/host to loopback (csrf.rs:52-60, admin.rs:163-165). The dev header x-admin-cert-sha256 (admin.rs:121-135) is only gated by the same env var.
Exploit: one env typo (ADMIN_MTLS=off) in prod downgrades admin auth to session-only and enables header-spoofed cert identity; or prod runs strict and locks out all admins (availability). Nothing refuses the dev seam in release builds (unlike config dev-KEK, crates/config/src/lib.rs:576-580).
Fix: implement rustls client-cert verifier + extension inserter; refuse ADMIN_MTLS=off unless cfg(debug_assertions); fail startup otherwise.

MEDIUM

M1 — No rate limiting anywhere; unbounded WS channel map. No limiter on demo login, WS upgrade, upload, approval, bus publish (publish body is arbitrary JSON, admin.rs:414-458). Bus (ws_bus.rs:28-56) creates one broadcast channel per distinct topic string forever: anonymous authority:* slugs (128 chars, ws.rs:29-46) let anyone grow the HashMap without bound; staff can fan out unlimited channels.
Exploit: WS-subscribe loop exhausts memory; login/upload spray; publish floods anonymous subscribers (staff-compromised account pushes arbitrary JSON to public channel — SPA must treat as untrusted).
Fix: per-IP + per-account token buckets (atomic check-and-decrement); bound channel map (validate slug against authority table, LRU-evict idle channels); cap publish body size.

M2 — Missing security headers. Public router (main.rs:123-161) and admin router (admin.rs:222-259) set only x-request-id; no HSTS/CSP/X-Content-Type-Options/X-Frame-Options/Referrer-Policy (SECURITY.md requires all). Grep confirms zero header middleware in crates/.
Exploit: MIME-sniff + clickjacking on admin SPA; first-visit TLS-strip; uploaded content executes in app origin when served.
Fix: tower-http SetResponseHeaderLayer on both listeners; HSTS preload once stable; admin frame-ancestors 'none'.

M3 — Unsalted fast hashes for PII-ish values. crates/auth/src/lib.rs:191-194 hash_str = plain SHA-256 of IPs/UAs/emails. (Magic tokens are 256-bit random so unsalted hash is acceptable; low-entropy IPs/emails are not.)
Exploit: DB leak + rainbow/dictionary re-identifies every session IP/email hash.
Fix: HMAC-SHA256 with a server-side pepper (from KEK/credential dir), or per-deploy salt; document rotation.

M4 — PII-boundary gaps. p2.sqlite schema exists (p2_001_init.sql:19-68) but has zero prod writers/readers (only db.rs tests ~1407-1495); cross-DB refs unenforceable; retention OQ-7 open (no sweep); audit lives in the other file so no atomic cross-DB commit. Originals keep EXIF incl. GPS (upload.rs:106 exif_preserved:true). pii-sweep (xtask/src/pii_sweep.rs:21-83) covers fact/capture.notes/text/*.txt only — misses audit.reason (URLs, fingerprints, filenames), task.detail (fetch failures embed URLs), source.uri, transcripts. Phone regex matches any 10-digit run (alert fatigue); no passport/DL/IBAN patterns.
Exploit: PII publishes via an unswept column; GPS leaks from "quarantined" original if a serve path exposes it.
Fix: extend sweep to audit/task/source/transcript surfaces; strip or gate EXIF at intake; decide OQ-7 retention + sweep; treat missing cross-DB parent as task+audit per db.rs:44-54.

M5 — Request log captures URIs incl. WS topics. crates/web/src/main.rs:38-65 logs method+uri at info; comment claims URLs never carry customer data, but /ws/bus?topic=billing:{customer_ref} does. Journald retains it.
Exploit: log read = customer_ref enumeration + behavior graph.
Fix: log route template or strip query; allowlist logged fields; never log topic args.

M6 — KEK handling nits. config load (crates/config/src/lib.rs:545-558) fs::reads the credential into a Vec that is never zeroized (copy is). rotate() (lines 426-465) holds all plaintexts unzeroized. xtask config-set --value (xtask/src/config.rs:208-228) puts secrets on the command line (ps/history) though --value-from-stdin exists.
Exploit: memory/disk forensics + process-table snooping recover KEK/secrets.
Fix: Zeroizing<Vec> for credential bytes; warn/refuse --value for type=secret; prefer stdin/prompt.

LOW

L1 — Admin cookie accepted on public WS. ws_bus.rs:111-112 tries ADM_COOKIE then PUB_COOKIE on the PUBLIC listener. Cross-host __Host- cookies limit blast radius in prod, but on loopback dev (shared host, ports differ, cookies ignore ports) a staff session rides public WS with Staff topic rights, widening CSWSH (H1).
Fix: accept only PUB_COOKIE on public; only ADM_COOKIE on admin.

L2 — robots.txt invites crawl of dynamic surface. crates/web/static/robots.txt:1-3 is Allow:/ with no Disallow for /api or crawl-delay.
Fix: Disallow /apostille/api/, add crawl-delay + sitemap when stable.

L3 — Secrets-scan gaps. xtask/src/secrets.rs:46-49 skips all binaries; rules miss dev KEK form (dev:<64hex>), private_key blocks, EAB blobs, npm tokens, JWT; filename rules miss *.cred and eab-*.txt (deploy/certificates flow writes eab-prod.txt, certs.md:28-29). Only tracked files scanned by default (staged path covers commits, but `ci` runs non-staged).
Fix: add dev:-KEK, private_key, .cred/EAB filename rules; run --staged + untracked check in CI.

L4 — TLS/socle nits. rustls defaults via axum-server (main.rs:357-375) with no explicit min-version/cipher suite; no HSTS (M2); cert rotation = restart with ~30s downtime (certificates.md:50-57, deploy hook restarts); test-only AcceptAny verifier is cfg(test)-gated (good, main.rs:908-949). service unit (apostille.service:60-63) correctly notes outbound is closed until Meta/carrier clients land — re-open deliberately then.
Fix: pin TLS 1.2+ with modern suites; add SIGHUP hot-reload; keep RestrictAddressFamilies in sync when clients land.

Supply chain: Cargo.lock committed; xtask audit exists but is NOT in ci (xtask/src/main.rs:141-165 runs secrets+fmt+clippy+test only) and needs network; no deny.toml so license/advisory gating is absent; web/ has no npm-audit gate; deploy pins zig hash (deploy.md:24, good). Fix: add deny.toml + `cargo deny check advisories bans sources` to ci; add npm audit; replace unmaintained rustls-pemfile 2.2.0 usage (admin_cert.rs:175-183) with rustls-pki-types PEM parsing.

What's already good: parameterized SQL everywhere; migration 005 CHECK+FK rebuild (005_constraints.sql:1-40); blob_path hex validation + atomic write; config AAD (key:version), fresh nonces, type-level secret write-only; CSRF double-submit constant-time + strict Origin allowlist; Host-derived redirect avoided (main.rs:377-396); TLS misconfiguration refuses to start (main.rs:365-375); deny_unknown_fields on mutating DTOs.

EXPLICITLY NOT COVERED: SvelteKit web/ sources (only robots/headers surface grepped; node_modules excluded); handbook-agent prompt-injection/LLM safety; forms/labels business logic (carrier payload math, idempotency, totals); publish/build-handbook output safety; handbook-store fixtures/seed data; OS-level hardening beyond reading apostille.service (no live host access); backup/DR/restore; staging plan; actual staging/prod runtime testing (no servers started, no writes per read-only mandate); crypto primitive review of XChaCha20/rustls internals; timing side-channels beyond tokens_match; full fuzzing of multipart/FTS edge cases.
