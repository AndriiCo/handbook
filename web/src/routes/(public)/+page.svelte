<script lang="ts">
	import { resolve } from '$app/paths';
	import { goto } from '$app/navigation';
	import Notice from '../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../lib/components/ui/skeleton/index.js';
	import Heading from '../../lib/components/brand/Heading.svelte';
	import Flag from '../../lib/components/brand/Flag.svelte';
	import HagueMap from '../../lib/components/brand/HagueMap.svelte';
	import { WORLD_ALPHA2 } from '../../lib/components/brand/HagueMap.members.js';
	import CountryPicker from '../../lib/public-ui/CountryPicker.svelte';
	import {
		addressOf,
		flagFor,
		kindLabel,
		laneOf,
		LANES,
		type AuthorityRow,
		type Lane
	} from '../../lib/public-ui/authority-meta.js';
	import {
		clampName,
		factCount,
		filterAuthorities,
		groupByCountry,
		listAuthorities,
		publicError
	} from '../../lib/public-ui/handbook.js';
	import { DOC_CLASSES, placePath } from '../../lib/public-ui/location.js';
	import {
		countryOptions,
		guessHomeCountry,
		HCCH_SLUG,
		placeOfCode,
		routeHref
	} from '../../lib/public-ui/route.js';
	import { HOME_COPY, INTL_TAG } from '../../lib/public-ui/home-copy.js';
	import { PLACE_COPY } from '../../lib/public-ui/place-copy.js';
	import { site } from '../../lib/locale.svelte.js';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import ArrowLeftRightIcon from '@lucide/svelte/icons/arrow-left-right';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';

	type Authority = AuthorityRow & { name: string; country: string; fact_count: number };

	const c = $derived(HOME_COPY[site.locale]);
	const pc = $derived(PLACE_COPY[site.locale]);
	// A row's status word: its lane in the site language, else the raw
	// kind as the API sends it (kindLabel).
	function kindText(a: Authority): string {
		return (c.kinds as Record<string, string>)[laneOf(a)] ?? kindLabel(a);
	}
	// `groupByCountry` names rows without a country `International`.
	const countryText = (country: string) =>
		country === 'International' ? c.international : country;

	let index = $state<Authority[] | null>(null);
	// The failed status, not its sentence, so a language switch re-words it.
	let loadStatus = $state<number | null>(null);
	const loadError = $derived(
		loadStatus === null ? null : publicError(loadStatus, 'index', site.locale)
	);
	let hcchFacts = $state<unknown[]>([]);

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
					loadStatus = res.status;
					return;
				}
				const rows = res.body?.authorities;
				index = Array.isArray(rows) ? rows : [];
			})
			.catch(() => {
				loadStatus = 500;
			});
		// The map's Hague membership is read from the status table's
		// accepted facts, never listed here.
		get(`/authorities/${HCCH_SLUG}/facts`)
			.then((res) => {
				if (res.ok && Array.isArray(res.body?.facts)) hcchFacts = res.body.facts;
			})
			.catch(() => {});
	});

	// ------------------------------------------------- guided question

	// Country names in the site language (CLDR), so no list is written here.
	const regionNames = $derived.by(() => {
		try {
			return new Intl.DisplayNames([INTL_TAG[site.locale]], { type: 'region' });
		} catch {
			return null;
		}
	});
	const countries = $derived(
		countryOptions(WORLD_ALPHA2, index, (code) => regionNames?.of(code), INTL_TAG[site.locale])
	);

	// The likely "used in" country from the time zone, shown and editable.
	const guessed = (() => {
		try {
			return guessHomeCountry({ timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone });
		} catch {
			return '';
		}
	})();
	let from = $state('');
	let to = $state(guessed);
	let doc = $state('');
	let tried = $state(false);
	const showGuess = $derived(guessed !== '' && to === guessed && from === '');

	const docChips = $derived(
		DOC_CLASSES.map((d) => ({ id: d.id, label: pc.docClasses[d.id] ?? d.label }))
	);
	const routeBase = resolve('/(public)/route');

	function swap() {
		[from, to] = [to, from];
	}

	function submit(e: SubmitEvent) {
		e.preventDefault();
		tried = true;
		if (!from || !to) return;
		void goto(routeHref(routeBase, { from: placeOfCode(from), to, doc }));
	}

	// One-tap common requests (personas J1–J9): routing ids only, the
	// words come from the copy table.
	const JOURNEYS = [
		{ key: 'usMxBirth', from: 'usa', to: 'mx', doc: 'vital_record' },
		{ key: 'mxUsBirth', from: 'mx', to: 'us', doc: 'vital_record' },
		{ key: 'usVnDiploma', from: 'usa', to: 'vn', doc: 'education' },
		{ key: 'usEgFbi', from: 'usa', to: 'eg', doc: 'fbi_check' },
		{ key: 'usUaPoa', from: 'usa', to: 'ua', doc: 'notarized_private' },
		{ key: 'usEsMarriage', from: 'usa', to: 'es', doc: 'vital_record' }
	];
	// Worked examples beside the explainer: routing ids only (they also
	// keep GP1's three dossier links on the page while the index loads).
	const EXAMPLES = [
		{ key: 'wa', slug: 'usa-wa-sos' },
		{ key: 'vietnam', slug: 'vietnam' },
		{ key: 'egypt', slug: 'egypt' },
		{ key: 'hcch', slug: HCCH_SLUG }
	];

	const mapPins = $derived(
		(index ?? []).map((a) => ({ slug: a.slug, label: a.region || a.country || a.name }))
	);

	// ------------------------------------------------- full index

	let query = $state('');
	const blank = $derived(query.trim() === '');
	// Filtered by `authority.kind` lanes. Rows without a kind sit in the
	// neutral lane; with no kinds at all there is no filter to offer.
	type Filter = 'all' | Lane;
	let tableFilter = $state<Filter>('all');
	const rows = $derived(index ?? []);
	const FILTERS = $derived.by(() => {
		const lanes = new Set(rows.map((a) => laneOf(a)));
		const out: { v: Filter; label: string }[] = [{ v: 'all', label: c.all }];
		for (const lane of LANES) {
			if (lanes.has(lane)) out.push({ v: lane, label: c.lanes[lane] });
		}
		return out.length > 2 ? out : out.slice(0, 1);
	});
	const showStatus = $derived(rows.some((a) => kindLabel(a) !== ''));
	const tableRows = $derived(
		rows.filter((a) => tableFilter === 'all' || laneOf(a) === tableFilter)
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
</script>

<svelte:head>
	<title>{c.headTitle}</title>
	<meta name="description" content={c.metaDescription} />
</svelte:head>

<section class="py-10 xl:py-16" aria-labelledby="start">
	<div class="max-w-[68ch]">
		<Heading level={1}><span id="start">{c.heroTitle}</span></Heading>
		<p class="text-body-lg mt-4 text-ink-muted">{c.intro}</p>
	</div>

	<form
		class="mt-8 rounded-xl border border-line bg-bg p-4 shadow-lg sm:p-6"
		onsubmit={submit}
		novalidate
	>
		<div class="grid items-end gap-3 md:grid-cols-[1fr_auto_1fr]">
			<CountryPicker
				id="hero-q"
				label={c.fromLabel}
				placeholder={c.countryPlaceholder}
				options={countries}
				bind:value={from}
				noMatch={c.noCountry}
				listLabel={c.countriesLabel(c.fromLabel)}
				clearLabel={c.clear(c.fromLabel)}
			/>
			<button
				type="button"
				onclick={swap}
				aria-label={c.swap}
				title={c.swap}
				class="inline-flex size-11 cursor-pointer items-center justify-center justify-self-center rounded-full border border-line bg-surface text-ink-muted transition-colors hover:border-focus hover:text-ink md:mb-1.5"
			>
				<ArrowLeftRightIcon class="size-4 rotate-90 md:rotate-0" aria-hidden="true" />
			</button>
			<CountryPicker
				id="hero-to"
				label={c.toLabel}
				placeholder={c.countryPlaceholder}
				options={countries}
				bind:value={to}
				noMatch={c.noCountry}
				listLabel={c.countriesLabel(c.toLabel)}
				clearLabel={c.clear(c.toLabel)}
			/>
		</div>
		{#if showGuess}
			<p class="mt-2 mb-0 text-xs text-ink-muted md:text-right">{c.guessed}</p>
		{/if}

		<fieldset class="mt-6 border-0 p-0">
			<legend class="mb-2 p-0 text-sm font-medium text-ink">{c.docLabel}</legend>
			<div class="flex flex-wrap gap-2">
				{#each docChips as d (d.id)}
					<button
						type="button"
						aria-pressed={doc === d.id}
						onclick={() => (doc = doc === d.id ? '' : d.id)}
						class={`inline-flex min-h-11 cursor-pointer items-center rounded-full border px-4 text-sm transition-colors ${
							doc === d.id
								? 'border-accent-fill bg-accent-fill font-medium text-accent-ink'
								: 'border-line bg-surface text-ink hover:border-focus'
						}`}
					>
						{d.label}
					</button>
				{/each}
			</div>
		</fieldset>

		<div class="mt-6 flex flex-wrap items-center gap-4">
			<button
				type="submit"
				class="inline-flex min-h-12 cursor-pointer items-center gap-2 rounded-lg bg-accent-fill px-6 text-base font-semibold text-accent-ink transition-opacity hover:opacity-90"
			>
				{c.submit}
				<ArrowRightIcon class="size-4" aria-hidden="true" />
			</button>
			{#if tried && (!from || !to)}
				<p class="m-0 text-sm text-ink" role="status">{c.needBoth}</p>
			{/if}
		</div>
	</form>

	<ul class="mt-6 grid list-none gap-4 p-0 md:grid-cols-2" aria-label={c.pathsLabel}>
		<li class="m-0 flex flex-col gap-2 rounded-xl border border-line bg-surface p-5">
			<h2 class="text-h3 m-0 font-semibold">{c.checkTitle}</h2>
			<p class="m-0 text-sm text-ink-muted">{c.checkText}</p>
			<button
				type="button"
				onclick={() => document.getElementById('hero-q')?.focus()}
				class="mt-auto inline-flex min-h-11 cursor-pointer items-center gap-1 self-start text-sm font-medium text-accent-strong"
			>
				{c.checkCta}
				<ArrowRightIcon class="size-4" aria-hidden="true" />
			</button>
		</li>
		<li class="m-0 flex flex-col gap-2 rounded-xl border border-line bg-surface p-5">
			<h2 class="text-h3 m-0 font-semibold">{c.doneTitle}</h2>
			<p class="m-0 text-sm text-ink-muted">{c.doneText}</p>
			<a
				href={resolve('/(customer)/order')}
				class="mt-auto inline-flex min-h-11 items-center gap-1 self-start text-sm font-medium text-accent-strong no-underline"
			>
				{c.doneCta}
				<ArrowRightIcon class="size-4" aria-hidden="true" />
			</a>
		</li>
	</ul>
</section>

<section class="py-10 xl:py-16" aria-labelledby="journeys">
	<Heading level={2}><span id="journeys">{c.journeysTitle}</span></Heading>
	<ul class="mt-6 grid list-none gap-x-6 p-0 md:grid-cols-2 xl:grid-cols-3">
		{#each JOURNEYS as j (j.key)}
			{@const words = c.journeys[j.key]}
			<li class="m-0 border-b border-line p-0">
				<a
					href={routeHref(routeBase, j)}
					class="group flex min-h-16 items-center gap-3 py-3 no-underline"
				>
					<span class="flex flex-none -space-x-1" aria-hidden="true">
						{#each [j.from, j.to] as code (code)}
							{@const f = flagFor('', code === 'usa' ? 'us' : code)}
							{#if f}<Flag code={f} label="" class="size-6 ring-2 ring-bg" />{/if}
						{/each}
					</span>
					<span class="min-w-0 flex-1">
						<span class="block font-medium text-ink group-hover:underline">{words.title}</span>
						<span class="block text-sm text-ink-muted">{words.note}</span>
					</span>
					<ChevronRightIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
				</a>
			</li>
		{/each}
	</ul>
</section>

<section class="py-10 xl:py-16" aria-labelledby="method">
	<Heading level={2}><span id="method">{c.methodTitle}</span></Heading>
	<div class="mt-6 grid items-start gap-8 xl:grid-cols-2">
		<div class="flex flex-col gap-6">
			<div>
				<h3 class="text-h3 m-0 font-semibold">{c.apostilleTitle}</h3>
				<p class="mt-2 mb-0 text-ink-muted">{c.apostilleText}</p>
			</div>
			<div>
				<h3 class="text-h3 m-0 font-semibold">{c.legalTitle}</h3>
				<p class="mt-2 mb-0 text-ink-muted">{c.legalText}</p>
			</div>
			<p class="m-0 text-sm">
				{c.examplesLabel}
				{#each EXAMPLES as ex, i (ex.slug)}
					{#if i > 0}<span aria-hidden="true"> · </span>{/if}<a
						class="underline underline-offset-4"
						href={authorityHref(ex.slug)}>{c.examples[ex.key]}</a
					>
				{/each}
			</p>
		</div>
		<figure class="m-0">
			<HagueMap facts={hcchFacts} pins={mapPins} class="w-full" />
			<figcaption class="mt-2 text-sm text-ink-muted">{c.mapLead}</figcaption>
		</figure>
	</div>
</section>

<section class="py-10 xl:py-16" aria-labelledby="index">
	<Heading level={2}><span id="index">{c.indexTitle}</span></Heading>
	<p class="mt-2 mb-0 text-ink-muted">{c.indexIntro}</p>
	<div class="mt-4 max-w-md">
		<label class="sr-only" for="index-q">{c.indexFilterLabel}</label>
		<input
			id="index-q"
			type="search"
			bind:value={query}
			placeholder={c.indexFilterPlaceholder}
			autocomplete="off"
			class="h-11 w-full rounded-lg border border-line bg-surface px-3 text-base text-ink outline-none placeholder:text-ink-muted focus:border-focus"
		/>
	</div>
	{#if FILTERS.length > 1}
		<div class="mt-4 flex flex-wrap gap-2" role="group" aria-label={c.filterLabel}>
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
		<Notice variant="error" heading={c.searchUnavailable}><p>{loadError}</p></Notice>
	{:else if index === null}
		<div role="status" aria-label={c.loadingIndex} class="mt-4 flex flex-col gap-2">
			<span class="sr-only">{c.loadingIndex}</span>
			<Skeleton class="h-11 w-full" />
			<Skeleton class="h-11 w-full" />
			<Skeleton class="h-11 w-2/3" />
		</div>
	{:else if tableRows.length === 0}
		<div class="mt-4 rounded-xl border border-line p-6">
			<p class="m-0 font-medium text-ink">
				{tableFilter === 'all' ? c.noOthers : c.noInGroup}
			</p>
			<p class="mt-1 mb-0 text-sm text-ink-muted">
				{tableFilter === 'all' ? c.newAppear : c.tryAll}
			</p>
		</div>
	{:else if !blank}
		<p class="mt-4 mb-2 text-sm text-ink-muted" role="status">
			{matches.length === 0
				? c.noMatch(query)
				: matches.length > MATCH_CAP
					? c.matchCap(MATCH_CAP, matches.length)
					: c.matchCount(matches.length)}
		</p>
		<ul class="m-0 list-none p-0" aria-label={c.matchingQuery(query)}>
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
							{factCount(authority.fact_count ?? 0, site.locale)}
						</span>
					</a>
				</li>
			{/each}
		</ul>
	{:else}
		<p class="mt-4 mb-2 text-sm text-ink-muted">
			{c.indexSummary(tableRows.length, groups.length)}
		</p>
		<ul class="m-0 list-none p-0" aria-label={c.byCountry}>
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
								<Flag code={flag} label={c.flagOf(countryText(group.country))} />
							{/if}
							<span class="min-w-0 flex-1 font-medium break-words text-ink">
								{countryText(group.country)}
							</span>
							<span class="flex-none font-mono text-xs text-ink-muted">
								{c.authorityCount(group.rows.length)}{#if group.facts > 0}
									· {factCount(group.facts, site.locale)}{/if}
							</span>
						</summary>
						{#if opened[group.country]}
							{@const place = placePath(group.iso2, group.country)}
							{#if place}
								<a
									class="ml-2 inline-flex min-h-11 items-center text-sm sm:ml-8"
									href={resolve('/(public)/in/[...path]', { path: place })}
								>
									{c.whichOffice(countryText(group.country))}
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
													{kindText(authority)}
												</span>
											{/if}
											<span class="flex-none font-mono text-xs text-ink-muted">
												{factCount(authority.fact_count ?? 0, site.locale)}
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
</style>
