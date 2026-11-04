<script lang="ts">
	// Cite: the [n] marker group for one value (R2 citations mode).
	//
	// Props ($props):
	// - refs: MarkerRef[] for this item, or the whole
	//   `numberSources(items).refs` map (Record<itemId, MarkerRef[]>), in
	//   which case `itemId` picks the item's list. MarkerRef is the
	//   contract's Ref ({ n, letter, refId }) plus `letters` and `key`.
	// - itemId: string, the Item id (contract.js); stamped on each marker
	//   as data-cite-item for the shared card.
	// - reserve?: boolean (default true). Keeps the gutter span in the row
	//   while citations are off, as wide and as tall as this item's group
	//   will be, so turning them on reflows nothing. Pass false inside
	//   running prose.
	//
	// Placement: the gutter is the last cell of a value row. Its track
	// must be allowed to grow to the reserved width: a grid
	// `minmax(9rem,14rem) 1fr minmax(3.5ch,max-content)`, not a fixed
	// `3.5ch`, which a group of two or more sources would overflow. In a
	// <dl>, put it inside the <dd> (a dl row may hold only dt and dd):
	// make the dd a flex row holding the value, then this component.
	//
	// Markup while citations are on, one link per source:
	//   <a class="cite" href="#src-N" id={refId} role="doc-noteref"
	//      aria-describedby="sources-h"><span class="sr-only">Citation </span>N</a>
	// Brackets, the thin space and the comma between markers come from
	// CSS or aria-hidden spans (no <sup>), so each link is named
	// "Citation N". While off no marker is in the DOM, so print and specs
	// see none. Markers are links, never buttons: the toggle stays the
	// page's first button named "Citations".
	//
	// Targets (WCAG 2.5.8). In the gutter, which is not a sentence, each
	// link is its own box of at least 24 x 24 px ("[1," "4]"), side by
	// side, so no two targets overlap at any font size or number of
	// digits; the box is lifted so the digits sit where a raised marker
	// would. In running prose (reserve false) the group stays inline and
	// tight (a target in a sentence is exempt) and ::before pads each
	// link's hit area to 24 x 24 px.
	import { citations } from '../citations.svelte.js';
	import type { MarkerRef } from '../cite.js';

	let {
		refs = [],
		itemId,
		reserve = true
	}: {
		refs?: MarkerRef[] | Record<string, MarkerRef[]> | null;
		itemId: string;
		reserve?: boolean;
	} = $props();

	const list = $derived.by((): MarkerRef[] => {
		if (Array.isArray(refs)) return refs;
		const own = refs && itemId ? refs[itemId] : null;
		return Array.isArray(own) ? own : [];
	});
	const shown = $derived(citations.on && list.length > 0);

	// The group's width, known before any marker renders, in the gutter's
	// own em (the group is 0.75em): each box is max(24px, its text). Its
	// text, in marker em: the padding, the digits, "[" on the first, "]"
	// on the last and "," on the others. Measured in Chromium (Public Sans,
	// 600, tabular): a digit is 0.7, a bracket 0.32, a comma 0.26; the
	// factors round up for the system-ui fallback.
	const PAD = 0.2;
	const DIGIT = 0.75;
	const BRACKET = 0.45;
	const COMMA = 0.35;
	const em = (marker: number) => `${(0.75 * marker).toFixed(3)}em`;
	const reserved = $derived.by((): string | null => {
		if (list.length === 0) return null;
		const last = list.length - 1;
		const boxes = list.map((r, i) => {
			const text =
				PAD +
				DIGIT * String(r.n).length +
				(i === 0 ? BRACKET : 0) +
				(i === last ? BRACKET : COMMA);
			return `max(24px, ${em(text)})`;
		});
		return `calc(${boxes.join(' + ')})`;
	});
</script>

