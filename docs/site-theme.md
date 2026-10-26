# Site Theme & Visual Design (draft for review)

- Status: draft (prompt 007, for author review; no implementation yet)
- Date: 2026-10-26
- Source: parallel researcher draft (full text recovered from child session log)
- Name note: this file is `docs/site-theme.md` and not `docs/DESIGN.md`
  because this Mac's filesystem is case-insensitive and `docs/design.md`
  already exists — the two names would collide.

**Design intent:** official-document trust. The site should feel like a
well-run clerk's office that answers on the first visit: precise, cited,
calm. No startup-hero theatrics. Proof (address, fee, form revision,
verified date, citation) is always one glance away.

Stack refs: [README](../README.md), [design.md](design.md),
[web-stack.md](../web-stack.md). Content/evidence model, audit, and `xtask
build-handbook` stay authoritative; this doc only governs visual system,
layout, and front-end behavior.

Resolution notes (from synthesis): the route list in §1 is the **public
subset only** — the two-build split in
[spa-separation.md](specs/spa-separation.md) governs (admin routes never
exist on the public origin). JS budgets reconciled: anonymous **entry ≤
40 KB Brotli** (agreed by both drafts); per-route page ≤ 30 KB, anonymous
page total ≤ 80 KB, customer total ≤ 120 KB (definitions in §2).

---

## 0. Design Principles

1. **Proof before decoration.** Every promise shows its evidence —
   address, fee, form revision, verified date, citation — within one
   glance. If a treatment hides provenance to look cleaner, the
   treatment loses.
2. **Phones are not a funnel.** Every task in J1–J9 completes
   one-handed on 360px (J9); mobile never degrades into a prompt to
   "continue on desktop" or into a paid tier.
3. **Honest upsell only.** Tier moves follow the journey triggers
   (printerlessness, hop confusion, flight math), never fear, grief,
   language need, or transition confusion (J5/J7/J8). Free tier (a)
   is fully usable.
4. **Never guess on legal-adjacent facts.** Uncertain, untranslated,
   or stale content says so in place with a date and a path forward
   — no silent machine translation, no confident rendering of a
   superseded form or address.
5. **Calm in crisis.** J7 customers get back-planners, walk-in
   intel, and the $1.99 estate path first; urgency is shown as dates
   and cutoffs, never countdown timers, stock pressure, or
   grief-targeted upsell.
6. **Language dignity.** ES-first (J8) is the same handbook, not a
   summary: EN field names preserved alongside ES guidance, forms
   stay in authority language, anti-scam notice always present.
7. **Every failure guides forward.** Errors, empty results, offline,
   and stale pages name what happened, what is still safe, and the
   single next action — with drafts and input preserved.

---

## 1. SvelteKit layout and component guidance

### 1.1 Route groups and shells

Use three route groups with distinct shells, all served under
`BASE_PATH=/apostille` with relative asset URLs (per web-stack sub-path
rule). Admin ships from a separate host; it never shares the public JS
entry.

```text
src/routes/(public)/+layout.svelte      # header, footer, i18n + theme switchers
src/routes/(public)/+page.svelte        # handbook home: search-first, not hero-first
src/routes/(public)/country/[slug]/+page.svelte
src/routes/(public)/authority/[slug]/+page.svelte
src/routes/(customer)/+layout.svelte    # requires session; adds order rail, WS status
src/routes/(customer)/order/[id]/+page.svelte
src/routes/(customer)/documents/+page.svelte
# admin is a separate BUILD + HOST from the same source (see
# spa-separation.md §1.2 two-build split — never on public origin):
admin/routes/(admin)/+layout.svelte     # call console, review queues, audit
```

- `(public)` is anonymous-first and must hit the JS budget in §2. No auth
  store, no WS client in the entry chunk; WS connects lazily only on pages
  that need live data (order tracking preview, correction status).
