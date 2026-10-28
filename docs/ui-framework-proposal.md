# UI Framework + Visual-Design Proposal (for author review)

Status: DECIDED 2026-10-28 (D38) — Tailwind v4, pilot home+dossier,
drive-bys approved. Implementation underway; the analysis below is kept
as the record.
Research basis: every file cited below was read (bodies, not listings);
bundle sizes measured with `wc -c` on the current tree. No builds, tests,
or writes were performed for this proposal.

> Amendment 2026-10-28: the author removed the 223KB gate the same day
> (`cargo xtask web-budget` is now advisory-only; JS/HTML/image sizes are
> unlimited until the initial-page optimization pass). The analysis below
> still stands as measured, but the recommendation's premise changed: with
> no gate, Tailwind v4's main objection (forcing a bump) is gone, and the
> choice becomes ecosystem/convention vs consolidation churn. Re-read §1–§2
> and OPEN-29 in that light before deciding.

## 0. Ground truth

- **Gate** (`xtask/src/main.rs`, `run_web_budget`): sums raw bytes of every
  `*.js`/`*.css` under `web/build`; budget `223*1024 = 228,352`. It measures
  the whole-build sum, not per-navigation cost. Every past bump
  (150→200→220→221→223KB) carries a dated comment naming the feature and
  which chunk pays.
- **Current position (measured):** JS 201,251 + CSS 24,885 = **226,136 bytes
  → headroom ≈ 2.2KB (~1%)**. Any option must account for this.
- **Two builds** (`web/vite.config.ts`, `web/README.md`): no `svelte.config.*`
  exists; Kit config is inline. `WEB_BUILD` unset → `src/routes-public/`
  (symlinks to shared layouts + `(public)`/`(customer)`), base `/apostille`,
  output `build/`; `WEB_BUILD=admin` → full `src/routes`, base `''`, output
  `build-admin/`. The gate watches **public only**; admin is unbudgeted.
  CSR everywhere (`ssr=false`, `prerender=false`, `index.html` fallback).
- **Styling today:** `app.css` (2.6KB) holds tokens matching `site-theme.md`
  §3 exactly; dark mode = `prefers-color-scheme` + `data-theme` override +
  blocking snippet in `app.html`. 35 pages, 5 lib components, **44 `<style>`
  blocks** — button/input/ledger/notice styles are copy-pasted per page
  (~0.5–1KB duplicated in each of ~10 pages). Zero runtime deps; Svelte 5
  runes forced.
- **Taste law** (`site-theme.md`): no purple/indigo/violet, no gradient
  accents, no cream backgrounds; civic+archival (Source Serif 4 display /
  Public Sans body / Plex Mono data — currently system stand-ins); flat +
  1px borders, one raised level for menus/sheets only; radius xs4/sm8/md14;
  44px targets; 3px focus rings; motion ≤120ms, opacity-only. The existing
  look is a sober clerk's ledger. The "nice UI" job is **density, rhythm,
  and polish inside that idiom** — not a rebrand.

## 1. Framework options, ranked for THIS repo

### Option A — Stay with CSS vars + consolidate a hand-rolled system (1st)

- **Bundle vs gate:** only option with *negative* cost. Deduping the ~10
  copies of button/input/ledger styles into shared classes in `app.css`
  (or one `ui.css`) removes an estimated **3–6KB raw** — the gate sum drops
  because duplicated per-route CSS disappears. Creates headroom instead of
  consuming it.
- **Two-build compat:** perfect; nothing changes about symlinks, bases,
  or outputs.
- **Dark mode:** zero work — vars + `data-theme` + snippet already correct.
- **Migration cost:** lowest. Additive: grow `app.css`, add ~4 shared
  components (`FactRow`, `Field`, `Button`, `DataTable`, joining
  `Notice`/`VerifiedChip`). Pages migrate opportunistically; no flag day.
- **Build-time cost:** zero. No new deps, no plugin risk.
- **Downside:** no utility shorthand; discipline needed to keep new pages
  on shared classes (enforce by review + a `check-imports`-style lint later).

### Option B — Tailwind v4 (2nd)

- **Bundle vs gate:** net **−5KB…+10KB raw**, likely over budget. v4 emits
  preflight (~11KB raw) + `@theme` vars (~2KB) + utilities for 35 pages
  (est. 8–20KB) → ~20–35KB CSS vs today's 25KB. With 2.2KB headroom this
  **probably forces a 223→~233KB bump** with a documented comment. Worse,
  Tailwind consolidates CSS into one entry file: per-navigation CSS for
  anonymous home grows from ~2–4KB code-split to the full ~20KB+.
- **Two-build compat:** good (`@tailwindcss/vite`; scanning follows the
  per-build routes dir). Caveat: admin-only utilities in shared `app.css`
  still ship in the public entry — same leak class as today, easier to
  cause accidentally.
