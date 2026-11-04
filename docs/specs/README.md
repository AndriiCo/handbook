# Specs

Design specs, written as drafts from prompt 007 on and built since.
Each index line starts with its status: **built** (the code implements
it; the spec is now the design record), **partial** (some of it is
built; the spec says which), **plan** (not built yet) or **research**
(findings that feed other specs). Each spec carries its own open
questions; the consolidated author-decision list is at the bottom of
this file.

## Index

- **partial** [app-features.md](app-features.md) — all application
  features (F1–F13) with acceptance criteria. Owns the single Viet Nam
  demo script.
- **built** [spa-separation.md](spa-separation.md) — anonymous /
  customer / admin SPA split (route groups, two-build deploy), admin
  host + passkey login, axum auth design, threat notes.
- **built** [analysis-agent.md](analysis-agent.md) — the in-project Model-API
  agent that turns captures into proposed facts/tasks (F4/F5
  drafting clause); runner shape, prompt sketch, worked examples.
- **built** [p2-forms-labels.md](p2-forms-labels.md) — P2 requirements:
  in-browser form fill-in + printpdf server rendering + FedEx/USPS
  labels (staff-manual purchase recommended). P2 = forms + labels
  only.
- **partial** [user-journeys.md](user-journeys.md) — J1–J9 critical journeys
  across tiers (a) free → (b) labels → (c) door-kit → (d)
  full-service WA; pricing questions; ES/legal-review flags.
  Tiers (c)/(d) live here as P3/ops.
- **research** [egypt-consular.md](egypt-consular.md) — verified Egypt-in-USA
  chain research with citations, USPS-only mail constraint,
  validation backlog V-1…V-19, next-country backlog.
- **research** [carrier-apis.md](carrier-apis.md) — USPS + FedEx label-API
  research (endpoints, auth, sandbox/certification, P2 payload
  mapping, API-buy vs staff-manual pricing).
- **built** [payments-billing.md](payments-billing.md) — Stripe (Checkout
  first) + Apple Pay via Stripe, invoice mirror model, Billing
  tab requirements, PCI SAQ A, refunds/disputes, P2 seam.
- **built** [secure-config.md](secure-config.md) — TPM2-sealed encrypted
  config (systemd-creds + XChaCha20-Poly1305 envelope, typed
  keys, write-only secrets, rotation, DR, Mac fallback).
- **built** [tpm-seal.md](tpm-seal.md) — software-TPM sealing abstraction
  (`Sealer` trait, software provider default, hardware stub seam,
  key hierarchy, Mac-vs-Linux matrix).
- **built** [admin-workers.md](admin-workers.md) — admin workers/captures
  API + UI (status, retry, fetch, sweep; staff-only).
- **built** [work-queue.md](work-queue.md) — production work items in
  `work.sqlite`, plant workers over an outbound WebSocket
  (`cargo xtask worker`), `research.nonparty` packs (P392).
- **built** [worker-enrolment.md](worker-enrolment.md) — workers self-register,
  staff approve them by pairing code, per-worker revocable tokens,
  settings (the Meta key included) delivered in `welcome`.
- **built** [form-framework.md](form-framework.md) — shared
  resemblance-form framework (validation, address, phone,
  country) + per-form projects under `forms/` + the US DOS
  DS-4194 pilot (goal-025 stream 3).
- **built** [config-db.md](config-db.md) — zero-env runtime config
  (29 DB keys, boot order, bootstrap remainder; staff-only).
- **built** [voice-telnyx.md](voice-telnyx.md) — Telnyx voice calls
  (Call Control client, webhook verify, recording + transcript
  UI; staff-only, consent-gated).
- **built** [single-443-quic.md](single-443-quic.md) — single-443
  Host-routed admin (444 deprecated, both-SANs cert) +
  native QUIC/HTTP-3 (quinn + h3 bridge, `quic.enabled`,
  Alt-Svc, aws-lc-rs only).
- **built** [customer-auth.md](customer-auth.md) — passkey ceremonies +
  linked-methods endpoints (tables, RP-ID decision, §6 rule
  enforcement points, staff enrolment for Phase 4).
- **partial** [shipping-design.md](shipping-design.md) — the shipping
  and label design built on carrier-apis (FedEx first).
