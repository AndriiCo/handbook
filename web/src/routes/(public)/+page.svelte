<script lang="ts">
	import { resolve } from '$app/paths';
	import { goto } from '$app/navigation';
	import Notice from '../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../lib/components/ui/skeleton/index.js';
	import Heading from '../../lib/components/brand/Heading.svelte';
	import Guilloche from '../../lib/components/brand/Guilloche.svelte';
	import Flag from '../../lib/components/brand/Flag.svelte';
	import HagueMap from '../../lib/components/brand/HagueMap.svelte';
	import ApostilleCertificate from '../../lib/components/brand/ApostilleCertificate.svelte';
	import silhouettes from '../../lib/geo/silhouettes.json';
	import {
		factCount,
		filterAuthorities,
		listAuthorities,
		publicError
	} from '../../lib/public-ui/handbook.js';
	import SearchIcon from '@lucide/svelte/icons/search';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';

	type Authority = {
		slug: string;
		name: string;
		country: string;
		fact_count: number;
	};

	let query = $state('');
	let index = $state<Authority[] | null>(null);
	let loadError = $state<string | null>(null);
	let qualityPct = $state<number | null>(null);
	let qualityModel = $state<string | null>(null);
	let comboOpen = $state(false);
	let activeDescendant = $state('');

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	// Golden-path authorities, resolved against the live index so names
	// stay fresh; static fallbacks keep links working before it loads.
	const FEATURED = [
		{ slug: 'vietnam', name: 'Viet Nam — receiving authorities', country: 'Viet Nam', flag: 'vn' },
		{
			slug: 'usa-wa-sos',
			name: 'Washington Secretary of State — Apostilles & Authentications',
			country: 'USA',
			flag: 'us'
		},
		{ slug: 'hcch', name: 'HCCH — Apostille Section status table', country: '', flag: 'nl' }
	];
	const featuredSlugs = new Set(FEATURED.map((f) => f.slug));
	const featured = $derived(
		FEATURED.map((f) => {
			const live = index?.find((a) => a.slug === f.slug);
			return { ...f, fact_count: live?.fact_count ?? 0 };
		})
	);

	// Last-verified per featured slug: newest citation capture date.
	let verifiedAt = $state<Record<string, string>>({});
	async function loadVerified(slugs: string[]) {
		for (const slug of slugs) {
			try {
				const res = await get(`/authorities/${slug}/facts`);
				const facts = Array.isArray(res.body?.facts) ? res.body.facts : [];
				let latest = '';
				for (const f of facts) {
					const cites = Array.isArray(f?.citations) ? f.citations : [];
					for (const c of cites) {
						if (typeof c?.captured_at === 'string' && c.captured_at > latest)
							latest = c.captured_at;
					}
				}
				if (latest) verifiedAt = { ...verifiedAt, [slug]: latest };
			} catch {
				// Silence: the count still renders without a date.
			}
		}
	}

	function relative(iso: string): string {
		const days = Math.floor((Date.now() - Date.parse(iso)) / 86400000);
		if (days <= 0) return 'today';
		if (days === 1) return 'yesterday';
		return `${days} days ago`;
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
				void loadVerified(FEATURED.map((f) => f.slug));
			})
			.catch(() => {
				loadError = publicError(500, 'index');
			});
		get('/quality')
			.then((res) => {
				const rate = res.body?.quote_found_rate;
				if (res.ok && typeof rate === 'number' && rate >= 0 && rate <= 1) {
					qualityPct = Math.round(rate * 100);
					if (typeof res.body?.model === 'string') qualityModel = res.body.model;
				}
			})
			.catch(() => {});
	});

	const blank = $derived(query.trim() === '');
	const results = $derived(
		index === null ? null : filterAuthorities(index, query).slice(0, 6)
	);

	// Index table: everything except the featured cards (never repeated),
	// with an All / Hague members / US states filter. Assumed: a slug
	// under `usa-` is a US state; every other authority is a Hague member.
	type Filter = 'all' | 'hague' | 'states';
	const FILTERS: { v: Filter; label: string }[] = [
		{ v: 'all', label: 'All' },
		{ v: 'hague', label: 'Hague members' },
		{ v: 'states', label: 'US states' }
	];
	let tableFilter = $state<Filter>('all');
	const isState = (slug: string) => slug.startsWith('usa-');
	const tableRows = $derived(
		(index ?? [])
			.filter((a) => !featuredSlugs.has(a.slug))
			.filter((a) => tableFilter === 'all' || (tableFilter === 'states') === isState(a.slug))
	);

	const memberIds = ['704', '840', '528'];
	const authorityHref = (slug: string) =>
		resolve('/(public)/authority/[slug]', { slug });

	function pick(slug: string) {
		comboOpen = false;
		query = '';
		void goto(authorityHref(slug));
	}