- `(customer)` lazy-loads `ws-client.js` + order store per-page. Presence
  and tracking events use the `authority:{slug}` / `call:{id}` topic
  envelopes (full topic set: `authority`, `call`, `submission`,
  `shipment` — see [specs/README.md](specs/README.md)).
- `(admin)` is not size-budgeted like public; it prioritizes density and
  keyboard operation. Shared UI primitives live in a small internal
  package (`ui/`) imported by both projects, but admin CSS/JS never leak
  into the public entry.

### 1.2 Layout patterns (no banned patterns)

- **Home:** search-first ledger, not eyebrow-pill + 3-card grid. Row 1:
  compact masthead (wordmark + jurisdiction search + language/theme menu).
  Row 2: "Start with your destination" search panel with recent
  authorities. Row 3: two-column ledger: left = authority index
  (table-like rows with verified dates), right = "Recently verified
  changes" feed with citations. Proof density is the hero.
- **Authority page:** document-dossier layout. Sticky page title block
  (authority name, verified-as-of, trust tier chip), then tabbed
  sections: Requirements / Forms / Fees / Visit / Mail. Each fact row
  shows statement + citation link + capture date. Sidebar holds fee
  total calculator and "Report a change" action.
- **Order flow:** linear stepper with persistent summary rail on
  desktop, bottom sheet summary on mobile. One question per screen on
  phones; steps are addressable URLs for back-button safety.
- **Call console (admin):** three-pane: script/questions left, live
  transcript center, fact-drafting right. WS-driven; degrades to polling
  snapshot if WS blocked.

### 1.3 Component inventory (build once in `ui/`)

`Masthead`, `JurisdictionSearch`, `VerifiedChip`, `FactRow` (statement +
source + date), `FeeTable`, `DocChecklist` (interactive, persisted),
`Stepper`, `OrderSummary`, `Timeline` (order/visit status), `Notice`
(info/success/warning/error variants — filled icon + heading + body,
never a bare left-border strip), `DataTable`, `ThemeMenu`, `LangMenu`,
`CitationPopover`, `FormDownloadCard` (filename, revision, SHA short,
size). Icons: inline SVG set (custom, government-form idiom: seal,
stamp, envelope, check). No emoji icons, no lucide-grid cards.

Radius system (never one radius everywhere): `xs 4px` (chips, inputs),
`sm 8px` (cards, buttons), `md 14px` (panels, sheets), `full` only for
status dots and avatar initials.

Elevation: flat + 1px borders by default; one raised level (`0 1px 2px
rgba(16,33,47,.08)`) for menus/sheets only. No site-wide glassmorphism;
blur is allowed only on the mobile bottom-sheet scrim.

---

## 2. Anonymous JS budget

**Hard budget: public entry ≤ 40 KB Brotli.** This is what an anonymous
visitor downloads to render any public page shell.

- **Entry** = SvelteKit client runtime + `(public)` layout + global CSS +
  router + i18n detector. Measured as Brotli-compressed bytes of the
  initial navigation's required JS+CSS, excluding images and fonts, on a
  cold cache. CI fails above 40 KB.
- **Page** = additional per-route chunk loaded on navigation (e.g.
  `authority/[slug]` + `FeeTable` + `CitationPopover`). Budget: ≤ 30 KB
  Brotli per public route chunk; ≤ 80 KB total for any anonymously
  reachable page (entry + page + shared chunks). Customer pages: ≤ 120
  KB total after auth (WS client allowed). Admin: no fixed budget, but
  each console route must code-split vendor deps (no monolith).
- Enforcement: `vite-bundle-visualizer` report stored per build; `cargo
  xtask ci` (or a Node check it shells to — the one sanctioned Node
  exception) parses the SvelteKit build manifest and asserts entry/page
  limits. Fonts load as `woff2` with `font-display: swap` and do not
  count toward JS but count toward a 120 KB total font budget (latin +
  latin-ext only; Cyrillic/variable axes lazy).
- Tactics: no date/chart/UI framework in entry; `Intl`-based dates;
  native `<dialog>`/`<details>` before JS widgets; `svelte:component` +
  dynamic `import()` for calculator, search-index, and WS client.

