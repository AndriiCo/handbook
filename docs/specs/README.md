# Specs (drafts for review)

All specs below are **drafts for author review** (prompt 007). Nothing
here is implemented yet. Each spec carries its own open questions;
the consolidated author-decision list is at the bottom of this file.

## Index

- [app-features.md](app-features.md) — all application features
  (F1–F13) with acceptance criteria. Owns the single Viet Nam
  demo script. (Its 10-day plan was superseded by the roadmap.)
- [roadmap.md](roadmap.md) — the 5–7-day swarm plan (ranked
  slice, cuts, streams S1–S7+H, day gates 2/4/6, external
  register, gaps ledger). The execution plan of record.
- [spa-separation.md](spa-separation.md) — anonymous / customer /
  admin SPA split (route groups, two-build deploy), admin host +
  mTLS + login, axum auth design, threat notes.
- [analysis-agent.md](analysis-agent.md) — the in-project Model-API
  agent that turns captures into proposed facts/tasks (F4/F5
  drafting clause); runner shape, prompt sketch, worked examples.
- [p2-forms-labels.md](p2-forms-labels.md) — P2 requirements:
  in-browser form fill-in + printpdf server rendering + FedEx/USPS
  labels (staff-manual purchase recommended). P2 = forms + labels
  only.
- [user-journeys.md](user-journeys.md) — J1–J9 critical journeys
  across tiers (a) free → (b) labels → (c) door-kit → (d)
  full-service WA; pricing questions; ES/legal-review flags.
  Tiers (c)/(d) live here as P3/ops.
- [egypt-consular.md](egypt-consular.md) — verified Egypt-in-USA
  chain research with citations, USPS-only mail constraint,
  validation backlog V-1…V-19, next-country backlog.
- [carrier-apis.md](carrier-apis.md) — USPS + FedEx label-API
  research (endpoints, auth, sandbox/certification, P2 payload
  mapping, API-buy vs staff-manual pricing).
- [payments-billing.md](payments-billing.md) — Stripe (Checkout
  first) + Apple Pay via Stripe, invoice mirror model, Billing
  tab requirements, PCI SAQ A, refunds/disputes, P2 seam.
- [secure-config.md](secure-config.md) — TPM2-sealed encrypted
  config (systemd-creds + XChaCha20-Poly1305 envelope, typed
  keys, write-only secrets, rotation, DR, Mac fallback).
- Legal drafts (lawyer review required, do not publish):
  [tos-draft.md](../legal/tos-draft.md),
  [privacy-draft.md](../legal/privacy-draft.md).
- Visual design: [../site-theme.md](../site-theme.md) (color
  scheme, dark/light, mobile-first, i18n, robots/sitemap/
  well-known, wow effects). Named `site-theme.md`, not
  `DESIGN.md`, because this Mac's filesystem is case-insensitive
  and `docs/design.md` already exists.
- Security: [../SECURITY.md](../SECURITY.md) (claude-red
  distillation for our stack + code findings F1–F8, the pre-API
  must-fix list).

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
9. Canonical PO-Box **fixtures** are `40220`/`40234` only — they
   must never leak into facts. `PO Box 40228` in
   [egypt-consular.md](egypt-consular.md) is a **cited observation**
   from the superseded WA form (pending validation V-5), not a
   fixture.

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
