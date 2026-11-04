# Web shells (SvelteKit, two builds)

One codebase, three route groups, two deploy units
(`docs/specs/spa-separation.md` §1.2):

- `src/routes/(public)/` — anonymous: home search, authority guides,
  follow/notify links. Must never import customer/admin libs.
- `src/routes/(customer)/` — signed-in customers: orders, forms,
  uploads, precheck, shipping, billing, security (passkeys and linked
  methods).
- `src/routes/(admin)/` — staff only. Ships ONLY in the admin build
  served from `admin.andrii.ai`.

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

Public (`/apostille/…`): `/` (search-first home),
`/authority/[...trail]` (the authority guide), `/in/[...path]`
(country and place index), `/notify/confirm`, `/notify/unsubscribe`.

Customer (`/apostille/…`): `/login`, `/order` (new order, including
the free form fill), `/orders`, `/forms`, `/uploads`, `/precheck`,
`/shipping`, `/billing`, `/security`, `/report`.

Admin (admin host, `/…`): `/dashboard`, `/triage` and `/review`
(review queue), `/authorities`, `/sources`, `/packs`, `/calls`,
`/compose`, `/email`, `/mail`, `/outbox`, `/labels`, `/staff-orders`,
`/work` (plant work queue), `/workers` (URL ingest), `/trace`,
`/staff`, `/enrol`, `/staff-login`, and `/config` (typed keys;
secrets are write-only and never displayed —
`docs/specs/secure-config.md` §1.5).

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

## Bundle sizes

No size budget is enforced. `node scripts/route-budget.mjs [route id]`
(after `npm run build`) prints Brotli sizes per route and per
navigation, for reference only.

## Notes

- SPA mode (`ssr = false`, `prerender = false`); `index.html` fallback
  covers deep links. Prerendered handbook pages may return later.
- Tailwind v4 (decision D38) on CSS-variable themes; mobile-first;
  `npm run check:tailwind` guards the setup.
- Theme: OS default + `localStorage` override (`theme:
  light|dark|auto`), blocking init snippet in `src/app.html`.
- i18n: `src/lib/i18n.ts` detects `?l=` → stored → `navigator.languages`
  (EN/ES/FR header labels only for now); new pages render EN-only.
- Links always use `resolve()` with route ids (group segments
  included, e.g. `/(customer)/orders`); groups are URL-free at
  runtime.
