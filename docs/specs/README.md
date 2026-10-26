# Specs (drafts for review)

All specs below are **drafts for author review** (prompt 007). Nothing
here is implemented yet. Each spec carries its own open questions;
the consolidated author-decision list is at the bottom of this file.

## Index

- [app-features.md](app-features.md) — all application features
  (F1–F13) with acceptance criteria + 10-day parallel workstream
  plan (agent cohort: SPEC/CODE/SEC/TEST lanes, streams S1–S7,
  demo gates day 4/6/8/10). Owns the single Viet Nam demo script.
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
   `shipment:{id}` (P2 endpoint nouns win; `order`/`filing` retired).
   Auth follows spa-separation §1.4. Freeze at scaffolding.
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

## Consolidated author decisions (blocking first)

**Blockers (implementation cannot start without these):**

1. Meta Models API spec: endpoint/auth, transcription
   (streaming?), drafting models, SeamlessStreaming availability.
2. Content-repo location (form-template home, country packs,
   audit-hash linkage).
3. Admin host: `admin.andrii.ai` (assumed) vs `admin.andrii.ui`
   (as written) — TLS, Origin allowlist, and mTLS CA depend on it.
4. Customer login: passkey/Apple (reuse) vs email-OTP vs
   none-for-demo; staff password+TOTP scope.
5. Photo/video/filing retention (only voice 365d decided) — blocks
   F8/P2 shipping.

**Needed during the 10 days:**

6. Conflict policy: block vs banner per topic (before day 9).
7. MCP tool list + auth (public vs keyed) + rate limits.
8. HCCH feed URIs + poll cadence; 500-URL seed list + politeness
   budget.
9. SAM 3.1 + Glimmer self-host packaging, weights licensing,
   single-server latency budget.
10. P2 label purchase: carrier-API buy vs staff-manual (spec
    recommends staff-manual); whose accounts, who pays.
11. P2 PII store: separate `p2.sqlite` (recommended) vs same-file
    tables; PII retention proposal (submitted 2y, drafts 90d,
    drawn PNGs 1y).
12. P2 replica-PDF acceptability research owner; wet-ink-only vs
    drawn-optional for demo (spec recommends wet-ink-only).
13. Tier b/c/d pricing, WA fee/SLA/pickup radius, $1.99-offer
    boundaries, "FedEx tomorrow" geography + remedy.
14. ES scope for MVP + legal-review owner (blocks launch of
    legal-adjacent pages).
15. Locale URL shape (`?l=` only vs future `/es/` prefixes);
    translation provenance rule.
