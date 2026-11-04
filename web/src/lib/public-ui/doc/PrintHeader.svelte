<script lang="ts">
	// PrintHeader: the letterhead on paper (R2 print).
	//
	// Props ($props):
	// - title: string, the page's h1 (Page.header.h1).
	// - url?: string | null, the canonical URL; default origin + path at
	//   print time (no ?cite, no #hash).
	// - trust?: Page.header.trust | null ({ capturedMin, capturedMax,
	//   acceptedMax, pages, calls, emails }), for the captured/approved
	//   line.
	// - dataAsOf?: number | string | Date | null, when the page's data was
	//   read (unix seconds, ISO or Date); default trust.acceptedMax.
	//
	// On screen it is an empty hidden hook (`data-print-header`, which
	// scopes the guide's print rules in app.css). Its text renders only
	// while printing (beforeprint or the print media query), so screen
	// readers and specs never meet a second copy of the title. On paper:
	// title, canonical URL, the captured/approved line, "Data as of …",
	// "Printed …" (set on beforeprint) and "Check the live page before
	// you send documents or money." No record ids.
	// It also opens every <details> on beforeprint (except inside #made)
	// and closes them again on afterprint.
	import { flushSync, onMount } from 'svelte';
	import { citations } from '../citations.svelte.js';
	import { formatDay, formatMoment, trustLine } from '../cite.js';
	import { guide } from './lang.svelte.js';

	type Trust = {
		capturedMin: string | null;
		capturedMax: string | null;
		acceptedMax: number | null;
		pages: number;
		calls: number;
		emails: number;
	};

	let {
		title = '',
		url = null,
		trust = null,
		dataAsOf = null
	}: {
		title?: string;
		url?: string | null;
		trust?: Trust | null;
		dataAsOf?: number | string | Date | null;
	} = $props();

	let printing = $state(false);
	let printedAt = $state<Date | null>(null);
	let pageUrl = $state('');

	const t = $derived(guide.t.print);
	const paper = $derived({ year: 'always' as const, lang: guide.lang });
	const trustText = $derived(trustLine(trust, paper));
	const asOf = $derived(formatMoment(dataAsOf ?? trust?.acceptedMax ?? null, paper));
	const printed = $derived(printedAt ? formatDay(printedAt, paper) : '');
	const stamp = $derived(
		[asOf ? t.dataAsOf(asOf) : '', printed ? t.printed(printed) : '']
			.filter(Boolean)
			.join(' · ')
	);

	onMount(() => {
		let opened: HTMLDetailsElement[] = [];
		const before = () => {
			printedAt = new Date();
			pageUrl = url || `${location.origin}${location.pathname}`;
			for (const d of document.querySelectorAll('details')) {
				if (d.open || d.closest('#made')) continue;
				d.open = true;
				opened.push(d);
			}
			flushSync(() => {
				printing = true;
			});
		};
		const after = () => {
			for (const d of opened) d.open = false;
			opened = [];
			printing = false;
		};
		const media = typeof matchMedia === 'function' ? matchMedia('print') : null;
		const onMedia = (e: MediaQueryListEvent) => (e.matches ? before() : after());
		addEventListener('beforeprint', before);
		addEventListener('afterprint', after);
		media?.addEventListener?.('change', onMedia);
		return () => {
			removeEventListener('beforeprint', before);
			removeEventListener('afterprint', after);
			media?.removeEventListener?.('change', onMedia);
			after();
		};
	});
</script>

<div class="print-header" data-print-header>
	{#if printing}
		<p class="m-0 text-sm font-semibold">{t.brand}</p>
		{#if title}
			<p class="text-h3 m-0 mt-1 font-semibold">{title}</p>
		{/if}
		<p class="m-0 font-mono text-xs break-all">{pageUrl}</p>
		{#if trustText}
			<p class="m-0 mt-2 text-sm">{trustText}</p>
		{/if}
		{#if stamp}
			<p class="m-0 text-sm">{stamp}</p>
		{/if}
		{#if citations.on}
			<p class="m-0 text-sm">{t.brackets}</p>
		{/if}
		<p class="m-0 mt-2 text-sm font-semibold">{t.checkLive}</p>
	{/if}
</div>

<style>
	.print-header {
		display: none;
	}

	@media print {
		.print-header {
			display: block;
			border-bottom: 1px solid var(--border);
			margin-bottom: 12pt;
			padding-bottom: 8pt;
		}
	}
</style>