</script>

<svelte:head>
	<title>Apostille Handbook — Start with your destination</title>
	<meta
		name="description"
		content="Find apostille and authentication requirements by destination authority."
	/>
</svelte:head>

<section class="grid items-center gap-10 py-10 xl:grid-cols-2 xl:gap-8 xl:py-16" aria-labelledby="start">
	<div class="max-w-[68ch]">
		{#if qualityPct !== null}
			<p
				class="inline-flex min-h-11 items-center gap-2 rounded-full border border-line bg-surface px-4 text-sm text-ink"
			>
				<span class="relative flex size-2 flex-none" aria-hidden="true">
					<span
						class="absolute inline-flex size-full animate-ping rounded-full bg-verified opacity-60 motion-reduce:animate-none"
					></span>
					<span class="relative inline-flex size-2 rounded-full bg-verified"></span>
				</span>
				{qualityPct}% of drafted records carry a verified quote{#if qualityModel}
					<span class="font-mono text-xs text-ink-muted">{qualityModel}</span>
				{/if}
			</p>
		{/if}
		<Heading level={1}>
			<span class="[font-feature-settings:salt]">Apostille rules,</span>
			<em class="font-brand">quoted</em> from the source.
		</Heading>
		<p class="text-body-lg mt-4 text-ink-muted">
			Every requirement shows the official page it came from — quote, source and date, one
			tap away.
		</p>
		<div class="relative mt-6">
			<div
				class="flex h-14 items-center gap-2 rounded-lg border border-line bg-surface pr-2 pl-4 shadow-lg"
			>
				<SearchIcon class="size-5 flex-none text-ink-muted" aria-hidden="true" />
				<label class="sr-only" for="hero-q">Search the handbook</label>
				<input
					id="hero-q"
					name="q"
					type="text"
					role="combobox"
					aria-expanded={comboOpen}
					aria-controls="hero-results"
					aria-activedescendant={activeDescendant}
					aria-autocomplete="list"
					bind:value={query}
					onfocus={() => (comboOpen = true)}
					onblur={() => setTimeout(() => (comboOpen = false), 150)}
					onkeydown={(e) => {
						if (e.key === 'Escape') comboOpen = false;
					}}
					placeholder="Country, state, or authority…"
					autocomplete="off"
					class="h-full min-w-0 flex-1 bg-transparent text-base text-ink outline-none placeholder:text-ink-muted"
				/>
				<kbd
					class="hidden flex-none items-center gap-0.5 rounded-xs border border-line bg-bg px-1.5 font-mono text-xs text-ink-muted sm:inline-flex"
					aria-hidden="true"
				>
					<span>⌘</span><span>K</span>
				</kbd>
			</div>
			{#if comboOpen && !blank && results !== null}
				<ul
					id="hero-results"
					role="listbox"
					aria-label="Matching authorities"
					class="absolute inset-x-0 top-full z-30 mt-2 list-none overflow-hidden rounded-xl border border-line bg-bg p-1 shadow-xl"
				>
					{#each results as a (a.slug)}
						<li role="option" id={`hero-opt-${a.slug}`} aria-selected="false" class="m-0 p-0">
							<button
								type="button"
								onclick={() => pick(a.slug)}
								onmouseenter={() => (activeDescendant = `hero-opt-${a.slug}`)}
								class="flex min-h-11 w-full cursor-pointer items-center gap-2 rounded-lg px-3 text-left hover:bg-surface"
							>
								<span class="truncate font-medium text-ink">{a.name}</span>
								<span class="ml-auto flex-none font-mono text-xs text-ink-muted">
									{factCount(a.fact_count ?? 0)}
								</span>
							</button>
						</li>
					{:else}
						<li class="px-3 py-2 text-sm text-ink-muted">No authority matches “{query}”.</li>
					{/each}
				</ul>
			{/if}
		</div>
		<ul class="mt-4 flex list-none flex-wrap gap-2 p-0" aria-label="Featured authorities">
			{#each featured as authority, i (authority.slug)}
				<li class="hero-pin m-0 p-0" style={`animation-delay: ${i * 40}ms`}>
					<a
						class="inline-flex min-h-11 items-center gap-2 rounded-full border border-line bg-bg px-3 text-sm font-medium text-ink no-underline transition-colors hover:border-focus"
						href={authorityHref(authority.slug)}
					>
						<Flag code={authority.flag} label={`${authority.country} flag`} />
						{authority.country || authority.name}
					</a>
				</li>
			{/each}
		</ul>
	</div>
	<div class="relative overflow-clip" aria-hidden="false">
		<Guilloche
			seed="hague-hero"
			class="absolute -top-10 -right-10 size-56 text-brand-300 opacity-70 [mask-image:radial-gradient(circle,black,transparent_70%)] dark:text-brand-700"
		/>
		<HagueMap members={memberIds} class="relative w-full" />
	</div>
</section>

<section class="py-10 xl:py-16" aria-labelledby="how">
	<Heading level={2} eyebrow="How it works">Three checks before anything is published</Heading>
	<ul class="mt-8 grid list-none gap-4 p-0 md:grid-cols-3">
		{#each [{ n: 1, model: 'Muse Spark', title: 'Spark drafts from official pages', text: 'Every fact starts as a quote copied from a watched government page — never from memory.', field: 1 }, { n: 9, model: 'Quote check', title: 'The quote is checked byte-exact', text: 'A refused or missing quote goes back to the model once; unverifiable drafts never publish.', field: 9 }, { n: 10, model: 'Human review', title: 'Staff approve, the dossier updates live', text: 'Only accepted facts reach the public dossier, seconds after approval.', field: 10 }] as step (step.title)}
			<li
				class="flex flex-col gap-3 rounded-xl border border-line bg-bg p-4 transition-shadow duration-200 hover:shadow-md"
			>
				<ApostilleCertificate highlight={step.field} />
				<p class="m-0">
					<span
						class="inline-flex min-h-11 items-center rounded-full border border-line bg-surface px-3 font-mono text-xs text-ink-muted"
					>
						{step.model}
					</span>
				</p>
				<h3 class="text-h3 m-0 font-semibold">{step.title}</h3>
				<p class="m-0 text-sm text-ink-muted">{step.text}</p>
			</li>
		{/each}
	</ul>
</section>

<section class="py-10 xl:py-16" aria-labelledby="featured">
	<Heading level={2} eyebrow="Featured">Start with a destination</Heading>
	{#if loadError}
		<Notice variant="error" heading="Featured destinations are unavailable."><p>{loadError}</p></Notice>
	{:else}
		<div class="mt-8 grid gap-4 md:grid-cols-2">
			{#each featured as authority, i (authority.slug)}
				{@const sil =
					authority.slug === 'vietnam'
						? silhouettes.vietnam
						: authority.slug === 'usa-wa-sos'
							? silhouettes.washington
							: silhouettes.netherlands}
				<a
					href={authorityHref(authority.slug)}
					class={`group relative overflow-hidden rounded-xl border border-line bg-bg p-5 no-underline transition-all duration-200 hover:-translate-y-0.5 hover:shadow-lg ${i === 0 ? 'md:row-span-2' : ''}`}
				>
					<svg
						viewBox={`0 0 ${sil.width} ${sil.height}`}
						class="pointer-events-none absolute -right-6 -bottom-6 w-44 opacity-[0.08] [mask-image:radial-gradient(circle_at_bottom_right,black,transparent_75%)]"
						aria-hidden="true"
						data-graphic="silhouette"
					>
						<path d={sil.d} style="fill: var(--color-ink);" />
					</svg>
					<span class="relative flex items-center gap-2">
						<Flag code={authority.flag} label={`${authority.country} flag`} />
						{#if verifiedAt[authority.slug]}
							<span
								class="ml-auto font-mono text-xs text-ink-muted"
								title={`Last verified ${verifiedAt[authority.slug]}`}
							>
								verified {relative(verifiedAt[authority.slug])}
							</span>
						{/if}
					</span>
					{#if i === 0}
						<span class="font-brand text-h2 mt-3 block font-medium text-balance text-ink">
							{authority.name}
						</span>
					{:else}
						<span class="text-h3 mt-3 block font-semibold text-ink">{authority.name}</span>
					{/if}
					<span class="mt-2 block font-mono text-xs text-ink-muted">
						{factCount(authority.fact_count ?? 0)}
					</span>
					<span
						class="mt-4 inline-flex min-h-11 items-center gap-1 text-sm font-medium text-accent-strong"
					>
						Open the dossier
						<ArrowRightIcon class="size-4" aria-hidden="true" />
					</span>
				</a>
			{/each}
		</div>
	{/if}
</section>

<section class="py-10 xl:py-16" aria-labelledby="index">
	<Heading level={2} eyebrow="Index">All authorities</Heading>
	<div class="mt-6 flex flex-wrap gap-2" role="group" aria-label="Filter authorities">
		{#each FILTERS as opt (opt.v)}
			<button
				type="button"
				aria-pressed={tableFilter === opt.v}
				onclick={() => (tableFilter = opt.v)}
				class={`inline-flex min-h-11 cursor-pointer items-center rounded-full border px-4 text-sm font-medium transition-colors ${tableFilter === opt.v ? 'border-accent-fill bg-accent-fill text-accent-ink' : 'border-line bg-bg text-ink hover:border-focus'}`}
			>
				{opt.label}
			</button>
		{/each}
	</div>
	{#if loadError}
		<Notice variant="error" heading="Search is unavailable."><p>{loadError}</p></Notice>
	{:else if index === null}
		<div role="status" aria-label="Loading the authority index" class="mt-4 flex flex-col gap-2">
			<span class="sr-only">Loading the authority index</span>
			<Skeleton class="h-11 w-full" />
			<Skeleton class="h-11 w-full" />
			<Skeleton class="h-11 w-2/3" />
		</div>
	{:else if tableRows.length === 0}
		<div class="mt-4 rounded-xl border border-line p-6">
			<p class="m-0 font-medium text-ink">
				{#if tableFilter === 'all'}
					Every authority is featured above.
				{:else}
					No authorities in this group yet.
				{/if}
			</p>
			<p class="mt-1 mb-0 text-sm text-ink-muted">
				{#if tableFilter === 'all'}
					The full list grows here as new destinations are verified.
				{:else}
					Try the “All” filter to see every destination.
				{/if}
			</p>
		</div>
	{:else}
		<table class="mt-4 w-full border-collapse text-left">
			<caption class="sr-only">Authorities{blank ? '' : ` matching “${query}”`}</caption>
			<thead>
				<tr class="border-b border-line text-sm text-ink-muted">
					<th scope="col" class="py-2 pr-4 font-medium">Authority</th>
					<th scope="col" class="hidden py-2 pr-4 font-medium sm:table-cell">Country</th>
					<th scope="col" class="py-2 pr-4 text-right font-medium">Facts</th>
					<th scope="col" class="hidden py-2 text-right font-medium md:table-cell">Status</th>
				</tr>
			</thead>
			<tbody>
				{#each (blank ? tableRows : filterAuthorities(tableRows, query)) as authority (authority.slug)}
					<tr class="border-b border-line">
						<th scope="row" class="py-2 pr-4 font-medium">
							<a class="inline-flex min-h-11 items-center" href={authorityHref(authority.slug)}>
								{authority.name}
							</a>
						</th>
						<td class="hidden py-2 pr-4 text-sm text-ink-muted sm:table-cell">
							{authority.country}
						</td>
						<td class="py-2 pr-4 text-right font-mono text-sm">{authority.fact_count ?? 0}</td>
						<td class="hidden py-2 text-right text-sm text-ink-muted md:table-cell">
							{isState(authority.slug) ? 'US state' : 'Hague member'}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</section>

<style>
	@media (prefers-reduced-motion: no-preference) {
		.hero-pin {
			animation: hero-rise 300ms var(--ease-out-quint) both;
		}
		@keyframes hero-rise {
			from {
				opacity: 0;
				transform: translateY(6px);
			}
			to {
				opacity: 1;
				transform: none;
			}
		}
	}
</style>