---

## 3. Color scheme — official-document trust

No purple/indigo/violet anywhere; no gradients as accents or text; no
cream page backgrounds. Primary accent is deep federal blue; secondary
seal-green is reserved for verified/success semantics so "green =
checked" stays unambiguous.

### 3.1 Light (default when `prefers-color-scheme: light` or no preference)

| Token | Hex | Use |
|---|---|---|
| `bg` | `#FFFFFF` | page |
| `surface` | `#EFF3F6` | panels, table zebra, wells |
| `surface-2` | `#E2EAF0` | hover wells, selected rows |
| `border` | `#CBD8E2` | 1px dividers, inputs |
| `text` | `#0F1F2B` | body, headings (16.9:1 on bg) |
| `text-muted` | `#334E63` | secondary text (9.1:1 on bg; never gray-400/500) |
| `accent` (federal blue) | `#0B4A75` | links, primary buttons, active states (8.6:1 on white) |
| `accent-ink` | `#FFFFFF` | text on accent fill |
| `accent-hover` | `#083A5C` | hover fill (darken, no scale lift) |
| `success` | `#176B3A` | verified chips, checklist done (5.9:1) |
| `success-bg` | `#E3F2E8` | success notice fill |
| `warning` | `#8A4B00` | caution text (5.9:1) |
| `warning-bg` | `#FDF0D7` | warning notice fill |
| `error` | `#A6192E` | errors, rejections (7.0:1) |
| `error-bg` | `#FBE7EA` | error notice fill |
| `focus` | `#0B4A75` | 3px focus ring (2px offset) |

### 3.2 Dark (default when `prefers-color-scheme: dark`)

| Token | Hex | Use |
|---|---|---|
| `bg` | `#0B141B` | page |
| `surface` | `#13222D` | panels |
| `surface-2` | `#1B2F3E` | hover wells |
| `border` | `#2E4658` | dividers |
| `text` | `#EDF3F7` | body (16.2:1 on bg) |
| `text-muted` | `#B9CBD8` | secondary (9.4:1 on bg) |
| `accent` | `#7CC0EA` | links, accents on dark (9.8:1 on bg) |
| `accent-fill` | `#164E74` | primary button fill in dark |
| `accent-ink` | `#FFFFFF` | text on accent-fill |
| `success` | `#5FD08A` | verified (9.5:1) |
| `success-bg` | `#123324` | notice fill |
| `warning` | `#F2B84B` | caution (9.1:1) |
| `warning-bg` | `#3A2A10` | notice fill |
| `error` | `#F07D8E` | errors (7.2:1) |
| `error-bg` | `#3D1420` | notice fill |
| `focus` | `#7CC0EA` | focus ring |

### 3.3 Theme behavior

- Default follows OS (`prefers-color-scheme`); stored override in
  `localStorage` (`theme: light|dark|auto`) exposed in the masthead menu
  as Light / Dark / System. `auto` is the initial value.
- Implement as `data-theme` on `<html>` + CSS custom properties;
  SvelteKit renders `auto` server-side without flashing by inlining a
  300-byte theme-init snippet (counts inside entry budget).
- Charts, seals, and status dots must use the token pairs above — never
  hard-coded grays. Print stylesheet forces light tokens, expands
  citations, hides nav/menus. It also carries the P2 print roadmap
  (see [p2-forms-labels](specs/p2-forms-labels.md) P2-R6): form
  printouts ultimately render from styled HTML/CSS at the correct
  page size, with PDF as an on-request option only.

---

## 4. Typography and texture

Banned: Inter-only, Geist, Space Grotesk, Instrument Serif, Roboto-only,
Arial-only stacks. Direction: civic + archival.

- **Display/headings:** `Source Serif 4` (600/700, optical sizing). Used
  for page titles and dossier headings only. Tight tracking (−0.01em),
  never gradient-filled.