- **Dark mode:** workable but fiddly: needs `@custom-variant dark` matching
  *both* `prefers-color-scheme` default *and* `data-theme` override, plus
  keeping the blocking snippet. Subtle theme-regression risk.
- **Migration cost:** highest. Touch all 35 pages + 5 components
  (44 style blocks → utilities). Large diff, high regression surface on
  hand-tuned a11y details (focus rings, 44px targets).

### Option C — UnoCSS (3rd)

- **Bundle vs gate:** best framework number — no mandatory preflight,
  utilities-only, est. **6–15KB CSS → net −10…−5KB**. Could pass the gate
  without a bump.
- **Downside:** same flag-day migration as Tailwind, smaller docs/community
  for the `data-theme` + OS-default combination, no first-party Svelte
  story, team learning curve. Better bytes than Tailwind, worse ecosystem —
  not worth it over A.

### Option D — vanilla-extract (4th, not recommended)

- Near-neutral bundle, but theme contracts fight the no-flash `data-theme`
  snippet, no first-party Svelte support, highest per-file migration cost
  (rewrite styles into TS) and build-pipeline cost. The repo's tokens are
  already frozen in `site-theme.md` §3 and tiny — the type-safety payoff
  doesn't cover the cost.

## 2. Recommendation

**Stay with CSS custom properties and build the missing system layer by
hand (Option A).**

Rationale: (1) it is the only option that *improves* the 2.2KB gate
headroom instead of spending it; (2) the tokens, dark-mode machinery,
radius scale, and component semantics are already specified
(`site-theme.md` §§1.3/3/4) and partially built — the gap is consolidation,
not technology; (3) zero new deps honors `web-stack.md`'s minimal-Node rule
and keeps both builds, `svelte-check`, and CI untouched; (4) migration is
additive and pilotable per page, while Tailwind/UnoCSS demand a 44-file flag
day for a design this small. Revisit Tailwind only if the gate ever moves to
per-navigation Brotli (Week-3 targets), where its entry-file consolidation
can be judged fairly.

## 3. Visual design direction (concrete, inside existing vars)

**Type scale** (implement `site-theme.md` §4 literally): desktop 36/28/22/17px,
mobile 30/25/20/16px; body 16px/1.55; data/mono 14–15px Plex Mono; section
labels 12.5px uppercase, 0.08em tracking, rule line below (the pattern already
on home `.section-label` — promote to a shared `.label` class). Display serif
for page titles/dossier headings only; never for UI chrome. No new families
until the §17-Q7 self-hosting decision lands — keep stand-in stacks.

**Spacing rhythm:** 4px base unit; page vertical rhythm 1rem (card padding) /
1.5rem (section gaps) / 2rem (major breaks, 3rem on ≥1080px); ledger row
padding 0.7rem stays (it *is* the look). Container stays 1080px; forms
28–36rem; admin density unchanged (keyboard-first tool, not marketing).

**Color usage (no new tokens):** federal blue `#0B4A75` for links/primary/
active-tab underline; `accent-hover` darken on hover (never scale lifts);
seal-green **only** for verified/success; warning for conflicts/consent
banners; error for destructive/rejections. Elevation stays flat + 1px borders;
the single raised shadow reserved for `CountrySelect` dropdown + mobile sheets.

**Component inventory — what changes on each of the 6 pages:**

1. **Public home** (`(public)/+page.svelte`): search-first ledger stays. Add:
   shared `.btn-primary` (44px, radius-sm), `.input` classes; "Recently
   verified" feed column at ≥1080px fed by accepted-facts refresh; skeleton
   ledger rows for loading (replace "Loading…" text); stamp-settle insert
   animation (120ms, opacity+translate, `prefers-reduced-motion` off-switch).
   No hero, no cards grid.
2. **Dossier** (`authority/[slug]/+page.svelte`): sticky title block (name +
   `VerifiedChip` with mono date + tier text); tabs keep underline-active
   pattern; new shared `FactRow` (statement + citation link + capture date +
   tier); rail becomes receipt-styled card (mono figures, rule lines) with fee
   *counts* only (no totals — pricing table still open) + persistent "Report
   a change" action. Conflict banners keep `Notice` warning shape.
3. **Login** (`(customer)/login/+page.svelte`): biggest visual lift, zero logic
   change. One 28rem card: passkey login primary, register secondary (outline
   button, not second primary), OAuth buttons as bordered neutral rows with
   provider names (no brand-color fills — civic idiom), magic-link as tertiary
   section with divider; inline `Notice` errors (replace ad-hoc `.notice.err`
   which today wrongly uses `warning-bg` for errors — a real bug, fix to
   `error-bg`); logged-in state becomes success `Notice` + action list.
4. **Admin config** (`admin/config/+page.svelte`): keep card-per-key density.
   Add: shared `.btn` + `.field` classes (delete local copies); `Set/Unset`
   as `VerifiedChip`-style mono chips (green Set / muted Unset); per-row save
   moves inline-right at ≥720px; saved/error states already use `Notice`
   correctly — unchanged. No aesthetic softening: admin stays dense.