- **built** [instruction-worker.md](instruction-worker.md) — the
  instruct sweep: new captures become instruction drafts and
  proposed fact updates (`crates/instruct`).
- **built** [atlas-harness.md](atlas-harness.md) — the HCCH research
  harness (`crates/atlas`, `cargo xtask atlas`).
- **plan** [dossier-depth.md](dossier-depth.md) — the New York-grade
  authority page (chain records, prompt v3, verify URLs).
- **built** [TOPICS.md](TOPICS.md) — the frozen fact-topic enum,
  enforced in code.
- Legal drafts (lawyer review required, do not publish):
  [tos-draft.md](../legal/tos-draft.md),
  [privacy-draft.md](../legal/privacy-draft.md).
- Visual design: [../site-theme.md](../site-theme.md) (color
  scheme, dark/light, mobile-first, i18n, robots/sitemap/
  well-known, wow effects). Named `site-theme.md`, not
  `DESIGN.md`, because this Mac's filesystem is case-insensitive
  and `docs/design.md` already exists.
- Security: [../ops/threat-model.md](../ops/threat-model.md) (the
  current threat model and the one open-findings list) and
  [../SECURITY.md](../SECURITY.md) (the claude-red baseline the code
  cites).

## Applied cross-draft resolutions

1. [analysis-agent.md](analysis-agent.md) implements the F4/F5
   drafting clause; topic enum is unioned and frozen day 2.
2. printpdf (pure Rust, verified docs.rs 0.12.8) is the P2 renderer;
   it supersedes the F7 "form-fill library TBD".
3. The verified Egypt chain supersedes the F7 sketch; the consulate
   mail hop is USPS-only (no FedEx) — the prepaid-label vision must
   honor per-hop carrier constraints.
4. The two-build split ([spa-separation.md](spa-separation.md)
   §1.2) governs; the route list in
   [site-theme.md](../site-theme.md) §1 is the public subset only.
   Exact SvelteKit config keys get verified at scaffolding.
5. WS topics: `authority:{slug}`, `call:{id}`, `submission:{id}`,
   `shipment:{id}`, `billing:{customer_ref}` (P2 endpoint nouns
   win; `order`/`filing` retired). Auth follows spa-separation
   §1.4; anonymous gets accepted facts only. F10 notify topic
   named at scaffolding. Freeze at scaffolding.
6. P2 = forms + labels only; door-kits and full-service are P3/ops
   in [user-journeys.md](user-journeys.md).
7. JS budgets: anonymous entry ≤ 40 KB Brotli (both drafts agree);
   public route chunk ≤ 30 KB; anonymous page total ≤ 80 KB;
   customer total ≤ 120 KB. Entry vs page defined in
   [site-theme.md](../site-theme.md) §2.
8. Sitemap generation merges into `xtask build-handbook` (F11);
   no separate sitemap tool.
9. Canonical PO-Box **fixtures** are `40220`/`40234` only. They
   must never appear in packs, migrations or seeded rows (the pack
   validator and `migrations_seed_no_facts_or_fixtures` enforce
   this). `PO Box 40228` in [egypt-consular.md](egypt-consular.md) is
   a **cited observation** from the superseded WA form (pending
   validation V-5), not a fixture. Amended 2026-11-03 (operator):
   Washington's own division contact page lists PO Box 40234, so a
   staff-accepted fact may carry that real box when its citation
   quotes it. Test fixtures get new values after the deadline.

## Consolidated author decisions

**Decided 2026-10-26 (prompt 008):**

1. Meta Models API spec: taken from
   https://dev.meta.ai/docs/overview.md — digested in
   [../meta-api.md](../meta-api.md) (Spark/Wonder drafting, Voice
   Transcribe realtime WS + file endpoints, SAM 3.1 on API,
   Glimmer self-host; Seamless NOT on API → P3 stays conditional).
2. Content repo: direction approved (separate repo; interim
   gitignored `store/forms/` fallback). Exact location still TBD —
   non-blocking (fallback unblocks F11/templates).
3. Admin host: **`admin.andrii.ai`** confirmed (`.ui` was a typo).
   No alternative proposed — separate origin is the right call.
