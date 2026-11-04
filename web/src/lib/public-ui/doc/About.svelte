<script lang="ts">
	import type { Component, Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { issuerName, partOf, traceAttrs } from './items.js';
	import { draftKey, loadDraft } from '../report-draft.js';
	import { guide } from './lang.svelte.js';
	import type { Item } from './contract.js';

	// "About this guide" (plan §3 row 13, printed): who we are not, what
	// the office charges when you apply yourself, how a value gets here,
	// and what this page is not. A section, never a footer (print hides
	// footers). "You can apply directly to …" names only an office that
	// issues apostilles (`issuer`: a profiled US state, or a page whose
	// lede names its issuer); any other page (a country's receiving
	// authorities) keeps the "Unofficial guide" line alone. The fee is
	// the per-document fee's own amount, read out of its value and traced
	// to it; the clause is dropped unless that value is `ok`. Then
	// "Report a change", which opens the report form docked beside this
	// page (ReportPanel, loaded on open; `#report` reopens it, e.g. after
	// sign-in; the link still goes to /report without JS), and the
	// follow form, which loads only when this
	// band nears the screen (and shows itself only while follow emails
	// are on).
	//
	// Props:
	// - name: the authority's name.
	// - issuer: the page is an issuing office's (default false).
	// - country: the authority's country (no "the" before a name that
	//   starts with it).
	// - fee: the `fees.per_document` Item, or null.
	// - inline: snippet `inline(item)` (a marker inside the sentence).
	// - slug, apiBase: for the follow form.
	let {
		name,
		issuer = false,
		country = null,
		fee = null,
		inline,
		slug,
		apiBase = ''
	}: {
		name: string;
		issuer?: boolean;
		country?: string | null;
		fee?: Item | null;
		inline?: Snippet<[Item]>;
		slug: string;
		apiBase?: string;
	} = $props();

	const t = $derived(guide.t.about);
	const office = $derived(issuerName(name, country, guide.lang));
	const amount = $derived.by((): Item | null => {
		if (!fee || fee.state !== 'ok' || fee.factIds.length === 0) return null;
		const m = /\$\s?\d[\d,]*(?:\.\d{1,2})?/.exec(fee.display);
		return m ? partOf(fee, 'amount', m[0]) : null;
	});

	let root = $state<HTMLElement | null>(null);
	let Follow = $state<Component<any> | null>(null);
	let Panel = $state<Component<any> | null>(null);
	let reporting = $state(false);
	let hasDraft = $state(false);

	const loginHref = resolve('/(customer)/login');
	const draftSaved = () => {
		try {
			return !!loadDraft(localStorage, draftKey(slug));
		} catch {
			return false;
		}
	};

	function openReport(event?: MouseEvent) {
		if (event && (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0)) return;
		event?.preventDefault();
		reporting = true;
		if (!Panel) {
			import('./ReportPanel.svelte')
				.then((m) => (Panel = m.default as Component<any>))
				.catch(() => (window.location.href = reportHref));
		}
	}

	function closeReport() {
		reporting = false;
		hasDraft = draftSaved();
		if (window.location.hash === '#report') {
			history.replaceState(history.state, '', window.location.pathname + window.location.search);
		}
	}

	const reportHref = $derived(`${resolve('/(customer)/report')}?destination=${encodeURIComponent(slug)}`);

	onMount(() => {
		hasDraft = draftSaved();
		const onHash = () => {
			if (window.location.hash === '#report') openReport();
		};
		onHash();
		window.addEventListener('hashchange', onHash);
		return () => window.removeEventListener('hashchange', onHash);
	});

	$effect(() => {
		const el = root;
		if (!el || Follow) return;
		const load = () => {
			import('../FollowCard.svelte').then((m) => (Follow = m.default as Component<any>)).catch(() => {});
		};
		if (typeof IntersectionObserver === 'undefined') {
			load();
			return;
		}
		const io = new IntersectionObserver(
			(seen) => {
				if (!seen.some((e) => e.isIntersecting)) return;
				io.disconnect();
				load();
			},
			{ rootMargin: '400px 0px' }
		);
		io.observe(el);
		return () => io.disconnect();
	});
</script>

<section bind:this={root} aria-labelledby="about-h" class="border-t border-line py-10 xl:py-16" data-about>
	<div class="max-w-3xl">
		<h2 id="about-h" class="text-h3 m-0 font-semibold text-ink">{t.heading}</h2>
		<p class="mt-3 mb-0 max-w-prose text-ink">
			{t.unofficial}{#if issuer && office}{' '}{t.applyDirect[0]}{office}{t.applyDirect[1]}{#if amount}{t.perDocument[0]}<span
						class="font-medium"
						{...traceAttrs(amount)}>{amount.display}</span
					>{@render inline?.(amount)}{t.perDocument[1]}{/if}.{/if}
		</p>
		<p class="mt-3 mb-0 max-w-prose text-ink-muted">{t.rule}</p>
		<p class="mt-3 mb-0 text-sm print:hidden">
			<a
				href={reportHref}
				onclick={openReport}
				aria-expanded={reporting}
				class="inline-flex min-h-6 items-center font-medium text-accent-strong underline underline-offset-4"
				>{hasDraft ? t.reportContinue : t.report}</a
			>
		</p>
		{#if reporting && Panel}
			<Panel {slug} {name} {apiBase} {loginHref} onclose={closeReport} />
		{/if}
		{#if Follow}
			<div class="mt-8 print:hidden">
				<Follow {slug} name={name} apiBase={apiBase} />
			</div>
		{/if}
	</div>
</section>
