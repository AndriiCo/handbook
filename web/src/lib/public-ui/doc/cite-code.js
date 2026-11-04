// Citations mode's card and endnotes as one chunk (R2, plan §2 route
// budget). Citations are off by default, so DossierGuide requests this
// only the first time they turn on (the button, Alt+C, `?cite=1` or the
// stored preference): most readers never download the card, the endnotes
// or bits-ui's popover. The markers (Cite.svelte) stay in the guide: they
// render nothing while citations are off.
export { default as CitationLayer } from './CitationLayer.svelte';
export { default as Endnotes } from './Endnotes.svelte';