- **Body/UI:** `Public Sans` (400/500/700). US-government lineage,
  excellent at small sizes and in tables. Body 16–17px, line-height
  1.55; table/data 14–15px.
- **Mono/data:** `IBM Plex Mono` (400/600) for fees, tracking numbers,
  form revisions, hashes, timestamps.
- Hierarchy is strongly stepped: 36/28/22/17px display scale on desktop,
  30/25/20/16px on mobile; section labels are 12.5px uppercase with
  0.08em tracking + rule line, not pills.
- Texture: hairline rules, ledger rows, stamp-like verified chips (solid
  fill + 4px radius + mono date), subtle paper grain via 1px
  repeating-linear-gradient rules only on dossier headers — no aurora
  blobs, glows, or full-page gradients.

---

## 5. Mobile-first adaptive rules (phones equal desktops)

Phones are first-class: every task completable one-handed on 360px wide.

1. **Breakpoints:** `0 / 720 / 1080 / 1440px`. Base styles target 360px;
   media queries add columns, never remove functionality.
2. **Navigation:** bottom-anchored primary actions (Search, Orders, Help)
   on `<720px`; masthead collapses to wordmark + search icon + menu.
   Customer order rail becomes a sticky bottom summary bar with
   expandable sheet.
3. **Tables → ledgers:** fee/fact tables reflow to stacked key-value rows
   under 720px (CSS `display:block` on rows, `data-label`
   pseudo-labels), preserving citation links and mono figures.
4. **Touch:** 44px minimum targets; stepper and checklist controls
   full-width; no hover-dependent disclosure (hover reveals nothing
   essential; tap does).
5. **Search:** jurisdiction search is a full-screen sheet on mobile with
   large input, recent + nearby authorities, offline-cached index of top
   106 authorities.
6. **Media:** route photos/parking tips load as swipeable strips with
   `loading=lazy`, `decoding=async`, explicit dimensions (no CLS);
   redacted derivatives only (per evidence-store rules).
7. **Performance:** same 40 KB entry budget governs mobile; images
   AVIF/WebP via axum `ServeDir` precompressed + `Cache-Control:
   immutable`; no layout shift from fonts (`size-adjust` fallbacks).
8. **WS on mobile:** reconnect with exponential backoff + visible "Live /
   Reconnecting / Offline" pill; offline order drafts persist in
   IndexedDB and sync on reconnect.

---

## 6. Internationalization (i18n)

Scope at launch: English, Spanish, French (matching evidence-store
assumption). All UI strings externalized from day one.

- **Detection:** `Accept-Language` negotiation on first visit + client
  `navigator.language` confirmation; explicit choice wins thereafter
  (`localStorage` locale + cookie for axum-rendered fallbacks).
- **Switcher:** in-masthead `LangMenu` (EN / ES / FR with full language
  names in their own language: English / Español / Français). Switching
  preserves route and query, swaps SvelteKit dictionary chunk (per-locale
  code-split, ≤ 12 KB Brotli each for core UI).
- **`?l=<lang>` param:** supported on every public route (`?l=es` forces
  Spanish for that navigation and persists the choice). Canonical URLs
  omit `?l` for the default locale to avoid duplicate-content splits;
  alternate locales use `?l=` (no path-prefix locales at launch — see
  open question 1).
- **Content:** handbook facts carry per-authority language metadata;
  untranslated fact statements show source-language text with a
  "Translation pending — showing original" notice rather than
  machine-translating legal facts silently.
- **Formatting:** `Intl.DateTimeFormat` / `Intl.NumberFormat` per locale;
  fees show currency code + symbol (`USD $8.00` / `8,00 $US`); addresses
  keep authority-native order, transliterated only when the authority
  publishes a transliteration.
- **Flagged questions:** (a) locale-URL shape — stay with `?l=` vs move
  to `/es/...` prefixes when SEO/traffic justifies it (prefixes
  complicate `BASE_PATH=/apostille` + axum nesting, so defer); (b)
  string authorship — who writes/translates legal strings, and does a
  mistranslation require the same `audit`-row + `build` provenance as a
  fact change? Recommendation: yes, translation edits ride the same
  fact/task pipeline.

