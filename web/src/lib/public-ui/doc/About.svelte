<script lang="ts">
	import type { Component, Snippet } from 'svelte';
	import { resolve } from '$app/paths';
	import { issuerName, partOf, traceAttrs } from './items.js';
	import type { Item } from './contract.js';

	// "About this guide" (plan §3 row 13, printed): who we are not, what
	// the office charges when you apply yourself, how a value gets here,
	// and what this page is not. A section, never a footer (print hides
	// footers). "You can apply directly to …" names only an office that
	// issues apostilles (`issuer`: a profiled US state, or a page whose
	// lede names its issuer); any other page (a country's receiving
	// authorities) keeps the "Unofficial guide" line alone. The fee is
	// the per-document fee's own amount, read out of its value and traced
	// to it; the clause is dropped unless that value is `ok`. Then a link
	// to report a change, and the follow form, which loads only when this
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

	const office = $derived(issuerName(name, country));
	const amount = $derived.by((): Item | null => {
		if (!fee || fee.state !== 'ok' || fee.factIds.length === 0) return null;
		const m = /\$\s?\d[\d,]*(?:\.\d{1,2})?/.exec(fee.display);
		return m ? partOf(fee, 'amount', m[0]) : null;
	});

	let root = $state<HTMLElement | null>(null);
	let Follow = $state<Component<any> | null>(null);

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
		<h2 id="about-h" class="text-h3 m-0 font-semibold text-ink">About this guide</h2>
		<p class="mt-3 mb-0 max-w-prose text-ink">
			Unofficial guide, not a government website.{#if issuer && office}{' '}You can apply directly to {office}{#if amount}{' '}for
					<span class="font-medium" {...traceAttrs(amount)}>{amount.display}</span>{@render inline?.(amount)} per
					document{/if}.{/if}
		</p>
		<p class="mt-3 mb-0 max-w-prose text-ink-muted">
			We publish a detail only after a staff reviewer matches it to the office's own words. General
			information, not legal advice — confirm with the office before you send documents or money.
		</p>
		<p class="mt-3 mb-0 text-sm print:hidden">
			<a
				href={resolve('/(customer)/report')}
				class="inline-flex min-h-6 items-center font-medium text-accent-strong underline underline-offset-4"
				>Report a change on the office's pages</a
			>
		</p>
		{#if Follow}
			<div class="mt-8 print:hidden">
				<Follow {slug} name={name} apiBase={apiBase} />
			</div>
		{/if}
	</div>
</section>