4. Customer login: **passkey + Apple + Google, email
   click-to-login (magic link) as fallback only**. Linking rules
   + Apple/Google revocation signals + full portal checklist in
   [../auth-playbook.md](../auth-playbook.md). Staff method still
   open (recommend passkey or password+TOTP + mTLS).
5. Instruction photos become **property of Andrii Co.** via an
   upload-time grant; **both customer and admin** can redact
   faces/plates with Meta models (SAM 3.1 via Model API) before
   anything publishes. Consequence: Meta Model API is an approved
   processor for customer media (transcription, redaction);
   data-handling terms must be read before real media flows.
6. Login with Meta added (prompt 009): passkey/Apple/Google/
   **Meta** + magic-link fallback; customers link any number of
   methods in their profile; Meta deauthorize + data-deletion
   signals handled — see
   [../auth-playbook.md](../auth-playbook.md) §4.
7. Print roadmap (prompt 009): v1 keeps server-rendered PDF
   (printpdf); ultimate goal is HTML-print-first (validated web
   UI + styled HTML/CSS at correct page size, no PDFs touched),
   with PDF as an on-request option — see P2-R6 in
   [p2-forms-labels.md](p2-forms-labels.md).
8. Conflict policy (prompt 011): **banner** — conflicting facts
   publish with a visible banner citing both sources (F6/F11).
9. Secrets path (prompt 011): **TPM-sealed now** — build
   [secure-config.md](secure-config.md) in-slice (env-only is
   the fallback only if the box lacks TPM2).
10. Label economics (prompt 011): **carrier API buy** with
    staff-manual as automatic fallback until USPS Labels
    approval + FedEx certification land; **no prices or dates
    in UI** until the numbered table is supplied.

**Decided in assumptions-audit grill (Final — accepted
2026-10-26 PDT):**

G1. SAM-via-API redaction (F9) is must-have in-slice,
    alongside the decided carrier-buy code — not instead
    of it. Glimmer self-host stays cut (Weeks 4–5).
    Prerequisites: `META_API_KEY` Day 2, F8 upload path +
    rights grant, demo photo source (Q3), SEC review.
G2. SAM is schedule-funded (Days 4–5 parallel track,
    absorbed by the Nov 2–4 spare). Tripwire: if the
    Day 4 gate slips more than a day, the Egypt
    chain-view UI defers to the spare (chain data and
    API stay) and SAM keeps its slot.
G3. Three live Meta calls in-slice: Spark drafting,
    SAM redaction on an author-provided photo, Voice
    Transcribe file endpoint for transcript ingest.
    Terms read Day 2 (before any real media flows);
    transcript annotation/fix UI included in the Day-3
    ingest work.
G4. Full-parallel execution: the loop track (watch →
    draft → approve → publish) and the admin track
    (config → mTLS+auth → admin UI) build simultaneously
    on separate swarm tracks, with single-writer
    sequencing on shared files (migrations, `db.rs`)
    and parent-owned merges behind `ci`. Author tracks
    (DNS, key+terms, seed, client certs, root window)
    run alongside.
G5. Staging is conditional: if the box lands by Day 5
    (Sat Oct 31), cut over to staging-gated deploys plus
    the `xtask accept` suite; otherwise auto-relax to
    prod-direct with prod smokes and atomic rollback
    builds through the Nov 4 submission, and staging
    becomes a post-submission item.
G6. Postgres declined for the slice (decided 2026-10-27):
    SQLite stands (rusqlite bundled); HTTP integration
    tests run on temp SQLite. Revisit only post-submission.

**Still needed during the 5–7 days:**

11. MCP tool list + auth (public vs keyed) + rate limits.
12. HCCH feed URIs + poll cadence; 10-URL seed list + politeness
    budget (500-URL crawl is Week 3+).
13. SAM 3.1 + Glimmer self-host packaging, weights licensing,
    single-server latency budget (Weeks 4–5 work; fixtures
    stand in).
14. P2 PII store: separate `p2.sqlite` (proposed default) vs
    same-file tables; PII retention proposal (submitted 2y,
    drafts 90d, drawn PNGs 1y).
15. P2 replica-PDF acceptability research owner; wet-ink-only vs
    drawn-optional for demo (spec recommends wet-ink-only).
16. Tier b/c/d pricing, WA fee/SLA/pickup radius, $1.99-offer
    boundaries (no prices render until supplied).