---

## 7. SEO, robots, sitemaps, .well-known

### 7.1 Ready-to-use `robots.txt`

Served by axum at `/apostille/robots.txt` (and root `/robots.txt`
redirecting to it). Customer/admin paths are never crawled.

```text
User-agent: *
Allow: /apostille/
Disallow: /apostille/order/
Disallow: /apostille/documents/
Disallow: /apostille/api/
Disallow: /apostille/*?l=*
Allow: /apostille/sitemap*.xml

Sitemap: https://andrii.ai/apostille/sitemap.xml
```

Note: `Disallow` on `?l=` keeps the default-locale canonical clean while
`hreflang` in the sitemap advertises alternates (see §7.2). Admin host
serves a closed robots:

```text
User-agent: *
Disallow: /
```

### 7.2 Sitemap strategy (merges into `xtask build-handbook` — no separate tool)

- `xtask build-handbook` already walks accepted facts per authority;
  extend it to also emit `sitemap.xml` + `sitemap-<country>.xml` shards
  (one per country, gzip + brotli precompressed) into the static dir
  axum serves. No new binary, no Node script.
- Include: home, every country page, every authority page, every public
  form-download page. Exclude: order/document/API/admin routes.
- Each `<url>` carries `<lastmod>` (= latest accepted-fact `created_at`
  for that authority), `<changefreq>` derived from observed churn
  (default `monthly`), and `hreflang` alternates (`en` canonical + `es` /
  `fr` via `?l=`).
- Sitemap index references shards; build writes an `audit` row +
  `facts_hash` so any sitemap diff traces to inputs. Stale-authority
  pages (no accepted fact in >180 days) stay listed but drop priority
  and gain a visible "reverify in progress" banner.

### 7.3 `.well-known` and trust files

Serve from axum (static, versioned, `Cache-Control: max-age=86400`):

- `/.well-known/security.txt` — contact, expires, PGP/policy links (RFC
  9116). Draft body:

  ```text
  Contact: mailto:security@andrii.ai
  Expires: 2027-06-01T00:00:00.000Z
  Preferred-Languages: en, es, fr
  Canonical: https://andrii.ai/.well-known/security.txt
  Policy: https://andrii.ai/apostille/security
  ```

- `/.well-known/change-password` → 302 to customer account page (future).
- `/apostille/humans.txt` — team + verification-method credits.
- `/apostille/security` — public page explaining evidence tiers,
  WARC/TLS retention, and how to report spoof domains (binds to
  `domain.status=fake` workflow).
- Favicon/manifest/icons under `/apostille/icons/`;
  `manifest.webmanifest` with theme colors matching §3.

---

## 8. Wow-effects shortlist (tasteful, SvelteKit-feasible)

Each is functional, skippable, and honors `prefers-reduced-motion`. No
scroll fade-up blankets, no hover-scale lifts, no bento grids, no aurora.

1. **Verified-ledger ticker** — home feed animates newly accepted facts
   in via WS with a stamp-settle (120ms translate + seal ping, once per
   item). Reduced-motion: instant insert with no animation.
2. **Fee-total calculator with receipt** — authority page sidebar tallies
   fees/copies/postage live and prints a receipt-styled summary (mono
   figures, rule lines). Pure Svelte reactivity, zero dep.
3. **Doc-checklist with persistence** — per-authority packing checklist
   (originals, copies, envelope, label) saved locally + to order;
   progress ring in the dossier header. Small SVG arc, no library.
4. **Order timeline with live leg** — customer order page shows
   mail/authority/return legs; current leg pulses subtly via opacity
   (not scale) on WS events; full timeline keyboard-navigable.
5. **Route-photo loupe** — visit photos open in a native `<dialog>` with
   pinch-zoom and redaction badges ("faces/plates masked · SAM 3.1"). No
   lightbox dep.