5. **Triage** (`admin/triage/+page.svelte`): queue cards keep shape. Add:
   conflict badge already correct; conflicts list keeps warning-bg + 3px left
   rule; Accept stays primary, Reject stays secondary (never red-fill —
   rejection is routine work, not danger); reason input gets shared `.input`;
   add keyboard hints (`j/k` + `a/r` later — follow-up, not this item);
   empty/loading states already `Notice`-shaped.
6. **Calls** (`calls/+page` + `calls/[id]/+page`): dial form gets shared
   `.field`/`.btn` + `Notice` errors (replace bare `.error` paragraphs);
   consent checkbox row keeps 44px; consent `Notice` warning stays pinned;
   detail panes keep bordered-card grid, add status pill (Live = success pulse
   opacity-only / ended = muted chip), transcript segments get speaker + mono
   timestamp line, controls grouped with Hang-up as secondary-danger
   (error-border outline, not red fill).

## 4. Phased rollout

- **Phase 0 — Foundations (1 turn):** extend `app.css` with shared classes
  (`.btn`, `.btn-primary`, `.btn-secondary`, `.input`, `.label`, `.ledger`,
  `.card`, `.receipt`, skeleton rows) + print-stylesheet stub (§3.3). Verify:
  `npm run build` + `cargo xtask web-budget` shows total *down*; no page
  restyled yet.
- **Phase 1 — Pilot: public home:** restyle home onto shared classes; add
  skeleton + verified feed. Verify: visual diff, axe/keyboard pass,
  `web-budget` total down (home chunk shrinks — advisory, no gate).
- **Phase 2 — System:** add `FactRow`, `Field`, `DataTable` to `lib/public-ui/`
  (+ tests where logic exists); convert dossier + login. Verify per page:
  `check-imports` green (new shared components live in `public-ui` only —
  never `lib/customer`).
- **Phase 3 — Sweep:** customer pages (forms/AddressBlock/CountrySelect/
  PhoneInput adopt `.field`/`.input`), then admin (config/triage/calls).
  Admin ships in `build-admin` only — verify via `build:admin` + smoke,
  never in public bundle.
- **Size tracking:** no gate — `cargo xtask web-budget` is advisory-only
  since 2026-10-28. Still report the total per phase (consolidation should
  move it *down*); if a phase grows it notably, say so in the phase notes
  with the bytes and which chunk pays.

## 5. Risks + what NOT to change

- **Import separation:** `check-imports.mjs` + symlinked `routes-public/` are
  load-bearing. New shared UI goes in `lib/public-ui/` only; `(public)` pages
  must never import `lib/customer`/`lib/admin`. Any framework with
  auto-imports or barrel files gets audited against this gate before adoption.
- **CSR constraints:** `ssr=false`/`prerender=false` + `index.html` fallback.
  No styling approach that assumes SSR (critical-CSS extraction,
  server-injected `<style>`) is acceptable. The blocking theme snippet in
  `app.html` stays exactly as-is.
- **No-JS surfaces:** there are none for app pages (CSR SPA) — do not claim
  any. Static surfaces (`robots.txt`, `sitemap*.xml`, `.well-known`) are
  axum-served and untouched by this work. Keep JS-failure copy honest
  ("unavailable, retry").
- **Other DO NOTs:** no admin chunk on public origin (two-build split
  untouched); no token-hue changes (contrast ratios in §3 are verified — any
  new shade needs re-verification); no webfonts until §17-Q7 resolves (CDN
  fonts violate offline-prod); no motion beyond the §15 budget; no emoji/lucide
  icons (inline SVG seal/stamp idiom when icons arrive); keep 44px targets and
  3px focus rings everywhere.

## 6. Open questions for the author

| ID | Question | Owner | Blocks |
|----|----------|-------|--------|
| OPEN-29 | Confirm Option A (consolidate CSS vars, no Tailwind)? Recommended: yes — gate headroom, zero deps, and the spec'd-but-unbuilt system layer all point there. (Fallback if no: Tailwind v4 + a documented ~10KB bump, not UnoCSS — ecosystem + hiring pool beat UnoCSS's byte edge.) | Author | UI Phases 0–3 |
| OPEN-30 | May Phase 0 fix the login error-color bug (`.notice.err` uses `warning-bg`; should be `error-bg`) and add the print-stylesheet stub as drive-bys? Recommended: yes — both are one-line-class changes inside touched files, and the error color is user-facing misinformation. | Author | Phase 0 scope |
| OPEN-31 | Scope of the pilot: home-only, or home + dossier together? Recommended: home-only first — one page proves the shared-class pattern and the gate math; dossier's `FactRow` work is Phase 2's natural companion. | Author | Phase 1 scope |
