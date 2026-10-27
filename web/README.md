# Public web shell (SvelteKit)

Day-1 skeleton for the S4 lane. Public shell + `(customer)` route
group skeleton. `(admin)` comes later. Public routes must never
import customer/admin routes (see `docs/specs/spa-separation.md`).

## Customer routes

`src/routes/(customer)/` (logged-in side, wired-where-possible):

- `login/` — DEMO login form (`customer_ref` → `POST
  /apostille/api/v1/demo/login`). 200 shows logged-in state + link
  to order; 422 shows an inline error.
- `order/` — order shell (authority select, document-type radio,
  contact/address with native `required` validation). Submit shows
  "not wired yet" — no fake success.
- `billing/` — invoice table shell (2 placeholder rows) + "no
  invoices yet" empty state. No payment buttons.
- `(customer)/+layout.svelte` — nav (Order, Billing, Log out→`/`)
  + `GET /apostille/api/v1/me` on mount: shows `customer_ref` or
  "not logged in" with a login link.

API URLs are built from `resolve('/')` + `/api/v1/...` — relative,
never a hardcoded host. Routes land at `/apostille/login`,
`/apostille/order`, `/apostille/billing` (group name is URL-free).

## Demo-login flow

1. Customer enters any reference on `/apostille/login`.
2. Browser `fetch`es `POST /apostille/api/v1/demo/login` (JSON
   `{ customer_ref }`, `credentials: same-origin`).
3. On 200 the **server** sets the session cookie (HttpOnly —
   unreadable from JS); the page shows the logged-in state.
4. The customer layout probes `GET /apostille/api/v1/me` on mount
   to display the identity.

## NOT wired yet

- Order submit (shows a notice, sends nothing).
- Invoices (placeholder rows + empty state only; no data fetch).
- Real auth (no passkey/OAuth/magic link; no logout endpoint —
  the Log out link just returns home).
- The demo endpoints themselves: no axum handler exists yet, so
  login currently fails with "service unreachable" until the API
  lane lands.

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
