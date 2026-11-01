<script lang="ts">
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import VerifiedChip from '../../../../lib/public-ui/VerifiedChip.svelte';
	import GuideStep from '../../../../lib/public-ui/GuideStep.svelte';
	import GuideEmpty from '../../../../lib/public-ui/GuideEmpty.svelte';
	import FollowCard from '../../../../lib/public-ui/FollowCard.svelte';
	import { Skeleton } from '../../../../lib/components/ui/skeleton/index.js';
	import * as HoverCard from '../../../../lib/components/ui/hover-card/index.js';
	import Heading from '../../../../lib/components/brand/Heading.svelte';
	import Guilloche from '../../../../lib/components/brand/Guilloche.svelte';
	import Flag from '../../../../lib/components/brand/Flag.svelte';
	import silhouettes from '../../../../lib/geo/silhouettes.json';
	import { subscribe } from '../../../../lib/live.svelte.js';
	import {
		factCount,
		groupFactsByTopic,
		publicError,
		readDossier,
		seedLedgerEvents,
		topicLabel
	} from '../../../../lib/public-ui/handbook.js';
	import { buildGuide, domain, markQuote } from '../../../../lib/public-ui/guide.js';

	const slug = $derived(page.params.slug ?? '');

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	// Per-authority display facts the API does not carry yet. Assumed:
	// Viet Nam's native name and HCCH Convention entry date come from the
	// watched HCCH pages (packs/vietnam-routing.json), not the API.
	const NATIVE: Record<string, { name: string; lang: string }> = {
		vietnam: { name: 'Việt Nam', lang: 'vi' }
	};
	const IN_FORCE: Record<string, string> = {
		vietnam: '2026-09-11'
	};
	const FLAG: Record<string, string> = { vietnam: 'vn', 'usa-wa-sos': 'us', hcch: 'nl' };
	const SIL: Record<string, keyof typeof silhouettes> = {
		vietnam: 'vietnam',
		'usa-wa-sos': 'washington',
		hcch: 'netherlands'
	};

	let facts = $state<any[] | null>(null);
	let loadError = $state<string | null>(null);
	let notFound = $state(false);
	let authorityName = $state<string | null>(null);
	let authorityCountry = $state<string | null>(null);
	let authorityRegion = $state<string | null>(null);
	let activeTopic = $state('');

	type LedgerEvent = {
		id: string;
		kind: 'verified' | 'changed';
		text: string;
		at: string;
		fresh: boolean;
	};
	let ledger = $state<LedgerEvent[]>([]);

	const groups = $derived(groupFactsByTopic(facts ?? []));
	const topicId = (topic: string) =>
		`topic-${topic.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;

	function firstCapture(fact: any): any | null {
		const cites = Array.isArray(fact?.citations) ? fact.citations : [];
		return cites.length > 0 ? cites[0] : null;
	}

	function latestCapture(list: any[]): string {
		let latest = '';
		for (const f of list) {
			for (const c of Array.isArray(f?.citations) ? f.citations : []) {
				if (typeof c?.captured_at === 'string' && c.captured_at > latest)
					latest = c.captured_at;
			}
		}
		return latest;
	}

	function shortDate(iso: string): string {
		return String(iso ?? '').split('T')[0];
	}

	function modelName(fact: any): string {
		const raw = String(fact?.drafted_by ?? fact?.created_by ?? 'unknown');
		return raw.split(' ')[0];
	}

	function seedLedger(list: any[]) {
		ledger = seedLedgerEvents(list) as LedgerEvent[];
	}

	function pushLiveEvent() {
		const id = `live-${Date.now()}`;
		ledger = [
			{ id, kind: 'verified', text: 'New fact approved by staff', at: '', fresh: true },
			...ledger
		];
		setTimeout(() => {
			ledger = ledger.map((e) => (e.id === id ? { ...e, fresh: false } : e));
		}, 600);
	}

	async function reload(current: string) {
		const res = await readDossier(current, { get });
		if (current !== slug) return; // stale slug, ignore
		if (!res.ok) {
			if (res.status === 404) notFound = true;
			else loadError = publicError(res.status, 'dossier');
			return;
		}
		const rows = res.body?.facts;
		const list = Array.isArray(rows) ? rows : [];
		facts = list;
		activeTopic = groupFactsByTopic(list)[0]?.topic ?? activeTopic;
		seedLedger(list);
		armFlash(list);
	}

	async function resolveName(current: string) {
		try {
			const res = await get('/authorities');
			if (current !== slug || !res.ok) return;
			const rows = res.body?.authorities;
			const found = Array.isArray(rows) ? rows.find((a) => a?.slug === current) : null;
			if (found && typeof found.name === 'string' && found.name) authorityName = found.name;
			if (found && typeof found.country === 'string' && found.country)
				authorityCountry = found.country;
			if (found && typeof found.region === 'string' && found.region)
				authorityRegion = found.region;
		} catch {
			// The slug stays the heading; the index is a nicety, not a gate.
		}
	}

	let sectionsRoot = $state<HTMLElement | null>(null);
	let spy: IntersectionObserver | null = null;

	function observeSections() {
		spy?.disconnect();
		if (!sectionsRoot || typeof IntersectionObserver === 'undefined') return;
		spy = new IntersectionObserver(
			(entries) => {
				for (const e of entries) {
					if (e.isIntersecting && e.target instanceof HTMLElement && e.target.dataset.topic) {
						activeTopic = e.target.dataset.topic;
					}
				}
			},
			{ rootMargin: '-30% 0px -60% 0px' }
		);
		for (const el of sectionsRoot.querySelectorAll('[data-topic]')) spy.observe(el);
	}

	onMount(() => {
		observeSections();
		return () => spy?.disconnect();
	});

	$effect(() => {
		// Re-observe after each load renders new sections.
		if (facts && sectionsRoot) observeSections();
	});

	$effect(() => {
		const current = slug;
		facts = null;
		loadError = null;
		notFound = false;
		activeTopic = '';
		authorityName = null;
		authorityCountry = null;
		ledger = [];
		reload(current).catch(() => {
			if (current !== slug) return;
			loadError = publicError(500, 'dossier');
		});
		void resolveName(current);
		// Live ledger: an approval in /review re-renders this dossier
		// over WS `authority:{slug}` — no polling.
		return subscribe(`authority:${current}`, (msg) => {
			if (current !== slug) return;
			if (msg?.type === 'fact_accepted') {
				pushLiveEvent();
				if (typeof msg?.fact_id === 'number') pendingFlashFactId = msg.fact_id;
				void reload(current).catch(() => {});
			}
		});
	});

	const guideSteps = $derived(buildGuide(facts ?? [], Date.now()));

	// WS flash: the last accepted fact's step lights up after reload.
	let flashStepId = $state<string | null>(null);
	let pendingFlashFactId = $state<number | null>(null);
	let flashTimer: ReturnType<typeof setTimeout> | null = null;

	function armFlash(list: any[]) {
		if (pendingFlashFactId === null) return;
		const found = list.find((f) => f?.id === pendingFlashFactId);
		pendingFlashFactId = null;
		if (!found) return;
		const step = guideSteps.find((s) => s.items.some((i) => i.factIds.includes(found.id)));
		if (!step) return;
		if (flashTimer) clearTimeout(flashTimer);
		flashStepId = step.id;
		flashTimer = setTimeout(() => {
			flashStepId = null;
			flashTimer = null;
		}, 1500);
	}

	const websiteUrl = $derived(
		(() => {
			for (const f of facts ?? []) {
				if (f?.topic !== 'website' || f?.status !== 'accepted') continue;
				const cites = Array.isArray(f.citations) ? f.citations : [];
				const url = cites[0]?.source_url;
				if (typeof url === 'string' && url.startsWith('https://')) return url;
			}
			return null;
		})()
	);

	const displayName = $derived(authorityName ?? slug);
	const native = $derived(NATIVE[slug]);
	const inForce = $derived(IN_FORCE[slug]);
	const flag = $derived(FLAG[slug] ?? null);
	const sil = $derived(SIL[slug] ? silhouettes[SIL[slug]] : null);
	const lastVerified = $derived(facts ? latestCapture(facts) : '');
</script>

<svelte:head>
	<title>{displayName} — Apostille Handbook</title>
</svelte:head>

{#if notFound}
	<Notice variant="warning" heading="Unknown authority “{slug}”.">
		<p>
			This slug is not in the handbook index.
			<a href={resolve('/(public)')}>Back to the authority index</a>.
		</p>
	</Notice>
{:else if loadError}
	<Notice variant="error" heading="This dossier is unavailable."><p>{loadError}</p></Notice>
{:else if facts === null}
	<div role="status" aria-label="Loading dossier" class="flex flex-col gap-2 pt-6">
		<span class="sr-only">Loading dossier</span>
		<Skeleton class="h-9 w-2/3" />
		<Skeleton class="h-4 w-1/3" />
		<Skeleton class="h-24 w-full" />
	</div>
{:else}
	<header class="relative overflow-clip py-10 xl:py-16" aria-labelledby="dossier-title">
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
		<nav aria-label="Breadcrumb">
			<ol class="m-0 flex list-none flex-wrap gap-1 p-0 text-sm text-ink-muted">
				<li class="m-0 p-0">
					<a class="text-ink-muted no-underline hover:text-ink hover:underline" href={resolve('/(public)')}>
						Home
					</a>
				</li>
				<li class="m-0 p-0" aria-hidden="true">/</li>
				<li class="m-0 p-0" aria-current="page">{displayName}</li>
			</ol>
		</nav>
		<p class="text-eyebrow mt-6 font-semibold tracking-[0.06em] text-ink-muted uppercase">
			Authority dossier
		</p>
		<div class="mt-2 flex items-center gap-3">
			{#if flag}
				<Flag code={flag} label={`${authorityCountry ?? displayName} flag`} class="size-8 vt-flag" />
			{/if}
			<Heading level={1}>
				<span class="vt-title">{displayName}</span>
			</Heading>
		</div>
		{#if native}
			<p lang={native.lang} class="mt-2 mb-0 text-lg text-ink-muted">{native.name}</p>
		{/if}
		{#if facts.length > 0}
			<p class="mt-3 mb-0 font-mono text-xs text-ink-muted">
				{#if inForce}<span>in force since {inForce}</span> · {/if}<span
					>last verified {shortDate(lastVerified) || '—'}</span
				>
				· <span>{factCount(facts.length)}</span>
			</p>
		{/if}
		<div class="mt-6 flex flex-wrap gap-2">
			<a
				href={`${resolve('/(customer)/order')}?destination=${encodeURIComponent(slug)}`}
				class="inline-flex min-h-11 items-center rounded-sm bg-accent-fill px-5 font-medium text-accent-ink no-underline transition-colors hover:bg-accent-hover"
			>
				Start an order for {authorityRegion ?? authorityCountry ?? displayName}
			</a>
			<a
				href={resolve('/(customer)/login')}
				class="inline-flex min-h-11 items-center rounded-sm border border-line bg-bg px-5 font-medium text-ink no-underline transition-colors hover:border-focus"
			>
				Report a change
			</a>
		</div>
	</header>

	<section aria-labelledby="howto" class="border-y border-line bg-surface py-10 xl:py-16">
		<div class="mx-auto w-full max-w-5xl px-4 sm:px-6">
			<h2 id="howto" class="text-h2 mt-0 mb-0 font-semibold">
				How to get your apostille<span class="sr-only"> for {displayName}</span>
			</h2>
			{#if guideSteps.length === 0}
				<div class="mt-8">
					<GuideEmpty websiteUrl={websiteUrl} slug={slug} />
				</div>
			{:else}
				<ol class="m-0 mt-8 list-none p-0">
					{#each guideSteps as step, i (step.id)}
						<GuideStep step={step} index={i} flash={flashStepId === step.id} />
					{/each}
				</ol>
				<p class="mt-8 mb-0 max-w-prose text-sm text-ink-muted">
					Every value above is staff-accepted, with its source one tap
					away. General information, not legal advice.
				</p>
			{/if}
		</div>
	</section>
	<section aria-label="Follow this office" class="mx-auto w-full max-w-5xl px-4 py-10 sm:px-6 xl:py-16">
		<FollowCard slug={slug} name={displayName} apiBase={apiRoot} />
	</section>
	{#if facts.length > 0}
		<section aria-label="Evidence" class="grid gap-8 pt-10 pb-16 lg:grid-cols-12 xl:pt-16">
			<div class="min-w-0 lg:col-span-8" bind:this={sectionsRoot}>
				<nav
					class="sticky top-14 z-20 -mx-1 border-b border-line bg-bg/90 px-1 py-1 backdrop-blur"
					aria-label="Dossier sections"
				>
					<ul class="m-0 flex list-none flex-wrap gap-1 p-0">
						{#each groups as g (g.topic)}
							<li class="m-0 p-0">
								<a
									href={`#${topicId(g.topic)}`}
									aria-current={activeTopic === g.topic ? 'true' : undefined}
									class={`inline-flex min-h-11 items-center rounded-full px-3 text-sm no-underline transition-colors ${
										activeTopic === g.topic
											? 'bg-accent-fill font-medium text-accent-ink'
											: 'text-ink-muted hover:bg-surface hover:text-ink'
									}`}
								>
									{topicLabel(g.topic)}
								</a>
							</li>
						{/each}
					</ul>
				</nav>
				{#each groups as g (g.topic)}
					<section id={topicId(g.topic)} data-topic={g.topic} aria-label={topicLabel(g.topic)} class="scroll-mt-32 py-10 xl:py-16">
						<div class="flex flex-col gap-4">
							{#each g.facts as fact (fact.id ?? fact.statement)}
								{@const cite = firstCapture(fact)}
								<article class="rounded-xl border border-line bg-bg p-5">
									<p class="text-body-lg m-0 break-words text-ink">{fact.statement}</p>
									<div class="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2 text-sm">
										{#if cite}
											<VerifiedChip tier="Staff approved" verified={shortDate(cite.captured_at)} />
										{/if}
										{#if cite && typeof cite.source_url === 'string'}
											<span class="font-mono text-xs text-ink-muted">{domain(cite.source_url)}</span>
										{/if}
										{#if cite?.captured_at}
											<span class="font-mono text-xs text-ink-muted">
												retrieved {shortDate(cite.captured_at)}
											</span>
										{/if}
										<span class="text-xs text-ink-muted">approved by staff · {modelName(fact)}</span>
										{#if cite?.quote}
											<HoverCard.Root>
												<HoverCard.Trigger
													class="inline-flex min-h-11 cursor-pointer items-center font-medium text-accent-strong underline decoration-dotted underline-offset-4"
												>
													View quote
												</HoverCard.Trigger>
												<HoverCard.Content class="max-w-80" align="start">
													<blockquote class="font-quote m-0 border-l-2 border-verified pl-3 italic">
														{@html markQuote(cite.quote, fact.statement)}
													</blockquote>
													<p class="mt-2 mb-0 text-xs text-ink-muted">
														{#if typeof cite.source_url === 'string' && cite.source_url.startsWith('https://')}
															<a
																href={cite.source_url}
																target="_blank"
																rel="noopener noreferrer">Source</a
															>
														{:else}
															<span>Source unavailable</span>
														{/if}
														· retrieved {shortDate(cite.captured_at)}
													</p>
												</HoverCard.Content>
											</HoverCard.Root>
										{/if}
									</div>
									{#if fact.conflict}
										<div class="mt-4 rounded-lg border-l-4 border-changed bg-changed-bg p-4">
											<p class="m-0 text-sm font-medium text-ink">
												Conflicting reports for this item.
											</p>
											<div class="mt-2 grid gap-2 sm:grid-cols-2">
												<div class="rounded-md border border-line bg-bg p-3">
													<p class="m-0 text-xs font-medium text-ink-muted">Accepted</p>
													<p class="m-0 mt-1 break-words text-sm text-ink">{fact.statement}</p>
												</div>
												{#each fact.conflicts ?? [] as sib (sib.id ?? sib.statement)}
													<div class="rounded-md border border-line bg-bg p-3">
														<p class="m-0 text-xs font-medium text-ink-muted">
															<span
																class="inline-block rounded-full border border-changed bg-changed-bg px-2 py-0.5 text-xs font-semibold text-changed"
															>
																{sib.status === 'accepted' ? 'Accepted' : 'Proposed'}
															</span>
														</p>
														<p class="m-0 mt-1 break-words text-sm text-ink">{sib.statement}</p>
													</div>
												{/each}
											</div>
										</div>
									{/if}
								</article>
							{/each}
						</div>
					</section>
				{/each}
			</div>

			<aside class="min-w-0 lg:col-span-4" aria-label="Evidence ledger">
				<div class="flex flex-col gap-4 lg:sticky lg:top-20">
					<section class="rounded-xl border border-line bg-bg p-5" aria-labelledby="ledger">
						<h2 id="ledger" class="text-h3 m-0 font-semibold">Evidence ledger</h2>
						{#if ledger.length === 0}
							<p class="mt-2 mb-0 text-sm text-ink-muted">No ledger events yet.</p>
						{:else}
							<ol class="m-0 mt-4 list-none p-0">
								{#each ledger as event (event.id)}
									<li
										animate:flip={{ duration: 200 }}
										class={`relative m-0 border-l-2 py-2 pr-2 pl-4 transition-colors duration-300 ${
											event.kind === 'verified' ? 'border-verified' : 'border-changed'
										} ${event.fresh ? 'bg-verified-bg' : ''}`}
									>
										<p class="m-0 text-sm font-medium text-ink">{event.text}</p>
										{#if event.at}
											<p class="m-0 mt-0.5 font-mono text-xs text-ink-muted">
												{shortDate(event.at)}
											</p>
										{/if}
									</li>
								{/each}
							</ol>
						{/if}
					</section>
					<section class="rounded-xl border border-line bg-surface p-5" aria-labelledby="about">
						<h2 id="about" class="text-h3 m-0 font-semibold">About this dossier</h2>
						<p class="mt-2 mb-0 text-sm text-ink-muted">
							{factCount(facts.length)} across {groups.length} topic{groups.length === 1
								? ''
								: 's'}. General information, not legal advice.
						</p>
					</section>
				</div>
			</aside>
		</section>
	{/if}
{/if}