17. ES scope for MVP + legal-review owner (Week 3+ work).
18. Locale URL shape (`?l=` only vs future `/es/` prefixes);
    translation provenance rule.
19. Carrier accounts + certification path (USPS Ship/EPA/Labels
    approval, FedEx developer project + label cert) — start
    Day 1; see [carrier-apis.md](carrier-apis.md) Q1–Q8.
20. Stripe + Apple Pay merchant setup (deferred past the slice;
    invoice mirror later) — see
    [payments-billing.md](payments-billing.md) OQ-P1…P8.
21. TOS/Privacy lawyer review + photo-grant language sign-off
    (parallel track, non-blocking for code) — see
    [tos-draft.md](../legal/tos-draft.md),
    [privacy-draft.md](../legal/privacy-draft.md).
22. TPM2 presence + systemd version on the provisioned box;
    KEK-escrow custodians; secret-set step-up — see
    [secure-config.md](secure-config.md) Q1–Q8.
23. Hosting details for first deploy (server IP, SSH user; key
    stays on the author's machine, host via env, never in repo).

**Decided 2026-10-27 (admin + voice + forms-framework goal):**

24. Admin host re-confirmed **`admin.andrii.ai`** (`.co` was a
    typo). Admin UI manages URL download/analysis workers.
25. Voice provider: **Telnyx** (Call Control + recording +
    transcription). Browser-initiated calls from the admin UI
    with questions-to-ask + live transcript.
26. Form-resemblance pilot: **US DOS DS-4194** (parent's pick,
    delegated by author). Common framework (validation,
    address pickers/validators, phone formatters, country
    dropdown/autocomplete) + per-form projects compiled into
    releases now, dynamic sandboxed hosting later.
27. Config: **zero env vars** except unavoidable bootstrap
    (store path); everything in DB per secure-config.md.
    Secrets sealed via a **software-TPM abstraction**,
    hardware-TPM-ready.

**Decided 2026-10-27 (single-443 + QUIC + build stamp):**

28. Admin moves off the separate 444 port onto **single-443
    Host-routed** serving (`admin.andrii.ai` → admin router),
    per the original SNI/Host design; one cert pair must carry
    both SANs. `net.admin_https_port` deprecated.
29. **Native QUIC/HTTP-3** via quinn + h3 on UDP 443 (same IPs),
    same routes over an h3→axum bridge, Alt-Svc advertised,
    `quic.enabled` DB key (staging first). No `ring` (workspace
    TLS rule) — aws-lc-rs provider throughout.
30. `/healthz` reports `0.1.0+<short-sha>` semver build metadata
    (and `/version` exposes `build` sha) so every deploy is
    identifiable on the wire.

**Decided 2026-10-28 (auth goal):**

31. Staff login: **passkey + mTLS client cert** (the open staff
    method from prompt 008 is closed; password+TOTP rejected).
    Cert identifies the device, passkey the person; either
    alone grants nothing. Invitation-gated enrolment — see
    [customer-auth.md](customer-auth.md) §5.
32. Google OAuth (Phase 2, apex-only): flat
    `/auth/google/start|callback` paths (playbook-consistent, no
    `/oauth/` segment); server-side authorization-code flow, no
    GIS JS; verification via Google tokeninfo (zero new crypto
    deps); OAuth state reuses `webauthn_challenge`
    (`kind='oauth'`) with a split binding cookie; Google-only
    unlink accepts a fresh-session proof — see
    [customer-auth.md](customer-auth.md) §9.
33. Apple + Meta OAuth (Phase 3, apex-only): flat
    `/auth/apple|meta/start|callback` paths; separate state
    kinds (`kind='apple'/'meta'`, the confused-deputy boundary)
    + per-provider binding cookies (`hb_apple`/`hb_meta`);
    shared 302 code vocabulary, provider-agnostic SPA copy;
    `jsonwebtoken 11` on `aws_lc_rs` for Apple ES256
    (verified ring-free; hand-rolled JWS rejected); Meta is
    sub-only (no email scope, no `/me` call), Graph API v26.0
    pinned — see [customer-auth.md](customer-auth.md) §§10–11.
34. Staff passkeys (Phase 4, §5): invite-gated enrolment
    (`xtask admin-invite` flags, token-hash-only rows, CN-bound
    to an issued cert) + UV-required login on the admin branch
    only; separate challenge kinds (`staff_register`/
    `staff_login`) as the branch boundary; enrol finish mints
    the staff session (mirrors customer register); offboard is
    xtask-only, cert-first, per-step transactions; step-up
    helper pilots on `set_config` (full list OPEN-17);
    OPEN-3/OPEN-4 parent-recommended (admin-assisted recovery
    only; no admin-branch OAuth) pending author confirm —
    see [customer-auth.md](customer-auth.md) §5.
35. Magic link (Phase 5, apex-only, fallback-only): POST-only
    JSON endpoints (`/auth/magic/start|consume`) — no GET
    consume, because `request_id_and_log` logs full URIs and a
    magic token is a standalone bearer (unlike the OAuth code,
    which needs the server-side client_secret); the emailed link
    carries the token in the URL fragment (`#t=`), never the
    query. New `magic_token` table (not challenge-kind reuse);
    owner resolved fresh at consume (ambiguous → fail closed);
    unverified email-hint rows flip to verified, no new
    `identity_link` rows; single 410 for miss/expired/replay;
    Origin-only guards (no CSRF echo — cross-browser consume is
    the point); mail via Gmail SMTP (author, OPEN-25) over
    ring-free lettre, host/port/creds/From configurable for a
    future own-sender swap; fallback window 7d + 1h idle
    (author-confirmed, OPEN-2) enforced via `session_usable` —
    see [customer-auth.md](customer-auth.md) §12.
36. Public serving (Slice A, O1 author-confirmed 2026-10-28):
    Svelte SPA + live API, not publish static HTML. The public
    search + dossier read `GET /api/v1/authorities` (new index:
    slug/name/country/accepted-fact-count, anonymous) and the
    existing per-slug `public_facts` (accepted-only + conflict
    banners); static builds stay audit/accept artifacts, not
    served content (build-pinned reads are a named follow-up).
    O2: canonical Viet Nam slug is `vietnam` (the pack is prod
    truth; the `viet-nam` publish-test slug follows). O3:
    `build-handbook` + `accept` run staff-on-demand for audit
    checkpoints — no daemon, no served `out_dir`.
37. P2 order container deferred (Slice D scoping): no order
    record joining form + shipment + labels in v1 — the join is
    the optional `submission_id` link on the shipment, and the
    orders/billing pages stay honest placeholders. An order
    container without money is scope without purpose; money is
    cut per the roadmap (no Stripe charging), so the container
    waits for the invoice mirror. Revisit with payments.
38. UI framework (author, 2026-10-28, OPEN-29/30/31 resolved):
    **Tailwind v4** (picked after the 223KB gate was removed the
    same day — the proposal's Option-A headroom premise fell away,
    leaving ecosystem/convention vs consolidation churn). Pilot =
    public home + dossier together (OPEN-31); Phase-0 drive-bys
    approved — login `.notice.err` to `error-bg` + print-stylesheet
    stub (OPEN-30). Themes stay CSS-var-driven (no `dark:` variants
    until a page needs non-var colors). The proposal's analysis is
    in git history (`docs/ui-framework-proposal.md`, removed
    2026-11-04).
39. D4 live-model plan approved as-is (author, 2026-10-28): no new
    binaries; pinned Meta/Gmail shapes; queue-not-pause; suppression
    A+B; Gmail historyId watermark; embedding vectors deferred.
40. §11 review-queue spec approved as-is (author, 2026-10-28);
    OQ-1..5 stay open.
41. Both prod timers enabled (author, 2026-10-28): watch (daily)
    + mail-poll (15 min) via the deploy train (root install).
42. Gmail consent: publish + verify once (author, 2026-10-28) —
    no Testing-mode weekly re-consent.
43. Setup wizard v1 is all-manual with guides (author,
    2026-10-28): no provider-OAuth login-to-configure; Gmail
    OAuth stays a future step. See [secure-config.md §9](secure-config.md).
44. Setup key is CLI-minted + sealed in DB, hash-only (author,
    2026-10-28): the one hand-configured server secret; same key
    resumes until the first staff passkey enrol burns it.
45. Setup window needs no mTLS cert (author, 2026-10-28): the
    setup key alone gates the wizard + first enrol; the window
    closes permanently at burn.
46. Admin paths carry no `/admin` segment (author, 2026-10-29):
    the host differentiates the console — API
    `/apostille/api/v1/*`, pages `/console`-style, staff login
    `/staff-login`, root redirects to `/console`. Hard cut, no
    redirects or dual-serve for the old paths.
47. `admin.require_cert` gates the whole admin host (author,
    2026-10-29): API + HTML + JS all require a CA-verified
    client cert; only the setup surface and the Telnyx webhook
    are exempt. Presence-only at the edge; registration stays
    per-route in `AdminAuth`.
48. `admin.require_cert` defaults off (parent, 2026-10-29):
    first boot has no certs, so the operator opts in after the
    CA exists; on without `tls.client_ca_file` refuses boot
    everywhere (explicit misconfiguration, fail closed).
49. Cross-origin live proof (OPEN-1, author, 2026-10-29):
    dedicated live-proof sprint immediately after staff
    enrolment unblocks; apex ↔ admin ceremony both directions
    green before admin-login sign-off — see
    [customer-auth.md](customer-auth.md) §5.
50. Staff recovery scope (OPEN-3, author, 2026-10-29):
    admin-assisted only (fresh invite, never self-service);
    TOTP/backup codes rejected; break-glass via the sealed
    envelope only (decision 59). Customer magic-link fallback
    is permanent, apex-only per §12. Folds the author outcome
    keyed "OPEN-13" (same question as OPEN-3).
51. Profile v1 is method-only (OPEN-8, author, 2026-10-29):
    linked-methods list with no masked-email display; raw
    email never stored.
52. Prod CA custody (OPEN-19, author, 2026-10-29): one offline
    encrypted copy of `ca-key.pem` held by the designated
    custodian (name TBD); live key stays root-only on the
    server; HSM deferred — see the [staff-enrolment
    runbook](../ops/staff-enrolment.md) §1.
53. Google RISC receiver now (OPEN-7, author, 2026-10-29):
    `POST /auth/google/risc` + `auth::risc` per §9.9 (author
    outcome keyed "OPEN-24", re-keyed — true OPEN-24 is
    CRL/OCSP and stays open).
54. Device cap 5 (author, 2026-10-29): at most 5 live passkeys
    per account (live `webauthn_credential` rows); 409 +
    remove-a-device copy at cap; self-service remove stays
    customer-only, staff device changes stay admin-assisted
    per decision 50. New scope (no OPEN row); true OPEN-17
    (step-up list) stays open.
55. Self-serve delete + 30-day purge (author, 2026-10-29):
    recorded scope, not built yet. New scope (no OPEN row);
    true OPEN-14 (apple/meta profile display) stays open.
56. Second staffer within 30 days of first-staffer enrol
    (author, 2026-10-29): cert + invite + UV passkey per the
    runbook; the clock starts at the first `staff_enrolled`
    audit row, dates TBD from prod. New scope; true OPEN-21
    (server-side key copies) stays open.
57. Suspend = revoke-all + re-enrol (author, 2026-10-29):
    per-CN `admin-cert-revoke` loop (or `admin-offboard` per
    staffer) until a single-command wrapper lands. New scope;
    no suspend flow exists in code yet.
58. Prod invites are 24h (author, 2026-10-29): runbook
    mandates `--days 1` on prod; code default stays 7d
    (range 1..30). Amends §5.1; true OPEN-20 (delivery
    channel) stays open, split-channel default stands.
59. Break-glass envelope + ordered drill (OPEN-23, author,
    2026-10-29): sealed offline envelope holding the
    re-mint authority; the setup-key re-mint drill in [deploy.md](../ops/deploy.md)
    is the ordered drill (author outcome keyed "OPEN-22",
    re-keyed — true OPEN-22, who may mint invites, stays
    open; quorum size for `--role admin` also stays open).
60. Already-normative confirmations (author, 2026-10-29):
    15-min single-use magic links (code `MAGIC_LINK_TTL_SECS`
    + §12); passkey UV required (spec §5 — code gap: the
    webauthn builder sets no explicit policy, enforcement
    flagged for the enrolment pass); no avatar surface and
    no `family_uid` semantic exist anywhere, so nothing to
    drop. True OPEN-4/15/28/32 stay open as written.
