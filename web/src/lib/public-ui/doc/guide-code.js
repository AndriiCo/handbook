// The customer guide's code as one chunk (R2): the presenter and every
// guide component. The authority page requests it as soon as its route
// module loads, so it downloads alongside the first API reads (`/facts`,
// `/plan`) instead of ahead of them, and the route chunk keeps only the
// reads, the live updates and the loading state. Every visit still
// loads it: it counts toward the page's 30 KB budget, which the guide
// is over (web/README.md, JS budgets).
export { default as DossierGuide } from './DossierGuide.svelte';
export { changedItems, present } from '../present.js';
