<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../lib/components/ui/skeleton/index.js';
	import {
		factCount,
		filterAuthorities,
		listAuthorities,
		publicError
	} from '../../lib/public-ui/handbook.js';

	// Slice A: live search over GET /api/v1/authorities, filtered
	// client-side as you type. Same-origin, no auth. EN-only until
	// the Week 3+ i18n pass.
	let query = $state('');
	let index = $state<
		{ slug: string; name: string; country: string; fact_count: number }[] | null
	>(null);
	let loadError = $state<string | null>(null);

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	$effect(() => {
		listAuthorities({ get })
			.then((res) => {
				if (!res.ok) {
					loadError = publicError(res.status, 'index');
					return;
				}
				const rows = res.body?.authorities;
				index = Array.isArray(rows) ? rows : [];
			})
			.catch(() => {
				loadError = publicError(500, 'index');
			});
	});

	const blank = $derived(query.trim() === '');
	const results = $derived(index === null ? null : filterAuthorities(index, query));

	// Featured destinations: the golden-path authorities, resolved
	// against the live index so names stay fresh; static fallback
	// names keep the shortcuts working before the index loads.
	const FEATURED = [
		{ slug: 'vietnam', name: 'Viet Nam — receiving authorities', country: 'Viet Nam' },
		{
			slug: 'usa-wa-sos',
			name: 'Washington Secretary of State — Apostilles & Authentications',
			country: 'USA'
		},
		{ slug: 'hcch', name: 'HCCH — Apostille Section status table', country: '' }
	];
	const featured = $derived(
		FEATURED.map((f) => index?.find((a) => a.slug === f.slug) ?? { ...f, fact_count: 0 })
	);
	const authorityHref = (slug: string) =>
		resolve('/(public)/authority/[slug]', { slug });
</script>

<svelte:head>
	<title>Apostille Handbook — Start with your destination</title>
	<meta
		name="description"
		content="Find apostille and authentication requirements by destination authority."
	/>
</svelte:head>

<section class="py-6" aria-labelledby="start">
	<h1 id="start" class="m-0 mb-4 font-sans text-3xl md:text-4xl">
		Start with your destination
	</h1>
	<form role="search" class="flex flex-col gap-2 md:flex-row" onsubmit={(e) => e.preventDefault()}>
		<label class="sr-only" for="q">Search the handbook</label>
		<input
			id="q"
			name="q"
			type="search"
			bind:value={query}
			placeholder="Country, state, or authority…"
			autocomplete="off"
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink md:flex-1"
		/>
		<button
			type="submit"
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink"
		>
			Search
		</button>
	</form>
</section>

<section class="pb-2" aria-labelledby="featured">
	<h2
		id="featured"
		class="border-b border-line pb-[0.4rem] font-sans text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase"
	>
		Featured destinations
	</h2>
	<ul class="m-0 mt-4 list-none p-0" aria-label="Featured destinations">
		{#each featured as authority (authority.slug)}
			<li
				class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]"
			>
				<a
					class="inline-flex min-h-11 items-center font-bold"
					href={authorityHref(authority.slug)}>{authority.name}</a
				>
				<span class="mr-auto text-sm text-ink-muted"
					>{authority.country ? `${authority.country} · ` : ''}{factCount(
						authority.fact_count ?? 0
					)}</span
				>
			</li>
		{/each}
	</ul>
</section>

<section class="pb-8" aria-labelledby="index">
	<h2
		id="index"
		class="border-b border-line pb-[0.4rem] font-sans text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase"
	>
		Authority index
	</h2>
	{#if loadError}
		<Notice variant="error" heading="Search is unavailable."><p>{loadError}</p></Notice>
	{:else if results === null}
		<div role="status" aria-label="Loading the authority index" class="mt-4 flex flex-col gap-2">
			<span class="sr-only">Loading the authority index</span>
			<Skeleton class="h-11 w-full" />
			<Skeleton class="h-11 w-full" />
			<Skeleton class="h-11 w-2/3" />
		</div>
	{:else if results.length === 0 && blank}
		<Notice variant="info" heading="The handbook has no published authorities yet.">
			<p>Check back after the first handbook build.</p>
		</Notice>
	{:else if results.length === 0}
		<Notice variant="info" heading="No authority matches “{query}”.">
			<p>Try a country name or a shorter term.</p>
		</Notice>
	{:else}
		<ul class="m-0 mt-4 list-none p-0" aria-label={blank ? 'Authority index' : 'Search results'}>
			{#each results as authority (authority.slug)}
				<li
					class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.7rem]"
				>
					<a
						class="inline-flex min-h-11 items-center font-bold"
						href={resolve('/(public)/authority/[slug]', { slug: authority.slug })}
						>{authority.name}</a
					>
					<span class="mr-auto text-sm text-ink-muted"
						>{authority.country} · {factCount(authority.fact_count ?? 0)}</span
					>
				</li>
			{/each}
		</ul>
	{/if}
</section>
