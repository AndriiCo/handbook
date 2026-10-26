# Public web shell (SvelteKit)

Day-1 skeleton for the S4 lane. Public shell only — `(customer)` and
`(admin)` route groups come later and must never be imported from
public routes (see `docs/specs/spa-separation.md`).

## Setup

Requires Node 26 + npm. From this directory:

```sh
npm install
npm run dev      # serves under /apostille (matches kit.paths.base)
npm run build    # adapter-static output in build/, served by axum
npm run check    # svelte-check
```

`kit.paths.base` is `/apostille`: dev and prod both serve under
`/apostille`. Always build links with `resolve()`/`base` from
`$app/paths`, never root-absolute hrefs. (Kit 3 removed the `base`
export — use `resolve('/')`.)

## JS budgets

- Day-1 skeleton gate: anonymous page total ≤ **150 KB** (raw JS+CSS,
  cold cache, excluding images/fonts).
- Week-3 targets per `docs/site-theme.md` §2: entry ≤ 40 KB Brotli,
  per-route chunk ≤ 30 KB, anonymous page ≤ 80 KB total, customer ≤
  120 KB. CI gate (`xtask web-budget`) not built yet.

## Notes

- SPA mode (`ssr = false`, `prerender = false`); `index.html` fallback
  covers deep links. Prerendered handbook pages may return later.
- No CSS framework, no runtime deps; mobile-first CSS in `src/app.css`.
- Theme: OS default + `localStorage` override (`theme:
  light|dark|auto`), blocking init snippet in the root layout.
- i18n: `src/lib/i18n.ts` detects `?l=` → stored → `navigator.languages`
  (EN/ES/FR header labels only for now).
