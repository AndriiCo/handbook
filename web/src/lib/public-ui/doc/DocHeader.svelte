<script lang="ts">
	import type { Snippet } from 'svelte';
	import { resolve } from '$app/paths';
	import Heading from '../../components/brand/Heading.svelte';
	import Guilloche from '../../components/brand/Guilloche.svelte';
	import Flag from '../../components/brand/Flag.svelte';
	import { countryName, silhouetteFor } from '../authority-meta.js';
	import { trustLine } from '../cite.js';
	import CitationsToggle from './CitationsToggle.svelte';
	import type { Header } from './contract.js';

	// The guide's header band (plan §3 row 0): breadcrumbs, the eyebrow,
	// the flag and the one h1 (Heading, data-heading), the lede naming
	// the issuer, the service status right under them, the independence
	// line with the official site, the captured/approved trust line, and
	// the citations switch with Print once a value is cited. The region
	// outline and the guilloche are decoration (`data-graphic`), hidden
	// in print and on phones.
	//
	// Props:
	// - header: `page.header` (present.js).
	// - slug: the authority slug (the guilloche seed).
	// - region, country: the authority row's, for the outline and the
	//   flag's name.
	// - cited: at least one value is numbered (the switch renders).
	// - count: numbered sources, for the switch's announcement.
	// - status: snippet with the service status band.
	let {
		header,
		slug,
		region = null,
		country = null,
		cited = false,
		count = null,
		status
	}: {
		header: Header;
		slug: string;
		region?: string | null;
		country?: string | null;
		cited?: boolean;
		count?: number | null;
		status?: Snippet;
	} = $props();

	const sil = $derived(silhouetteFor({ region, country }));
	const trust = $derived(trustLine(header.trust));
</script>

<header class="relative overflow-clip py-10 xl:py-16">
	{#if sil}
		<svg
			viewBox={`0 0 ${sil.width} ${sil.height}`}
			class="pointer-events-none absolute top-1/2 -right-10 hidden w-[420px] -translate-y-1/2 opacity-[0.07] [mask-image:radial-gradient(circle,black,transparent_72%)] sm:block"
			aria-hidden="true"
			data-graphic="silhouette"
		>
			<path d={sil.d} style="fill: var(--color-ink);" />
		</svg>
		<Guilloche
			seed={slug}
			class="pointer-events-none absolute top-1/2 -right-24 hidden size-96 -translate-y-1/2 text-brand-300 opacity-60 [mask-image:radial-gradient(circle,black,transparent_70%)] sm:block dark:text-brand-700"
		/>
	{/if}
	<div class="relative max-w-3xl">
		<nav aria-label="Breadcrumb">
			<ol class="m-0 flex list-none flex-wrap gap-1 p-0 text-sm text-ink-muted">
				<li class="m-0 p-0">
					<a class="inline-flex min-h-6 items-center text-ink-muted no-underline hover:text-ink hover:underline" href={resolve('/(public)')}>Home</a>
				</li>
				<li class="m-0 p-0" aria-hidden="true">/</li>
				{#each header.crumbs as crumb, i (`${i}-${crumb.label}`)}
					<li class="m-0 p-0">
						{#if crumb.href}
							<a class="inline-flex min-h-6 items-center text-ink-muted no-underline hover:text-ink hover:underline" href={crumb.href}>
								{crumb.label}
							</a>
						{:else}
							{crumb.label}
						{/if}
					</li>
					<li class="m-0 p-0" aria-hidden="true">/</li>
				{/each}
				<li class="m-0 p-0" aria-current="page">{region || header.h1}</li>
			</ol>
		</nav>
		<p class="text-eyebrow mt-6 mb-0 font-semibold tracking-[0.06em] text-ink-muted uppercase">{header.eyebrow}</p>
		<div class="mt-2 flex items-center gap-3">
			{#if header.flag}
				<Flag code={header.flag} label={`${countryName(country) || header.h1} flag`} class="size-8 vt-flag" />
			{/if}
			<Heading level={1}>
				<span id="guide-title" class="vt-title">{header.h1}</span>
			</Heading>
		</div>
		{#if header.lede}
			<p class="text-body-lg mt-3 mb-0 max-w-prose text-ink-muted">{header.lede}</p>
		{/if}
		{@render status?.()}
		<p class="mt-5 mb-0 text-sm text-ink-muted">
			Unofficial guide, not a government website{#if header.officialHost}
				<span aria-hidden="true">&nbsp;·</span>
				Official site:
				{#if header.officialUrl}
					<a
						href={header.officialUrl}
						target="_blank"
						rel="noopener noreferrer"
						class="font-medium text-accent-strong underline underline-offset-2"
						>{header.officialHost}<span aria-hidden="true">&nbsp;↗</span><span class="sr-only"> (opens in a new tab)</span></a
					>
				{:else}
					<span class="font-medium text-ink">{header.officialHost}</span>
				{/if}
			{/if}
		</p>
		{#if trust}
			<p class="mt-1 mb-0 font-mono text-xs text-ink-muted" data-print-hide>{trust}</p>
		{/if}
		{#if cited}
			<div class="mt-6">
				<CitationsToggle {count} />
			</div>
		{/if}
	</div>
</header>
