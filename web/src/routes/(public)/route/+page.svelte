<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import Flag from '../../../lib/components/brand/Flag.svelte';
	import { memberAlpha2FromFacts } from '../../../lib/components/brand/HagueMap.members.js';
	import ChainView from '../../../lib/public-ui/ChainView.svelte';
	import FollowCard from '../../../lib/public-ui/FollowCard.svelte';
	import CitationMark from '../../../lib/public-ui/CitationMark.svelte';
	import { flagFor } from '../../../lib/public-ui/authority-meta.js';
	import { listAuthorities, publicError } from '../../../lib/public-ui/handbook.js';
	import { parseStatement } from '../../../lib/public-ui/guide.js';
	import {
		answerFor,
		DOC_CLASSES,
		parseLocation,
		placeWord,
		readLocation,
		sourceHost,
		type LocationView
	} from '../../../lib/public-ui/location.js';
	import {
		codeOfPlace,
		destinationRow,
		hasCountry,
		HCCH_SLUG,
		methodFor,
		parseRouteQuery,
		placeOfCode,
		routeHref
	} from '../../../lib/public-ui/route.js';
	import { ROUTE_COPY } from '../../../lib/public-ui/route-copy.js';
	import { PLACE_COPY } from '../../../lib/public-ui/place-copy.js';
	import { INTL_TAG } from '../../../lib/public-ui/home-copy.js';
	import { site } from '../../../lib/locale.svelte.js';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import LinkIcon from '@lucide/svelte/icons/link';

	// The guided answer (home hero, common requests): a document issued
	// in `from` (a place address, `usa` or `usa/washington`), used in `to`
	// (a country code), optionally of one document class. Everything shown
	// is runtime: the method from accepted HCCH facts, the issuing office
	// from the place API, the embassy chain and fees from accepted facts.
	// The address is the answer, so it can be sent to family or an employer.

	const c = $derived(ROUTE_COPY[site.locale]);
	const pc = $derived(PLACE_COPY[site.locale]);
	const q = $derived(parseRouteQuery(page.url.searchParams));
	const fromCode = $derived(codeOfPlace(q.from));
	const toCode = $derived(q.to);
	const ready = $derived(fromCode !== '' && toCode !== '');

	const regionNames = $derived.by(() => {
		try {
			return new Intl.DisplayNames([INTL_TAG[site.locale]], { type: 'region' });
		} catch {
			return null;
		}
	});
	const nameOf = (code: string) => regionNames?.of(code.toUpperCase()) ?? code.toUpperCase();
	const fromName = $derived(fromCode ? nameOf(fromCode) : '');
	const toName = $derived(toCode ? nameOf(toCode) : '');

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	async function get(p: string) {
		const res = await fetch(`${apiRoot}/api/v1${p}`, { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}
	const enc = encodeURIComponent;

	// ------------------------------------------------- loading

	let index = $state<any[] | null>(null);
	let members = $state<string[] | null>(null);
	let view = $state<LocationView | null>(null);
	let viewMissing = $state(false);
	let loadStatus = $state<number | null>(null);
	const loadError = $derived(
		loadStatus === null ? null : publicError(loadStatus, 'index', site.locale)
	);

	$effect(() => {
		listAuthorities({ get })
			.then((res) => {
				if (!res.ok) loadStatus = res.status;
				else index = Array.isArray(res.body?.authorities) ? res.body.authorities : [];
			})
			.catch(() => (loadStatus = 500));
		get(`/authorities/${HCCH_SLUG}/facts`)
			.then((res) => (members = memberAlpha2FromFacts(res.ok ? res.body?.facts : [])))
			.catch(() => (members = []));
	});

	$effect(() => {
		const from = q.from;
		view = null;
		viewMissing = false;
		if (!from) return;
		readLocation(from, { get })
			.then((res) => {
				if (from !== q.from) return;
				if (res.ok) view = parseLocation(res.body);
				else viewMissing = true;
			})
			.catch(() => {
				if (from === q.from) viewMissing = true;
			});
	});

	// A state or province page names the place itself ("Washington").
	const fromPlaceName = $derived(view?.location.subdivision ? view.location.label : fromName);

	const method = $derived(members === null ? null : methodFor(fromCode, toCode, members));

	// Step 1: the issuing office for the picked document.
	const docClasses = $derived(
		view
			? DOC_CLASSES.filter(
					(d) => view!.routes.some((r) => r.doc_class === d.id) || view!.by_place.includes(d.id)
				)
			: []
	);
	const docLabel = (id: string) =>
		pc.docClasses[id] ?? DOC_CLASSES.find((d) => d.id === id)?.label ?? id;
	const answer = $derived(view && q.doc ? answerFor(view, q.doc) : null);
	const office = $derived(answer?.kind === 'route' ? answer.route : null);
	const placeKind = $derived(view ? placeWord(view) : 'state');

	// The office's published fees and timing (plan blocks, each cited).
	type Fact = { id?: number; statement?: string; citations?: any[]; drafted_by?: string; created_by?: string };
	type PlanBlock = {
		kind: string;
		title?: string;
		fact_ids?: number[];
		show_for?: string[];
		lines?: { label: string; amount_cents: number; per?: string; fact_id?: number }[];
	};
	let officePlan = $state<{ blocks: PlanBlock[]; facts: Fact[] } | null>(null);
	$effect(() => {
		const slug = office?.office.slug ?? '';
		officePlan = null;
		if (!slug) return;
		get(`/authorities/${enc(slug)}/plan`)
			.then((res) => {
				if (slug !== (office?.office.slug ?? '') || !res.ok) return;
				officePlan = {
					blocks: Array.isArray(res.body?.plan?.blocks) ? res.body.plan.blocks : [],
					facts: Array.isArray(res.body?.facts) ? res.body.facts : []
				};
			})
			.catch(() => {});
	});
	const shows = (b: PlanBlock) =>
		!Array.isArray(b.show_for) || b.show_for.length === 0 || b.show_for.includes(q.doc);
	const feeLines = $derived(
		(officePlan?.blocks ?? []).filter((b) => b.kind === 'fees' && shows(b)).flatMap((b) => b.lines ?? [])
	);
	const timing = $derived(
		(officePlan?.blocks ?? []).find((b) => b.kind === 'timing' && shows(b)) ?? null
	);
	const factById = (id: number | undefined) =>
		id === undefined ? null : ((officePlan?.facts ?? []).find((f) => f.id === id) ?? null);
	// A US office's fee facts are written in dollars; elsewhere the plan
	// carries no currency, so only the label and its quote show.
	const usd = (cents: number) =>
		new Intl.NumberFormat(INTL_TAG[site.locale], { style: 'currency', currency: 'USD' }).format(
			cents / 100
		);

	// Step 2: the destination's own guide and, for legalization, its chain.
	const dest = $derived(index ? destinationRow(index, toCode) : null);
	let destFacts = $state<any[] | null>(null);
	$effect(() => {
		const slug = dest?.slug ?? '';
		destFacts = null;
		if (!slug || method !== 'legalization') return;
		get(`/authorities/${enc(slug)}/facts`)
			.then((res) => {
				if (slug === (dest?.slug ?? ''))
					destFacts = res.ok && Array.isArray(res.body?.facts) ? res.body.facts : [];
			})
			.catch(() => (destFacts = []));
	});
	const hasChain = $derived(
		(destFacts ?? []).some((f) => f?.status === 'accepted' && f?.topic === 'legalization_chain')
	);

	const routeBase = resolve('/(public)/route');
	const hrefWith = (patch: Partial<{ from: string; to: string; doc: string }>) =>
		routeHref(routeBase, { ...q, ...patch });
	const pickDoc = (id: string) =>
		goto(hrefWith({ doc: q.doc === id ? '' : id }), { replace: true, reset: false });
	const dossierHref = (address: string) =>
		resolve('/(public)/authority/[...trail]', { trail: address });
	const orderHref = $derived(
		office
			? `${resolve('/(customer)/order')}?destination=${enc(office.office.slug)}`
			: resolve('/(customer)/order')
	);

	let copied = $state(false);
	async function copyLink() {
		try {
			await navigator.clipboard.writeText(page.url.href);
			copied = true;
			setTimeout(() => (copied = false), 2000);
		} catch {
			// No clipboard: the address bar still holds the link.
		}
	}
</script>

{#snippet cite(fact: Fact | null)}
	<CitationMark
		citation={fact?.citations?.[0] ?? null}
		value={parseStatement(fact?.statement ?? '').value}
		draftedBy={String(fact?.drafted_by ?? fact?.created_by ?? 'unknown').split(' ')[0]}
		variant="chip"
	/>
{/snippet}

<svelte:head>
	<title>{ready ? c.headTitle(fromName, toName) : c.headBlank}</title>
</svelte:head>

{#if !ready}
	<section class="py-10 xl:py-16" aria-labelledby="route-title">
		<Heading level={1}><span id="route-title">{c.pickTitle}</span></Heading>
		<p class="text-body-lg mt-4 max-w-[68ch] text-ink-muted">{c.pickText}</p>
		<p class="mt-6">
			<a class="inline-flex min-h-11 items-center gap-1 font-medium" href={resolve('/(public)')}>
				{c.backHome}
				<ArrowRightIcon class="size-4" aria-hidden="true" />
			</a>
		</p>
	</section>
{:else}
	<header class="py-10 xl:py-12" aria-labelledby="route-title">
		<p class="text-eyebrow m-0 font-semibold tracking-[0.06em] text-ink-muted uppercase">
			{c.eyebrow}
		</p>
		<div class="mt-3 flex items-center gap-2" aria-hidden="true">
			{#each [fromCode, toCode] as code, i (i)}
				{@const f = flagFor('', code)}
				{#if i === 1}<ArrowRightIcon class="size-4 text-ink-muted" />{/if}
				{#if f}<Flag code={f} label="" class="size-7" />{/if}
			{/each}
		</div>
		<div class="mt-3">
			<Heading level={1}><span id="route-title">{c.title(fromPlaceName, toName)}</span></Heading>
		</div>
		<div class="mt-4 flex flex-wrap gap-x-6 gap-y-1 text-sm">
			<a class="inline-flex min-h-11 items-center" href={resolve('/(public)')}>{c.change}</a>
			<button
				type="button"
				onclick={copyLink}
				class="inline-flex min-h-11 cursor-pointer items-center gap-1 text-accent-strong"
			>
				<LinkIcon class="size-4" aria-hidden="true" />
				<span aria-live="polite">{copied ? c.copied : c.copyLink}</span>
			</button>
		</div>
	</header>

	{#if loadError}
		<Notice variant="error" heading={c.loading}><p>{loadError}</p></Notice>
	{/if}

	<!-- The method: apostille, legalization, nothing, or not yet known. -->
	<section
		class="rounded-xl border border-line bg-surface p-5 sm:p-6"
		aria-labelledby="route-method"
		aria-busy={method === null}
	>
		{#if method === null}
			<span class="sr-only">{c.loading}</span>
			<Skeleton class="h-7 w-2/3" />
			<Skeleton class="mt-3 h-4 w-full" />
		{:else}
			<h2 id="route-method" class="text-h2 m-0 font-brand font-medium" data-method={method}>
				{c.methods[method].title}
			</h2>
			<p class="mt-3 mb-0 max-w-[68ch] text-ink">
				{c.methods[method].text(fromPlaceName, toName)}
			</p>
			{#if method === 'apostille' || method === 'legalization'}
				<p class="mt-3 mb-0 text-sm">
					<a href={dossierHref(HCCH_SLUG)}>{c.methodSource}</a>
				</p>
			{/if}
		{/if}
	</section>

	<!-- Step 1: where the document was issued. -->
	<section class="py-10" aria-labelledby="route-step1">
		<h2 id="route-step1" class="text-h2 m-0 font-semibold">{c.step1(fromPlaceName)}</h2>
		{#if viewMissing}
			<p class="mt-3 mb-0 text-ink-muted">{c.noOffices(fromPlaceName)}</p>
		{:else if !view}
			<div class="mt-4 flex flex-col gap-2" role="status" aria-label={c.loading}>
				<Skeleton class="h-11 w-full" />
				<Skeleton class="h-11 w-2/3" />
			</div>
		{:else}
			{#if docClasses.length > 0}
				<h3 class="mt-4 mb-0 text-sm font-medium text-ink">{c.whatDoc}</h3>
				<div class="mt-2 flex flex-wrap gap-2" role="group" aria-label={pc.docType}>
					{#each docClasses as d (d.id)}
						<button
							type="button"
							aria-pressed={q.doc === d.id}
							onclick={() => pickDoc(d.id)}
							class={`inline-flex min-h-11 cursor-pointer items-center rounded-full border px-4 text-sm transition-colors ${
								q.doc === d.id
									? 'border-accent-fill bg-accent-fill font-medium text-accent-ink'
									: 'border-line bg-bg text-ink hover:border-focus'
							}`}
						>
							{docLabel(d.id)}
						</button>
					{/each}
				</div>
			{:else}
				<p class="mt-3 mb-0 text-ink-muted">{c.noOffices(fromPlaceName)}</p>
			{/if}

			<div aria-live="polite" class="mt-6">
				{#if office}
					<article class="rounded-xl border border-line bg-bg p-5" aria-label={c.officeLabel}>
						<p class="text-eyebrow m-0 font-semibold tracking-wide text-ink-muted uppercase">
							{c.officeLabel}{#if office.inherited}
								· {pc.federal}{/if}
						</p>
						<h3 class="mt-2 mb-0 text-lg font-semibold">
							<a href={dossierHref(office.office.path)}>{office.office.name}</a>
						</h3>
						{#if office.prereq}
							<p class="mt-2 mb-0 text-sm text-ink">{pc.first(office.prereq)}</p>
						{/if}
						<p class="mt-3 mb-0 flex flex-wrap items-center gap-2 text-xs text-ink-muted">
							{#if office.verified}
								<span
									class="inline-block rounded-xs border border-success bg-success-bg px-2 py-[0.15rem] font-mono font-semibold text-success"
									data-verified="yes">{pc.verifiedByStaff}</span
								>
							{:else}
								<span
									class="inline-block rounded-xs border border-line bg-surface-2 px-2 py-[0.15rem] font-mono font-semibold text-ink-muted"
									data-verified="no">{pc.unverified}</span
								>
							{/if}
							{#if office.source_uri}
								<span>
									{pc.source}
									<a href={office.source_uri} target="_blank" rel="noopener noreferrer"
										>{sourceHost(office.source_uri)}</a
									>
								</span>
							{/if}
						</p>
						{#if feeLines.length > 0 || timing}
							<div class="mt-4 border-t border-line pt-4">
								<p class="text-eyebrow m-0 font-semibold tracking-[0.06em] text-ink-muted uppercase">
									{c.feesTitle}
								</p>
								<ul class="m-0 mt-2 flex list-none flex-col gap-1 p-0 text-sm">
									{#each feeLines as fee, i (`${i}-${fee.label}`)}
										<li class="flex flex-wrap items-center gap-x-2">
											<span class="text-ink">
												{fee.label}{#if fromCode === 'us'}: {usd(fee.amount_cents)}{/if}{#if fee.per}{' '}{c.per(fee.per)}{/if}
											</span>
											{#if factById(fee.fact_id)}{@render cite(factById(fee.fact_id))}{/if}
										</li>
									{/each}
									{#if timing}
										<li class="flex flex-wrap items-center gap-x-2">
											<span class="text-ink">{timing.title}</span>
											{#if factById(timing.fact_ids?.[0])}{@render cite(factById(timing.fact_ids?.[0]))}{/if}
										</li>
									{/if}
								</ul>
							</div>
						{/if}
					</article>
				{:else if answer?.kind === 'by_place'}
					<p class="m-0 text-sm text-ink">{pc.byPlaceAnswer(docLabel(q.doc), placeKind)}</p>
					<h3 class="mt-4 mb-0 text-sm font-medium text-ink">{c.pickPlace}</h3>
					<ul class="m-0 mt-2 grid list-none grid-cols-2 gap-x-6 p-0 sm:grid-cols-3 lg:grid-cols-4">
						{#each view.places as place (place.path)}
							<li class="m-0 p-0">
								<a class="flex min-h-11 items-center text-sm" href={hrefWith({ from: place.path })}
									>{place.label}</a
								>
							</li>
						{/each}
					</ul>
				{:else if q.doc && docClasses.length > 0}
					<p class="m-0 text-sm text-ink-muted">{pc.noOffice}</p>
				{/if}
			</div>
		{/if}
		{#if placeOfCode(fromCode) && !viewMissing}
			<p class="mt-6 mb-0 text-sm">
				<a href={resolve('/(public)/in/[...path]', { path: q.from })}>{c.allOffices(fromPlaceName)}</a>
			</p>
		{/if}
	</section>

	<!-- Step 2: what the destination needs. -->
	<section class="border-t border-line py-10" aria-labelledby="route-step2">
		<h2 id="route-step2" class="text-h2 m-0 font-semibold">{c.step2(toName)}</h2>
		{#if method === 'same'}
			<p class="mt-3 mb-0 text-ink">{c.step2Same}</p>
		{:else if method === 'apostille'}
			<p class="mt-3 mb-0 max-w-[68ch] text-ink">{c.step2Apostille(toName)}</p>
		{:else if method === 'legalization'}
			{#if hasChain}
				<p class="mt-3 mb-4 text-ink">{c.step2Chain}</p>
				<ChainView facts={destFacts ?? []} />
			{:else if destFacts !== null || !dest}
				<p class="mt-3 mb-0 text-ink-muted">{c.step2NoChain(toName)}</p>
			{/if}
		{/if}
		{#if index !== null}
			{#if dest}
				<p class="mt-6 mb-0">
					<a
						class="inline-flex min-h-11 items-center gap-1 font-medium"
						href={dossierHref(dest.path || dest.slug)}
					>
						{c.guideFor(toName)}
						<ArrowRightIcon class="size-4" aria-hidden="true" />
					</a>
				</p>
				{#if !dest.fact_count}
					<div class="mt-6 max-w-xl">
						<FollowCard slug={dest.slug} name={dest.name ?? toName} apiBase={apiRoot} />
					</div>
				{/if}
			{:else if hasCountry(index, toCode) && placeOfCode(toCode)}
				<p class="mt-6 mb-0">
					<a
						class="inline-flex min-h-11 items-center gap-1 font-medium"
						href={resolve('/(public)/in/[...path]', { path: placeOfCode(toCode) })}
					>
						{c.allOffices(toName)}
						<ArrowRightIcon class="size-4" aria-hidden="true" />
					</a>
				</p>
			{:else}
				<p class="mt-6 mb-0 text-ink-muted">{c.notCovered(toName)}</p>
			{/if}
		{/if}
	</section>

	<!-- What we can do: file it for a US document, otherwise guide only. -->
	<section class="mb-10 rounded-xl border border-line bg-surface p-5 sm:p-6" aria-labelledby="route-next">
		{#if fromCode === 'us'}
			<h2 id="route-next" class="text-h3 m-0 font-semibold">{c.doneTitle}</h2>
			<p class="mt-2 mb-4 text-sm text-ink-muted">{c.doneText}</p>
			<a
				href={orderHref}
				class="inline-flex min-h-12 items-center gap-2 rounded-lg bg-accent-fill px-6 font-semibold text-accent-ink no-underline transition-opacity hover:opacity-90"
				data-cta="order"
			>
				{c.doneCta}
				<ArrowRightIcon class="size-4" aria-hidden="true" />
			</a>
		{:else}
			<h2 id="route-next" class="text-h3 m-0 font-semibold">{c.abroadTitle(fromPlaceName)}</h2>
			<p class="mt-2 mb-0 text-sm text-ink-muted">{c.abroadText}</p>
		{/if}
	</section>
{/if}
