# Role: lane `guide` (readable dossier)

A separate Muse session in its own worktree, on its own branch, merged by the builder under
`.agents/roles/builder.md`. Rules: AGENTS.md. Never edit `.agents/`, `.muse/`, `AGENTS.md` or
`docs/ops/operator-today.md`; never push to `main`; never deploy.

## Goal
Prompt M1 of the 11-01 review: a deterministic, cited, step-by-step guide on
`/authority/[slug]`, built from accepted facts, web-only.

## You own (touch nothing else)
- `web/src/lib/public-ui/guide.js`, `GuideStep.svelte`, `CitationMark.svelte`, `GuideEmpty.svelte`
- `web/src/routes/(public)/authority/[slug]/+page.svelte`, `web/src/lib/public-ui/handbook.js`
- `web/e2e/fixtures/facts-*.json`, `web/e2e/fixtures/authorities.json`
- `web/test/public-guide.test.mjs`
- `docs/lanes/guide.md`, your lines in `docs/prompts/LOG.md`
Request `web/scripts/shots.mjs` ROUTES changes from the builder in `docs/lanes/guide.md`.

## Proof
The AGENTS.md §4 web gates. Shots of /authority/vietnam, /authority/usa-wa-sos and /authority/hcch
at 390 and 1440, light and dark. Commit `guide-<n>`. Cut-off: Mon 11-02 12:00 PST, so the builder
can merge before 15:00.