{#snippet group()}
	<span class="cite-group" class:cite-row={reserve} data-cite-group={itemId}
		>{#each list as r, i (r.refId)}<a
				class="cite"
				href={`#src-${r.n}`}
				id={r.refId}
				role="doc-noteref"
				aria-describedby="sources-h"
				data-cite-n={r.n}
				data-cite-letters={(r.letters?.length ? r.letters : [r.letter]).join(' ')}
				data-cite-item={itemId}
				data-cite-key={r.key ?? ''}
				><span class="sr-only">{'Citation '}</span><span class="cite-n">{r.n}</span
				>{#if i < list.length - 1}<span class="cite-sep" aria-hidden="true">,</span>{/if}</a
			>{/each}</span
	>
{/snippet}

{#if reserve}
	<span
		class="cite-gutter"
		class:cite-held={list.length > 0}
		data-cite-gutter
		style:--cite-reserve={reserved}
	>
		{#if shown}{@render group()}{/if}
	</span>
{:else if shown}
	{@render group()}
{/if}

<style>
	/* The gutter column: present while off too, and as wide and as tall
	   as its group will be (--cite-reserve; a 24 px box lifted 6 px), so
	   rows never reflow and a group never reaches over the value. The
	   group stays on one line, end-aligned, at the gutter's top: raised
	   against the value's first line. As a grid or flex item the gutter
	   stretches with the row; inline (inside a <dd>), it hangs from the
	   top of its line, so it is the same box on and off and never makes
	   the line taller. */
	.cite-gutter {
		display: inline-flex;
		justify-content: flex-end;
		align-items: flex-start;
		min-inline-size: max(3.5ch, var(--cite-reserve, 0px));
		vertical-align: top;
	}

	.cite-gutter.cite-held {
		min-block-size: 18px;
	}

	.cite-group {
		color: var(--accent);
		font-size: 0.75em;
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
	}

	.cite {
		position: relative;
		border-radius: var(--radius-xs);
		color: inherit;
		font-weight: 600;
		padding-inline: 0.1em;
		scroll-margin-block: 5rem;
		text-decoration: none;
		white-space: nowrap;
	}

	.cite:hover > .cite-n {
		text-decoration: underline;
		text-underline-offset: 2px;
	}

	/* Row gutter: every link is a box of at least 24 x 24 px holding its
	   digits, "[" on the first, "]" on the last and "," between; the
	   boxes sit side by side, so neighbouring targets never overlap. The
	   group is lifted 6 px, so the digits (centred in their box) sit at
	   the top of the gutter like a raised marker. */
	.cite-row {
		display: flex;
		line-height: 1;
		margin-block-start: -6px;
	}

	.cite-row .cite {
		display: flex;
		align-items: center;
		justify-content: center;
		min-block-size: 24px;
		min-inline-size: 24px;
	}

	.cite-row .cite:first-child::before {
		content: '[';
		content: '[' / '';
	}

	.cite-row .cite:last-child::after {
		content: ']';
		content: ']' / '';
	}

	/* Prose: a tight inline group raised against the text, brackets
	   around it, a thin space before; silent to screen readers. */
	.cite-group:not(.cite-row) {
		line-height: 0;
		vertical-align: 0.45em;
	}

	.cite-group:not(.cite-row)::before {
		content: '\2009[';
		content: '\2009[' / '';
	}

	.cite-group:not(.cite-row)::after {
		content: ']';
		content: ']' / '';
	}

	.cite-group:not(.cite-row) .cite-sep {
		margin-inline-end: 0.3em;
	}

	/* Prose: pads each link's hit area to 24 x 24 px around the digits. */
	.cite-group:not(.cite-row) .cite::before {
		content: '';
		position: absolute;
		top: 50%;
		left: 50%;
		height: 24px;
		width: max(100%, 24px);
		transform: translate(-50%, -50%);
	}

	/* Set by CitationLayer on the marker whose card is open. */
	:global(a.cite[data-cite-active]) {
		background: var(--surface-2);
		color: var(--text);
	}
</style>
