# Prompt 045 — Tailwind v4 pilot: foundations + home/dossier (D38)

- Prompt timestamp (UTC): 2026-10-28T19:16:14Z
- Response timestamp (UTC): 2026-10-28T19:24:00Z
- Author: Andrii
- Agent/model: Muse Code (Muse Spark)

## Prompt

```text
Move fordward with a plan.
```

(Verbatim.) Read as: proceed with the UI plan. The framework choice was
genuinely open after prompt 044 removed the gate, so OPEN-29/30/31 went
to the author via structured input before any code: Tailwind v4, pilot
home+dossier together, drive-bys yes.

## Response

D38 recorded in `docs/specs/README.md` (same turn, per §6); proposal
marked DECIDED. Then implemented in one pass:

**Foundations.** `tailwindcss` + `@tailwindcss/vite` devDeps
(build-time only, zero runtime); plugin in `vite.config.ts`; `app.css`
imports `theme.css` + `utilities.css` WITHOUT preflight (a reset would
restyle the 33 unmigrated pages) and without cascade layers (utilities
stay unlayered, beating element selectors by specificity). `@theme`
maps the civic palette, font stacks, radius, and repo breakpoints
(720/1080/1440) onto the existing CSS vars — the three-state dark mode
(OS + `data-theme` + snippet) works with zero `dark:` variants.
`--accent-fill` defined for light (= accent) so one `bg-accent-fill`
token covers both themes. Admin uses zero utilities, so nothing leaks
across the two-build content scan (follow-up when admin adopts them).

**Pilot.** Home + dossier `<style>` blocks replaced with token
utilities: `sr-only`, `min-h-11` 44px targets, `text-[0.8rem]` kickers,
`md:` variants at the repo 720px, tab buttons via full-string
`tabCls` ternaries (active/idle colors never co-occur in the cascade),
dynamic classes kept as full literals for the scanner. Same look, new
mechanism — pixel-matched via arbitrary values where the scale
disagrees (0.8rem, 0.7rem rows, 0.4rem label rule). No `FactRow`
component: single-use (AGENTS.md §2), revisit with a second consumer.

**Drive-bys (OPEN-30).** Login `.notice.err` `warning-bg` → `error-bg`
(one line); print stub in `app.css` (hide `.menus` + `footer`, light
ink-on-paper — the one justified `!important`, beats scoped layout
rules, print-only).

## Commits

- `fdc9755` UI: Tailwind v4 foundations + home/dossier pilot (D38)
- (this log entry commits separately, per convention)

## Verification

- `cargo xtask ci` exit 0 (incl. node 78/78, svelte-check 0/0,
  check:imports, both builds, smoke).
- Generated-CSS proof (layout chunk `0.*.css`): `bg-accent-fill`,
  `text-ink-muted`, `border-line`, `font-display`, `sr-only`,
  `md\:text-4xl`, `min-h-11`, `border-accent`, `grid-cols-` all present;
  `@media (width>=720px)` (range syntax, exact repo breakpoint);
  `--color-accent-fill:var(--accent-fill)` with light `#0b4a75` /
  dark `#164e74`; `data-theme` overrides + `prefers-color-scheme`
  intact; `@media print` present; preflight absent
  (`border:0 solid` × 0, `::placeholder` × 0), our box-sizing intact —
  unmigrated pages render byte-identical.
- Advisory `web-budget`: 226136 → 230050 bytes (+3914: theme layer
  + pilot utilities, minus deleted page styles). No gate since 044.

## Follow-ups

- Remaining 33 pages migrate opportunistically (proposal Phases 2–3);
  each migration deletes a `<style>` block and re-reports the total.
- When an admin page first uses utilities, scope the content scan
  (today: zero admin utilities, zero public-bundle leak).
- Preflight stays out until a page-by-page audit says otherwise (form
  controls need `[font:inherit]` meanwhile).
- No deploy in this turn (UI-only change, unshipped — rides the next
  release train).