6. **Citation hover-cards** (desktop) / tap-cards (mobile) — any fact
   citation previews source domain, capture date, and trust tier without
   navigation. Svelte action + `prefers-reduced-motion`-aware 80ms fade
   (opacity only, single element).

---

## 9. Accessibility (contrast, focus, motion)

- **Contrast:** body ≥ 7:1 both themes (§3 values verified); muted ≥
  4.5:1; chips/buttons ≥ 4.5:1 for text; status never color-only (icon +
  label + text).
- **Focus:** 3px solid `focus` ring with 2px offset on all interactives
  (`:focus-visible`); skip links ("Skip to content / search"); stepper,
  tabs, timeline, and menus fully keyboard-operable with `aria-*`
  wiring; SvelteKit page-title + `aria-live` announcements on route
  change and WS events.
- **Motion:** global `@media (prefers-reduced-motion: reduce)` disables
  all six wow-effects, transitions, and WS pulse; `matchMedia` gate in
  the WS client suppresses animated inserts; no autoplay, no parallax,
  no scroll-jacking.
- **Forms:** real `<label>`s, inline errors with `aria-describedby`,
  44px targets, autocomplete tokens on address/email, offline-draft
  recovery announced politely.

---

## 11. Content tone and legal-adjacent copy

- **Voice:** clerk-precise, warm, plain-language (grade 8). Short
  sentences, named dates (`verified 2026-10-20`), no superlatives,
  no "guaranteed acceptance."
- **Legal-adjacent rule:** handbook gives general information, never
  advice. POA/corporate pointers (J4), Viet Nam explainer (J5),
  Egypt chain (J6) carry the attorney-approved disclaimer in EN +
  ES: "General information, not legal advice." Verbatim text gated
  on legal review (see journeys §ES scope).
- **No silent MT:** untranslated facts show source-language text +
  `Notice` info "Translation pending — showing original (EN).
  Reviewed translation due [date]." Human- vs. machine-reviewed ES
  strings flagged with reviewer + date; machine-only legal-adjacent
  strings never ship on J4/J5/J6 paths.
- **Forms:** authority language wins; ES guides explain
  field-by-field with EN field names inline (`Nombre / First name:
  "Given names"`). Never rename a field the customer must sign.
- **Staleness:** >180-day authorities keep the §7.2 banner with
  fixed copy shape: "Last verified [date] — reverify in progress.
  Confirm at [citation] before mailing." No invented "current as of
  today."

## 12. Honest upsell and price/date display

- **Trigger-mapped only:** upsell components reference the journey
  trigger that fired it (e.g. "No printer? (J9)" → tier (c);
  "flight in <14 days (J7)" → tier (d)). No interstitial, no
  pre-checked tier, no fear copy ("Don't risk rejection!").
- **Grief rule (J7):** estate/inheritance surfaces the $1.99 path
  first with cap/end boundary (`through June 2027 or $50k cap`);
  speed upsell allowed only on back-planner math that misses the
  flight.
- **No price/date without provenance:** fees show cited passthrough
  + service fee separately; SLAs ("FedEx tomorrow," pickup
  windows) show scope + remedy or do not render. Placeholder
  questions in journeys §Pricing gate every price/date component.
- **Comparison shape:** tier deltas as ledger rows (`adds: return
  label, tracked`), never feature-check grids with the free column
  grayed out.

## 13. States: empty, loading, error, offline, stale

- **Empty:** `Notice` info + heading + one action. Search no-hit:
  "No authority matches '[q]'" + recent authorities + "Browse all
  106" link. New order with no documents: checklist starter, not a
  blank rail.
- **Loading:** skeleton ledger rows (same row height, `surface`
  pulse via opacity only), never spinners on full pages; search
  sheet responds <100ms with cached top-106 index, then refines.
