<script lang="ts">
	// CitationsToggle: the citations switch, a Print button and the
	// screen-only print hint (R2 citations mode).
	//
	// Props ($props):
	// - count?: number | null. Numbered sources for the announcement;
	//   default the count Endnotes and CitationLayer report (calculations
	//   left out, cite.js sourceCount).
	// - print?: boolean (default true). Render the Print button.
	//
	// Mount it once, in the header, when at least one value is cited.
	// On mount it reads the starting state (?cite=1|0, then hb_cite,
	// then off), installs the Alt/⌥+C hotkey (doc/hotkey.js) and mirrors
	// html[data-cite] for print.
	// Markup: <button aria-pressed aria-keyshortcuts="Alt+C"> named
	// "Citations": the ⌥C / Alt+C hint is aria-hidden and hidden on touch
	// screens, (hover: none). A polite live region announces "Citations
	// shown: N sources numbered" / "Citations hidden". Next to Print, a
	// screen-only hint: "Turn on citations to print sources." while off,
	// "Sources print at the end." while on. Both lines share one grid
	// cell, so switching never changes the block's size at any width.
	//
	// Hiding citations removes the markers, the card and the endnotes.
	// From the button, focus stays on the button. From Alt+C with focus
	// on one of them, focus moves without scrolling to what stays in
	// place: the value next to the marker (or its paragraph), or the band
	// that follows the endnotes; never to the top of the page.
	import { onMount } from 'svelte';
	import CheckIcon from '@lucide/svelte/icons/check';
	import PrinterIcon from '@lucide/svelte/icons/printer';
	import TextQuoteIcon from '@lucide/svelte/icons/text-quote';
	import { Kbd } from '../../components/ui/kbd/index.js';
	import { citations } from '../citations.svelte.js';
	import { installCitationHotkey } from './hotkey.js';

	let { count = null, print = true }: { count?: number | null; print?: boolean } = $props();

	let button = $state<HTMLButtonElement | null>(null);
	let apple = $state(false);

	function applePlatform(): boolean {
		try {
			const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
			const p = nav.userAgentData?.platform || nav.platform || nav.userAgent || '';
			return /mac|iphone|ipad|ipod/i.test(p);
		} catch {
			return false;
		}
	}

	const CITED = 'a.cite, #sources, [data-cite-card]';

	// The nearest element that stays when a marker leaves: in a row, the
	// value before the reserved gutter (or the gutter itself); in prose,
	// the text around the group.
	function besideMarker(marker: Element): HTMLElement | null {
		const gutter = marker.closest('[data-cite-gutter]');
		if (gutter instanceof HTMLElement) {
			let at: Element | null = gutter;
			for (let depth = 0; at && depth < 3; depth++, at = at.parentElement) {
				const before = at.previousElementSibling;
				if (before instanceof HTMLElement) return before;
			}
			return gutter;
		}
		const host = marker.closest('.cite-group')?.parentElement;
		return host instanceof HTMLElement ? host : null;
	}

	function placeFor(active: Element): HTMLElement | null {
		const marker =
			active.closest('a.cite') ??
			(active.closest('[data-cite-card]') ? document.querySelector('a.cite[data-cite-active]') : null);
		if (marker) return besideMarker(marker);
		const notes = active.closest('#sources');
		if (notes) {
			const near = notes.nextElementSibling ?? notes.previousElementSibling;
			return near instanceof HTMLElement ? near : null;
		}
		return null;
	}

	// Focus without scrolling; a temporary tabindex makes any element
	// focusable and leaves again on blur.
	function holdFocus(el: HTMLElement) {
		if (!el.hasAttribute('tabindex')) {
			el.setAttribute('tabindex', '-1');
			el.addEventListener('blur', () => el.removeAttribute('tabindex'), { once: true });
		}
		el.focus({ preventScroll: true });
	}

	function toggle(fromButton: boolean) {
		if (typeof count === 'number') citations.count = count;
		if (citations.on) {
			// Markers, the card and the endnotes leave the DOM: keep focus
			// on the page rather than dropping it to <body>.
			const a = document.activeElement;
			if (a instanceof Element && a !== button && a.closest(CITED)) {
				const place = fromButton ? null : placeFor(a);
				if (place) holdFocus(place);
				else button?.focus({ preventScroll: true });
			}
		}
		citations.toggle();
	}

	onMount(() => {
		apple = applePlatform();
		citations.init();
		citations.message = '';
		const detach = citations.attach();
		const remove = installCitationHotkey({ onToggle: () => toggle(false) });
		return () => {
			remove();
			detach();
		};
	});
</script>

<div class="cite-toggle flex flex-wrap items-center gap-2">
	<button
		bind:this={button}
		type="button"
		aria-pressed={citations.on}
		aria-keyshortcuts="Alt+C"
		onclick={() => toggle(true)}
		class="cite-switch inline-flex min-h-11 cursor-pointer items-center gap-2 rounded-sm border border-line bg-bg px-4 font-medium text-ink transition-colors hover:border-focus"
	>
		{#if citations.on}
			<CheckIcon class="size-4 flex-none" aria-hidden="true" />
		{:else}
			<TextQuoteIcon class="size-4 flex-none" aria-hidden="true" />
		{/if}
		Citations
		<Kbd class="cite-kbd" aria-hidden="true">{apple ? '⌥C' : 'Alt+C'}</Kbd>
	</button>
	{#if print}
		<button
			type="button"
			onclick={() => window.print()}
			class="inline-flex min-h-11 cursor-pointer items-center gap-2 rounded-sm border border-line bg-bg px-4 font-medium text-ink transition-colors hover:border-focus"
		>
			<PrinterIcon class="size-4 flex-none" aria-hidden="true" />
			Print
		</button>
	{/if}
	<p class="cite-hint m-0 text-sm text-ink-muted" data-print-hide>
		<span class:cite-hint-off={citations.on}>Turn on citations to print sources.</span>
		<span class:cite-hint-off={!citations.on}>Sources print at the end.</span>
	</p>
	<span class="sr-only" aria-live="polite" aria-atomic="true">{citations.message}</span>
</div>

<style>
	/* Pressed: the check icon says "on"; the fill only reinforces it. */
	.cite-switch[aria-pressed='true'] {
		background: var(--surface);
		border-color: var(--focus);
	}

	/* The shortcut hint is for keyboards; touch screens never show it. */
	@media (hover: none) {
		.cite-toggle :global(.cite-kbd) {
			display: none;
		}
	}

	/* Both hints hold the same cell, so the block keeps its size (and the
	   page below it its place) when citations turn on or off. The hidden
	   one leaves the accessibility tree and innerText. */
	.cite-hint {
		display: grid;
	}

	.cite-hint > span {
		grid-area: 1 / 1;
	}

	.cite-hint-off {
		visibility: hidden;
	}
</style>
