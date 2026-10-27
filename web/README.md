# Web shells (SvelteKit, two builds)

S4/S5 lanes. One codebase, three route groups, two deploy units
(`docs/specs/spa-separation.md` §1.2):

- `src/routes/(public)/` — anonymous: home search + authority dossiers.
  Themed, EN-only. Must never import customer/admin libs.
- `src/routes/(customer)/` — logged-in: orders, uploads, precheck,
  billing, order form, demo login.
- `src/routes/(admin)/` — staff only: dashboard, triage, call console,
  config. Ships ONLY in the admin build served from `admin.andrii.ai`.

## Builds

```sh
npm run build        # public: (public)+(customer), base /apostille → build/
npm run build:admin  # admin: full routes, base '' (host root) → build-admin/
```

The split uses Kit config keys verified against `@sveltejs/kit`
3.1.0 + adapter-static 4 (`files.routes`, `paths.base`, adapter
`pages`), switched by `WEB_BUILD=admin` in `vite.config.ts`. The
public tree is `src/routes-public/` (symlinks to the shared
layouts + `(public)`/`(customer)` — no `(admin)`).

## Routes

Public (`/apostille/…`): `/` (search-first ledger home),
`/authority/[slug]` (dossier: tabs, fee tally, citations).

Customer: `/orders` (list), `/order` (new-order form), `/uploads`
(file picker + validation), `/precheck` (WS status pill + snapshot),
`/billing`, `/login` (DEMO login).

Admin (admin host `/…`): `/admin` (dashboard), `/admin/triage`
(review queue), `/admin/console` (call console), `/admin/config`
(typed keys: bool/text editable, secrets set-new-only, never
displayed — `docs/specs/secure-config.md` §1.5).

API URLs derive from the home route id (`resolve('/(public)')` →
base + '/'). Kit 3 types expose no `base` export and narrow `Path`
to page pathnames, so `resolve()` pathnames can't carry `/api/…`
(see `apiRoot` in any wired page).

Shared libs: `src/lib/public-ui/` (both builds), `src/lib/customer/`
uploads helpers, `src/lib/admin/config-client.js`,
`src/lib/ws-client.js` (topic envelopes, backoff, Live /
Reconnecting / Offline status; lazy-imported after mount only).

## Setup

Requires Node 26 + npm. From this directory:

```sh
npm install
npm run dev          # public tree under /apostille
npm run dev:admin    # full tree at host root (admin shape)
npm run build && npm run build:admin && npm run smoke
npm run check        # svelte-check (syncs FULL routes via WEB_BUILD=admin)
npm run check:imports
npm test             # node --test over test/
```

`npm run check` must sync with `WEB_BUILD=admin`: Kit 3 sync reads
the inline vite options, so a plain sync would type only the public
tree and reject `/(admin)/…` route ids.

## JS budgets

- Day-1 skeleton gate: anonymous page total ≤ **150 KB** (raw JS+CSS,
  cold cache, excluding images/fonts).
- Week-3 targets per `docs/site-theme.md` §2: entry ≤ 40 KB Brotli,
  per-route chunk ≤ 30 KB, anonymous page ≤ 80 KB total, customer ≤
  120 KB. CI gate (`xtask web-budget`) not built yet.
- Measured 2026-10-27 (approx, whole-bundle Brotli): public JS total
  45 KB; anonymous home ≈ entry 1.3 KB + runtime chunks ≈ 28 KB +
  route nodes ≈ 2 KB ≈ **33 KB** — inside entry/page budgets. Formal
  per-navigation gate stays Week 3+.

## Notes

- SPA mode (`ssr = false`, `prerender = false`); `index.html` fallback
  covers deep links. Prerendered handbook pages may return later.
- No CSS framework, no runtime deps; mobile-first CSS in `src/app.css`.
- Theme: OS default + `localStorage` override (`theme:
  light|dark|auto`), blocking init snippet in `src/app.html`.
- i18n: `src/lib/i18n.ts` detects `?l=` → stored → `navigator.languages`
  (EN/ES/FR header labels only for now); new pages render EN-only.
- Links always use `resolve()` with route ids (group segments
  included, e.g. `/(customer)/orders`); groups are URL-free at
  runtime.
