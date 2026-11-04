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
	import {
		addressOf,
		flagFor,
		kindLabel,
		laneOf,
		LANES,
		LANE_LABEL,
		latestCapture,
		silhouetteFor,
		verifiedAtOf,
		type AuthorityRow,
		type Lane
	} from '../../lib/public-ui/authority-meta.js';
	import {
		clampName,
		factCount,
		filterAuthorities,
		groupByCountry,
		listAuthorities,
		publicError,
		relativeDay
	} from '../../lib/public-ui/handbook.js';
	import { placePath } from '../../lib/public-ui/location.js';
	import SearchIcon from '@lucide/svelte/icons/search';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';

	type Authority = AuthorityRow & { name: string; country: string; fact_count: number };

	let query = $state('');
	let index = $state<Authority[] | null>(null);
	let loadError = $state<string | null>(null);
	let qualityPct = $state<number | null>(null);
	let qualityModel = $state<string | null>(null);
	let qualityRunAt = $state<string | null>(null);
	let comboOpen = $state(false);
	let activeDescendant = $state('');

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	// Golden-path routing ids only (GP1 keeps >= 3 dossier links even
	// while the index loads). Names, countries, flags and counts come
	// from the live index; nothing about them is written here.
	const FEATURED = ['vietnam', 'usa-wa-sos', 'hcch'];
	// Routing id of the HCCH status-table authority: the map's Hague
	// membership is read from its accepted facts, never listed here.
	const HCCH = 'hcch';
	const featuredSlugs = new Set(FEATURED);
	const featured = $derived(
		FEATURED.map((slug) => {
			const row = index?.find((a) => a.slug === slug) ?? null;
			return {
				slug,
				row,
				name: row?.name || slug,
				label: row?.region || row?.country || row?.name || slug,
				country: row?.country ?? '',
				flag: flagFor(row?.country, row?.iso2),
				sil: silhouetteFor(row),
				fact_count: row?.fact_count ?? 0
			};
		})
	);

	// Accepted facts per featured slug: last-verified dates for the
	// cards, and the HCCH facts that tint the member map.
	let factsBySlug = $state<Record<string, any[]>>({});
	const verifiedAt = $derived(
		Object.fromEntries(
			featured.map((f) => [f.slug, verifiedAtOf(f.row) || latestCapture(factsBySlug[f.slug])])
		) as Record<string, string>
	);
	async function loadFeaturedFacts(slugs: string[]) {
		await Promise.all(
			slugs.map(async (slug) => {
				try {
					const res = await get(`/authorities/${slug}/facts`);
					if (!res.ok) return;
					const facts = Array.isArray(res.body?.facts) ? res.body.facts : [];
					factsBySlug = { ...factsBySlug, [slug]: facts };
				} catch {
					// Silence: the card still renders without a date.
				}
			})
		);
	}
	const mapPins = $derived(
		(index ?? []).map((a) => ({ slug: a.slug, label: a.region || a.country || a.name }))
	);

	function shortDate(iso: string): string {
		return String(iso ?? '').split('T')[0] || '';
	}

	$effect(() => {
		void loadFeaturedFacts(FEATURED);
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
		get('/quality')
			.then((res) => {
				const rate = res.body?.quote_found_rate;
				if (res.ok && typeof rate === 'number' && rate >= 0 && rate <= 1) {
					qualityPct = Math.round(rate * 100);
					if (typeof res.body?.model === 'string') qualityModel = res.body.model;
					if (typeof res.body?.run_at === 'string') qualityRunAt = res.body.run_at;
				}
			})
			.catch(() => {});
	});

	const blank = $derived(query.trim() === '');
	const results = $derived(
		index === null ? null : filterAuthorities(index, query).slice(0, 6)
	);

	// Index table: everything except the featured cards (never repeated),
	// filtered by `authority.kind` lanes. Rows without a kind sit in the
	// neutral lane; with no kinds at all there is no filter to offer.
	type Filter = 'all' | Lane;
	let tableFilter = $state<Filter>('all');
	const others = $derived((index ?? []).filter((a) => !featuredSlugs.has(a.slug)));
	const FILTERS = $derived.by(() => {
		const lanes = new Set(others.map((a) => laneOf(a)));
		const out: { v: Filter; label: string }[] = [{ v: 'all', label: 'All' }];
		for (const lane of LANES) {
			if (lanes.has(lane)) out.push({ v: lane, label: LANE_LABEL[lane] });
		}
		return out.length > 2 ? out : out.slice(0, 1);
	});
	const showStatus = $derived(others.some((a) => kindLabel(a) !== ''));
	const tableRows = $derived(
		others.filter((a) => tableFilter === 'all' || laneOf(a) === tableFilter)
	);
	// World scale (~700 rows): blank query → one collapsed row per
	// country (children render on first open); a query → a flat list
	// of the first MATCH_CAP matches.
	const MATCH_CAP = 50;
	const groups = $derived(groupByCountry(tableRows));
	const matches = $derived(blank ? [] : filterAuthorities(tableRows, query));
	let opened = $state<Record<string, boolean>>({});

	const authorityHref = (slug: string) =>
		resolve('/(public)/authority/[...trail]', { trail: slug });

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
				Eval: {qualityPct}% of Muse Spark drafts quote the source verbatim{#if qualityRunAt}
					· {shortDate(qualityRunAt)}{/if}{#if qualityModel}
					<span class="font-mono text-xs text-ink-muted">{qualityModel}</span>
				{/if}
			</p>
		{/if}
		<Heading level={1}>
			<span class="[font-feature-settings:salt]">Apostille rules,</span>
			<em class="font-brand">quoted</em> from the source.
		</Heading>
		<p class="text-body-lg mt-4 text-ink-muted">
			Every requirement comes from the office's own pages. On each guide, citations are off by
			default: turn them on with Alt+C or the Citations button to see the quote, source and date.
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
								onclick={() => pick(addressOf(a))}
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
						{#if index === null}
							<Skeleton class="h-4 w-20" />
							<span class="sr-only">Featured dossier {i + 1}</span>
						{:else}
							{#if authority.flag}
								<Flag code={authority.flag} label={`${authority.country} flag`} />
							{/if}
							{authority.label}
						{/if}
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
		<HagueMap facts={factsBySlug[HCCH] ?? []} pins={mapPins} class="relative w-full" />
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
				{@const sil = authority.sil}
				<a
					href={authorityHref(authority.slug)}
					class={`group relative overflow-hidden rounded-xl border border-line bg-bg p-5 no-underline transition-all duration-200 hover:-translate-y-0.5 hover:shadow-lg ${i === 0 ? 'md:row-span-2' : ''}`}
				>
					{#if sil}
						<svg
							viewBox={`0 0 ${sil.width} ${sil.height}`}
							class="pointer-events-none absolute -right-6 -bottom-6 w-44 opacity-[0.08] [mask-image:radial-gradient(circle_at_bottom_right,black,transparent_75%)]"
							aria-hidden="true"
							data-graphic="silhouette"
						>
							<path d={sil.d} style="fill: var(--color-ink);" />
						</svg>
					{/if}
					<span class="relative flex min-h-5 items-center gap-2">
						{#if authority.flag}
							<Flag code={authority.flag} label={`${authority.country} flag`} class={i === 0 ? 'size-5 vt-flag' : 'size-5'} />
						{/if}
						{#if verifiedAt[authority.slug]}
							<span
								class="ml-auto font-mono text-xs text-ink-muted"
								title={`Last verified ${verifiedAt[authority.slug]}`}
							>
								verified {relativeDay(verifiedAt[authority.slug])}
							</span>
						{/if}
					</span>
					{#if index === null}
						<Skeleton class="mt-3 h-7 w-3/4" />
						<Skeleton class="mt-2 h-4 w-16" />
						<span class="sr-only">Featured dossier {i + 1}</span>
					{:else}
						{#if i === 0}
							<span class="font-brand text-h2 vt-title mt-3 block font-medium text-balance text-ink">
								{authority.name}
							</span>
						{:else}
							<span class="text-h3 mt-3 block font-semibold text-ink">{authority.name}</span>
						{/if}
						<span class="mt-2 block font-mono text-xs text-ink-muted">
							{factCount(authority.fact_count ?? 0)}
						</span>
					{/if}
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
	{#if FILTERS.length > 1}
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
	{/if}
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
					No other authorities yet.
				{:else}
					No authorities in this group yet.
				{/if}
			</p>
			<p class="mt-1 mb-0 text-sm text-ink-muted">
				{#if tableFilter === 'all'}
					New destinations appear here as staff verify them.
				{:else}
					Try the “All” filter to see every destination.
				{/if}
			</p>
			{#if tableFilter === 'all'}
				<p class="mt-3 mb-0 text-sm">
					Start with a featured dossier:{#each featured as f, i (f.slug)}
						{#if i > 0}<span aria-hidden="true"> · </span>{/if}<a
							class="underline underline-offset-4"
							href={authorityHref(f.slug)}>{f.label}</a
						>{/each}
				</p>
			{/if}
		</div>
	{:else if !blank}
		<p class="mt-4 mb-2 text-sm text-ink-muted" role="status">
			{matches.length === 0
				? `No authority matches “${query}”.`
				: matches.length > MATCH_CAP
					? `Showing ${MATCH_CAP} of ${matches.length} matches — refine the search.`
					: `${matches.length} ${matches.length === 1 ? 'match' : 'matches'}`}
		</p>
		<ul class="m-0 list-none p-0" aria-label={`Authorities matching “${query}”`}>
			{#each matches.slice(0, MATCH_CAP) as authority (authority.slug)}
				<li class="m-0 border-b border-line p-0">
					<a
						class="flex min-h-11 items-center gap-3 py-2 no-underline"
						href={authorityHref(addressOf(authority))}
						title={authority.name}
					>
						<span class="min-w-0 flex-1 font-medium break-words text-ink">
							{clampName(authority.name)}
						</span>
						<span class="hidden flex-none text-sm text-ink-muted sm:inline">
							{authority.country}
						</span>
						<span class="flex-none font-mono text-xs text-ink-muted">
							{factCount(authority.fact_count ?? 0)}
						</span>
					</a>
				</li>
			{/each}
		</ul>
	{:else}
		<p class="mt-4 mb-2 text-sm text-ink-muted">
			{tableRows.length} authorities in {groups.length}
			{groups.length === 1 ? 'country' : 'countries'}. Open a country to see its authorities.
		</p>
		<ul class="m-0 list-none p-0" aria-label="Authorities by country">
			{#each groups as group (group.country)}
				{@const flag = flagFor(group.country, group.iso2)}
				<li class="m-0 border-b border-line p-0">
					<details
						class="group"
						ontoggle={(e) => {
							if ((e.currentTarget as HTMLDetailsElement).open) {
								opened = { ...opened, [group.country]: true };
							}
						}}
					>
						<summary
							class="country-summary flex min-h-11 cursor-pointer list-none items-center gap-3 py-2"
						>
							<ChevronRightIcon
								class="size-4 flex-none text-ink-muted transition-transform group-open:rotate-90 motion-reduce:transition-none"
								aria-hidden="true"
							/>
							{#if flag}
								<Flag code={flag} label={`${group.country} flag`} />
							{/if}
							<span class="min-w-0 flex-1 font-medium break-words text-ink">{group.country}</span>
							<span class="flex-none font-mono text-xs text-ink-muted">
								{group.rows.length}
								{group.rows.length === 1 ? 'authority' : 'authorities'}{#if group.facts > 0}
									· {factCount(group.facts)}{/if}
							</span>
						</summary>
						{#if opened[group.country]}
							{@const place = placePath(group.iso2, group.country)}
							{#if place}
								<a
									class="ml-2 inline-flex min-h-11 items-center text-sm sm:ml-8"
									href={resolve('/(public)/in/[...path]', { path: place })}
								>
									Which office handles my document? All offices in {group.country}
								</a>
							{/if}
							<ul class="m-0 mb-3 list-none p-0 pl-2 sm:pl-8">
								{#each group.rows as authority (authority.slug)}
									<li class="m-0 p-0">
										<a
											class="flex min-h-11 items-center gap-3 py-1 no-underline"
											href={authorityHref(addressOf(authority))}
											title={authority.name}
										>
											<span class="min-w-0 flex-1 text-sm break-words text-ink">
												{clampName(authority.name)}
											</span>
											{#if showStatus && kindLabel(authority) && authority.kind !== 'country'}
												<span class="hidden flex-none text-xs text-ink-muted md:inline">
													{kindLabel(authority)}
												</span>
											{/if}
											<span class="flex-none font-mono text-xs text-ink-muted">
												{factCount(authority.fact_count ?? 0)}
											</span>
										</a>
									</li>
								{/each}
							</ul>
						{/if}
					</details>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	/* Safari keeps its disclosure triangle without this (list-none
	   covers the others); the chevron is the marker. */
	.country-summary::-webkit-details-marker {
		display: none;
	}
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
