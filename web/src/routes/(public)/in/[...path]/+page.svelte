<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../../lib/components/ui/skeleton/index.js';
	import Heading from '../../../../lib/components/brand/Heading.svelte';
	import Flag from '../../../../lib/components/brand/Flag.svelte';
	import { flagFor } from '../../../../lib/public-ui/authority-meta.js';
	import {
		answerFor,
		clampName,
		DOC_CLASSES,
		factCount,
		parseLocation,
		placeWord,
		publicError,
		readLocation,
		sourceHost,
		type LocationView
	} from '../../../../lib/public-ui/location.js';

	// A place page (P4-1): "What is your document?" answered with the
	// office that apostilles it here and any prerequisite, then every
	// public office grouped by level. All data is runtime
	// (`GET /api/v1/locations/{path}`); nothing here names a place. A
	// route stays marked unverified until staff accept its rule.
	const path = $derived(page.params.path ?? '');

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	async function get(p: string) {
		const res = await fetch(`${apiRoot}/api/v1${p}`, { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	let view = $state<LocationView | null>(null);
	let notFound = $state(false);
	let loadError = $state<string | null>(null);

	$effect(() => {
		const current = path;
		view = null;
		notFound = false;
		loadError = null;
		readLocation(current, { get })
			.then((res) => {
				if (current !== path) return;
				if (res.status === 404) notFound = true;
				else if (!res.ok) loadError = publicError(res.status, 'index');
				else view = parseLocation(res.body);
			})
			.catch(() => {
				if (current === path) loadError = publicError(500, 'index');
			});
	});

	const total = $derived(view ? view.groups.reduce((n, g) => n + g.offices.length, 0) : 0);
	const flag = $derived(view ? flagFor(view.location.label, view.location.iso2) : null);
	const dossierHref = (address: string) =>
		resolve('/(public)/authority/[...trail]', { trail: address });
	const placeHref = (address: string) => resolve('/(public)/in/[...path]', { path: address });

	// The picked document lives in `?doc=` so an answer is a link.
	const docClasses = $derived(
		view
			? DOC_CLASSES.filter(
					(d) => view!.routes.some((r) => r.doc_class === d.id) || view!.by_place.includes(d.id)
				)
			: []
	);
	const picked = $derived(page.url.searchParams.get('doc') ?? '');
	const answer = $derived(view && picked ? answerFor(view, picked) : null);
	const pickedLabel = $derived(DOC_CLASSES.find((d) => d.id === picked)?.label ?? '');
	function pick(id: string) {
		const q = new URLSearchParams(page.url.search);
		if (id === picked) q.delete('doc');
		else q.set('doc', id);
		const qs = q.toString();
		void goto(`${page.url.pathname}${qs ? `?${qs}` : ''}`, { replace: true, reset: false });
	}
	// "Not sure? We'll route it": an order with this place's own office
	// preselected on a state page, the open destination list otherwise.
	const orderHref = $derived.by(() => {
		const own = view?.groups.find((g) => g.level === 'state')?.offices[0];
		const base = resolve('/(customer)/order');
		return view?.location.subdivision && own
			? `${base}?destination=${encodeURIComponent(own.slug)}`
			: base;
	});
</script>

<svelte:head>
	<title>{view ? `Apostille offices in ${view.location.label}` : 'Apostille offices'} — Apostille Handbook</title>
</svelte:head>

{#if notFound}
	<Notice variant="warning" heading="No offices are listed for “{path}”.">
		<p>
			This place is not in the handbook index.
			<a href={resolve('/(public)')}>Back to the authority index</a>.
		</p>
	</Notice>
{:else if loadError}
	<Notice variant="error" heading="This place is unavailable."><p>{loadError}</p></Notice>
{:else if !view}
	<div role="status" aria-label="Loading place" class="flex flex-col gap-2 pt-6">
		<span class="sr-only">Loading place</span>
		<Skeleton class="h-9 w-2/3" />
		<Skeleton class="h-4 w-1/3" />
		<Skeleton class="h-24 w-full" />
	</div>
{:else}
	<header class="py-10 xl:py-16" aria-labelledby="place-title">
		<nav aria-label="Breadcrumb">
			<ol class="m-0 flex list-none flex-wrap gap-1 p-0 text-sm text-ink-muted">
				<li class="m-0 p-0">
					<a class="text-ink-muted no-underline hover:text-ink hover:underline" href={resolve('/(public)')}>
						Home
					</a>
				</li>
				{#each view.crumbs.slice(0, -1) as crumb (crumb.path)}
					<li class="m-0 p-0" aria-hidden="true">/</li>
					<li class="m-0 p-0">
						<a class="text-ink-muted no-underline hover:text-ink hover:underline" href={placeHref(crumb.path)}>
							{crumb.label}
						</a>
					</li>
				{/each}
				<li class="m-0 p-0" aria-hidden="true">/</li>
				<li class="m-0 p-0" aria-current="page">{view.location.label}</li>
			</ol>
		</nav>
		<p class="text-eyebrow mt-6 font-semibold tracking-[0.06em] text-ink-muted uppercase">
			Apostille offices
		</p>
		<div class="mt-2 flex items-center gap-3">
			{#if flag}
				<Flag code={flag} label={`${view.location.label} flag`} class="size-8" />
			{/if}
			<Heading level={1}><span id="place-title">{view.location.label}</span></Heading>
		</div>
		<p class="mt-3 mb-0 font-mono text-xs text-ink-muted">
			{total}
			{total === 1 ? 'office' : 'offices'}{#if view.places.length > 0}
				· {view.places.length} {placeWord(view)}{view.places.length === 1 ? '' : 's'}{/if}
		</p>
	</header>

	{#if docClasses.length > 0}
		<section aria-labelledby="which-office" class="rounded border border-line bg-surface px-4 py-8 sm:px-6">
			<h2 id="which-office" class="text-h2 m-0 font-semibold">What is your document?</h2>
			<div class="mt-4 flex flex-wrap gap-2" role="group" aria-label="Document type">
				{#each docClasses as doc (doc.id)}
					<button
						type="button"
						aria-pressed={picked === doc.id}
						onclick={() => pick(doc.id)}
						class={`inline-flex min-h-11 items-center rounded-full border px-4 text-sm transition-colors ${
							picked === doc.id
								? 'border-accent-fill bg-accent-fill font-medium text-accent-ink'
								: 'border-line bg-bg text-ink hover:border-focus'
						}`}
					>
						{doc.label}
					</button>
				{/each}
			</div>
			<div aria-live="polite" class="mt-6">
				{#if answer?.kind === 'route'}
					{@const route = answer.route}
					<article class="rounded border border-line bg-bg p-5" aria-label="Where to apostille it">
						<p class="text-eyebrow m-0 font-semibold tracking-wide text-ink-muted uppercase">
							{pickedLabel}{#if route.inherited}
								· federal{/if}
						</p>
						<h3 class="mt-2 mb-0 text-lg font-semibold">
							<a href={dossierHref(route.office.path)}>{route.office.name}</a>
						</h3>
						{#if route.prereq}
							<p class="mt-2 mb-0 text-sm text-ink">First: {route.prereq}</p>
						{/if}
						<p class="mt-3 mb-0 flex flex-wrap items-center gap-2 text-xs text-ink-muted">
							{#if route.verified}
								<span
									class="inline-block rounded-xs border border-success bg-success-bg px-2 py-[0.15rem] font-mono font-semibold text-success"
									data-verified="yes">Verified by staff</span
								>
							{:else}
								<span
									class="inline-block rounded-xs border border-line bg-surface-2 px-2 py-[0.15rem] font-mono font-semibold text-ink-muted"
									data-verified="no">Unverified</span
								>
							{/if}
							{#if route.source_uri}
								<span>
									Source:
									<a href={route.source_uri} target="_blank" rel="noopener noreferrer"
										>{sourceHost(route.source_uri)}</a
									>
								</span>
							{/if}
						</p>
					</article>
				{:else if answer?.kind === 'by_place'}
					<p class="m-0 text-sm text-ink">
						A {pickedLabel.toLowerCase()} is apostilled in the {placeWord(view)} that issued it. Pick
						the {placeWord(view)} below.
					</p>
				{:else if picked}
					<p class="m-0 text-sm text-ink-muted">No office here is listed for this document yet.</p>
				{/if}
			</div>
			<p class="mt-6 mb-0 text-sm">
				Not sure? <a href={orderHref}>We’ll route it</a> — start an order and staff pick the office.
			</p>
		</section>
	{/if}

	<section aria-labelledby="all-offices" class="py-10">
		<h2 id="all-offices" class="text-h2 m-0 font-semibold">All offices here</h2>
		{#each view.groups as group (group.level)}
			<section aria-labelledby={`level-${group.level}`} class="mt-6">
				<h3 id={`level-${group.level}`} class="m-0 text-sm font-semibold tracking-wide text-ink-muted uppercase">
					{group.label}
				</h3>
				<ul class="m-0 mt-2 list-none p-0">
					{#each group.offices as office (office.slug)}
						<li class="m-0 border-b border-line p-0">
							<a
								class="flex min-h-11 items-center gap-3 py-2 no-underline"
								href={dossierHref(office.path)}
								title={office.name}
							>
								<span class="min-w-0 flex-1 font-medium break-words text-ink">
									{clampName(office.name)}
								</span>
								{#if office.region && group.level !== 'state'}
									<span class="hidden flex-none text-sm text-ink-muted sm:inline">{office.region}</span>
								{/if}
								<span class="flex-none font-mono text-xs text-ink-muted">
									{factCount(office.fact_count)}
								</span>
							</a>
						</li>
					{/each}
				</ul>
			</section>
		{/each}
	</section>

	{#if view.places.length > 0}
		<section aria-labelledby="places" class="border-t border-line py-10">
			<h2 id="places" class="text-h2 m-0 font-semibold">By {placeWord(view)}</h2>
			<p class="mt-2 mb-4 max-w-prose text-sm text-ink-muted">
				Each {placeWord(view)} page shows its own offices next to the federal ones.
			</p>
			<ul class="m-0 grid list-none grid-cols-2 gap-x-6 p-0 sm:grid-cols-3 lg:grid-cols-4">
				{#each view.places as place (place.path)}
					<li class="m-0 p-0">
						<a class="flex min-h-11 items-center text-sm" href={placeHref(place.path)}>{place.label}</a>
					</li>
				{/each}
			</ul>
		</section>
	{/if}
{/if}