- **Error:** `Notice` error with filled icon + heading + body +
  retry (per §1.3, never bare left-border). Inline field errors use
  `aria-describedby`, preserved input, one error per field, stated
  fix ("Use the Tumwater PO Box — Olympia closed June 2025
  [citation]").
- **Offline/stale:** WS pill states `Live / Reconnecting /
  Offline` (§5.8); offline drafts persist to IndexedDB with "Saved
  on this device — will sync" `Notice` warning. Stale-fact rows
  keep old value struck-through beside the new draft until
  accepted, never a silent swap.

## 14. Form, validation, and photo pre-check UX

- **Flow:** one question per screen <720px, addressable step URLs,
  persistent summary (rail ≥720px, bottom sheet below). Back button
  never loses input; drafts restore with polite `aria-live`
  announcement.
- **Validation:** validate on blur + submit, never on keystroke;
  success is quiet (chip/check), failure is inline + cited.
  `autocomplete` tokens on address/email; fees/dates in `IBM Plex
  Mono` with `Intl` formatting per §6.
- **Photo pre-check (J9):** ternary result only — Pass / Fail /
  Retake — each with reason + annotated thumbnail + one-tap retake.
  Fail names the defect ("glare over seal") and whether a retake or
  a rescan is needed; no numeric "quality scores."
- **Printerless paths:** every "print label" step offers QR/
  post-office scan or FedEx hold-for-pickup alongside PDF (J9);
  shareable checklist link alongside printable PDF.

## 15. Motion budget (companion to §8)

- **Allowances:** micro-feedback ≤120ms, card/popover fade 80ms
  opacity-only, one stamp-settle per new ledger item (120ms
  translate + seal ping). One animated element per event; no
  chains, loops (except WS current-leg opacity pulse), parallax,
  or scroll-triggered blankets.
- **Easing:** `ease-out` for entrances, linear for opacity pulses.
  No scale lifts on hover (§3.3 accent darkens instead).
- **Gates:** all §8 effects disabled under
  `prefers-reduced-motion: reduce` via the §9 media query +
  `matchMedia` WS gate; `Live / Reconnecting` pill changes are
  text-first, animation-second. CI lints for banned properties in
  animations (`transform: scale` on cards, `filter: blur` outside
  bottom-sheet scrim).

## 16. Trust, verification, and anti-scam display

- **Verified chip:** solid fill + 4px radius + mono date (`Verified
  2026-10-20`), `success` tokens only; tiers shown as text (`Tier 1
  · in-person`), never color alone. Every `FactRow` keeps statement
  + citation link + capture date; citation hover/tap cards show
  domain, capture date, tier (§8.6).
- **Correction loop:** every authority page carries "Report a
  change"; J5 adds "Was your apostille refused in Viet Nam? Tell
  us" — reports feed the fact/task pipeline, never a dead inbox.
- **Anti-scam (J8):** ES journeys show a persistent `Notice`
  warning: what Andrii Co. will never ask for (full SSN by chat,
  gift-card payment, consulate cash drop) + the real phone
  `+1(425)800-8688`. Spoof-domain reports link to
  `/apostille/security` (§7.3).

## 17. Open questions

1. Locale-URL shape: ship `?l=` only, or reserve `/es|fr/` prefixes now
   for future SEO (cost: axum nesting + canonical migration)?
2. String authorship: do UI/fact translations require the same `audit` +
   `build` provenance as facts, and who approves Spanish/French legal
   strings?
3. Admin host: exact hostname + auth (SSO vs local), and may any admin
   component share the public `ui/` package or stay fully forked?
4. Fee/facts staleness threshold: is 180 days the right "reverify"
   trigger, and what banner copy is legally safe?
5. Photo retention: voice is 365 days — what retention governs customer
   route photos and redacted derivatives?
6. PGP key for `security.txt`: publish a key now or `Contact:`-only at
   launch?
7. Font licensing/self-hosting: confirm self-hosted Source Serif 4 /
   Public Sans / Plex Mono woff2 (no Google Fonts CDN, per offline-prod
   rule)?
8. Order-draft storage: is IndexedDB + WS-sync acceptable for
   PII-adjacent drafts, or must drafts live server-side from keystroke
   one?
