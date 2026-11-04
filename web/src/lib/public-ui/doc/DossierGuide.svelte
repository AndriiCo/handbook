<script lang="ts">
	import Cite from './Cite.svelte';
	import PrintHeader from './PrintHeader.svelte';
	import DocHeader from './DocHeader.svelte';
	import StatusBanner from './StatusBanner.svelte';
	import Glance from './Glance.svelte';
	import RouteCards from './RouteCards.svelte';
	import Contents from './Contents.svelte';
	import GuideSection from './GuideSection.svelte';
	import FeeBlock from './FeeBlock.svelte';
	import FormBlock from './FormBlock.svelte';
	import SendBlock from './SendBlock.svelte';
	import Updates from './Updates.svelte';
	import Made from './Made.svelte';
	import About from './About.svelte';
	import EmptyState from './EmptyState.svelte';
	import { numberSources, orderedItems, pagesPhrase, sourceCount } from '../cite.js';
	import { citations } from '../citations.svelte.js';
	import { changePanels } from '../guide.js';
	import { notConfirmedLabels } from '../present.js';
	import { SLOT, bySlot, isUS } from './items.js';
	import { repeatItems, sendRepeats } from './send.js';
	import { guide } from './lang.svelte.js';
	import type { ComponentProps } from 'svelte';
	import type { Speed } from '../fees.js';
	import type { Office, Provenance } from '../plan.js';
	import type { EvidenceNote } from './postal.js';
	import type { Item, Page } from './contract.js';

	// The customer guide (R2, plan §3): the whole page under one root,
	// built from `present()`'s Page. Bands, each a top-level section with
	// the page padding (py-10 xl:py-16): the header (with the service
	// status), "At a glance" with the route cards (profiled pages), the
	// guide — every step inside one article with a tighter rhythm, and an
	// "On this page" rail at 1440 — then the numbered sources (citations
	// on), "How this page was made" and "About this guide". An empty page
	// shows the empty state instead of the guide.
	//
	// Citations: every source is numbered once over the page's canonical
	// order, then the send step's repeats of other steps' values (each a
	// copy under its own id, doc/send.js), then the calculator's estimate
	// last (so neither ever renumbers the office's pages); markers, the
	// shared card, the endnotes and print all read that one numbering.
	// The card and the endnotes are a chunk of their own (cite-code.js),
	// requested the first time citations turn on: they are off by
	// default. When that request fails, an alert under the header says
	// so with a Retry button, and the next click or key press anywhere
	// asks again.
	//
	// Live updates: `flash` names Items that just changed (the page sets
	// it after a WS refetch); their rows light up for 1.5 s, and
	// `announce` is read out politely ("Updated: Opening hours").
	//
	// Props:
	// - page: the Page (present.js).
	// - name: the authority's name; slug, apiBase: its slug and the API
	//   root (`/apostille`).
	// - region, country: the authority row's (outline, flag, carriers).
	// - officeSet: accepted office records (plan.ts Office[]) or null.
	// - options: the `/p2/orders/options` body, or null.
	// - evidence: `/plan` `evidence[]`; forms: `/plan` `forms[]`.
	// - provenance, planVersion, planAcceptedAt: the accepted /plan's.
	// - models: model names that drafted the records.
	// - lang, translations, pathname: the records view's language
	//   (stored machine translations), not the guide's: the guide's own
	//   words follow the site language (lang.svelte.js).
	// - now: the clock for office status and closures.
	// - flash, announce: the live-update highlight and announcement.
	let {
		page,
		name = '',
		slug,
		apiBase = '',
		region = null,
		country = null,
		officeSet = null,
		options = null,
		evidence = null,
		forms = null,
		provenance = null,
		planVersion = 0,
		planAcceptedAt = null,
		models = [],
		lang = null,
		translations = {},
		pathname = '',
		now = new Date(),
		flash = [],
		announce = ''
	}: {
		page: Page;
		name?: string;
		slug: string;
		apiBase?: string;
		region?: string | null;
		country?: string | null;
		officeSet?: Office[] | null;
		options?: ComponentProps<typeof SendBlock>['options'];
		evidence?: EvidenceNote[] | null;
		forms?: ComponentProps<typeof FormBlock>['forms'];
		provenance?: Provenance | null;
		planVersion?: number;
		planAcceptedAt?: number | null;
		models?: string[];
		lang?: 'es' | 'fr' | null;
		translations?: { [id: number]: { statement: string; model: string } };
		pathname?: string;
		now?: Date;
		flash?: string[];
		announce?: string;
	} = $props();

	const t = $derived(guide.t);
	let column = $state<HTMLElement | null>(null);
	let article = $state<HTMLElement | null>(null);
	let docs = $state<number | null>(1);
	let speed = $state<Speed>('standard');
	let estimate = $state<Item | null>(null);
	let current = $state<string | null>(null);

	// The US postal rules (USPS branch without a street) on a US page,
	// profiled or not, as the presenter applies them to the glance.
	const us = $derived(isUS(country) || page.profile === 'us_state');
	const repeats = $derived(page.thin ? null : sendRepeats(page.sections));
	const cited = $derived(
		numberSources([...orderedItems(page), ...repeatItems(repeats), ...(estimate ? [estimate] : [])], guide.lang)
	);
	const count = $derived(sourceCount(cited.sources));
	const values = $derived(page.sections.flatMap((s) => [...s.items, ...s.also]));
	const missing = $derived(notConfirmedLabels(page));
	const fees = $derived(page.sections.find((s) => s.id === 'fees') ?? null);
	const fee = $derived(fees ? bySlot(fees.items, SLOT.perDocument) : null);
	const noticeActive = $derived(page.status.active.length > 0);
	// The glance's cost row points to the faster options in the fees step.
	const faster = $derived.by(() => {
		if (!fees) return null;
		const real = (i: Item) => i.factIds.length > 0 && i.state !== 'missing' && i.state !== 'absent';
		const any = fees.items.some((i) => (i.slot === SLOT.expedite || i.slot === SLOT.sameDay) && real(i));
		return any ? { step: fees.step } : null;
	});
	// "Get it back": mail takes time both ways when the page has a mail route.
	const afterNote = $derived(page.routes.some((r) => r.id === 'mail') ? t.guide.afterNote : null);
	// Who the label service is not.
	const issuer = $derived(/\bsecretary of state\b/i.test(name) ? t.guide.issuerSos : t.guide.issuerOffice);
	// "You can apply directly to …" only for an office that issues them.
	const issuing = $derived(page.profile === 'us_state' || !!page.header.lede);

	// The card and the endnotes, once citations are on (then kept). A
	// failed request says so (`citeFailed`) and asks again on Retry or on
	// the next click or key press.
	let citeCode = $state.raw<typeof import('./cite-code.js') | null>(null);
	let citeFailed = $state(false);
	let citeTry = $state(0);
	$effect(() => {
		void citeTry;
		if (!citations.on || citeCode) return;
		let live = true;
		import('./cite-code.js')
			.then((m) => {
				citeCode = m;
				citeFailed = false;
			})
			.catch(() => {
				if (live) citeFailed = true;
			});
		return () => {
			live = false;
		};
	});
	$effect(() => {
		if (!citeFailed || typeof window === 'undefined') return;
		const retry = () => {
			citeFailed = false;
			citeTry += 1;
		};
		window.addEventListener('pointerdown', retry, { once: true, capture: true });
		window.addEventListener('keydown', retry, { once: true, capture: true });
		return () => {
			window.removeEventListener('pointerdown', retry, { capture: true });
			window.removeEventListener('keydown', retry, { capture: true });
		};
	});

	const LABELS: Record<string, string> = $derived(t.guide.nav);
	// The office pages' change notes (`/changes`), read once and lazily:
	// when the end of the guide nears the screen. A 404, an empty or an
	// unreadable body reads as no notes (no console error of ours).
	// "Updates" shows only with something to list.
	let changes = $state<unknown>(undefined);
	let sentinel = $state<HTMLElement | null>(null);
	const panels = $derived(changes ? changePanels(changes) : []);
	const showUpdates = $derived(page.updates.length > 0 || panels.length > 0);

	const links = $derived([
		...page.sections.map((s) => ({ id: s.id, label: LABELS[s.id] ?? s.heading })),
		...(showUpdates ? [{ id: 'updates', label: LABELS.updates }] : [])
	]);

	$effect(() => {
		const el = sentinel;
		const current = slug;
		if (!el || !current || changes !== undefined) return;
		let cancelled = false;
		const load = () => {
			fetch(`${apiBase}/api/v1/authorities/${encodeURIComponent(current)}/changes`, { credentials: 'same-origin' })
				.then((res) => (res.ok ? res.json().catch(() => null) : null))
				.then((body) => {
					if (!cancelled) changes = body ?? null;
				})
				.catch(() => {
					if (!cancelled) changes = null;
				});
		};
		if (typeof IntersectionObserver === 'undefined') {
			load();
			return () => {
				cancelled = true;
			};
		}
		const io = new IntersectionObserver(
			(seen) => {
				if (!seen.some((e) => e.isIntersecting)) return;
				io.disconnect();
				load();
			},
			{ rootMargin: '800px 0px' }
		);
		io.observe(el);
		return () => {
			cancelled = true;
			io.disconnect();
		};
	});

	// The section in view, for the rail (never a sticky bar on phones).
	$effect(() => {
		const root = article;
		void links.length;
		if (!root || typeof IntersectionObserver === 'undefined') return;
		const spy = new IntersectionObserver(
			(entries) => {
				for (const e of entries) {
					if (e.isIntersecting && e.target instanceof HTMLElement && e.target.id) current = e.target.id;
				}
			},
			{ rootMargin: '-25% 0px -65% 0px' }
		);
		for (const el of root.querySelectorAll('section[id]')) spy.observe(el);
		return () => spy.disconnect();
	});

	// Live update: light the changed rows for 1.5 s.
	$effect(() => {
		const ids = flash;
		const root = column;
		if (!root || ids.length === 0) return;
		const lit: Element[] = [];
		for (const id of ids) {
			const safe = typeof CSS !== 'undefined' && CSS.escape ? CSS.escape(id) : id;
			for (const el of root.querySelectorAll(`[data-item="${safe}"], [data-conflict="${safe}"]`)) {
				el.setAttribute('data-flash', '');
				lit.push(el);
			}
		}
		const timer = setTimeout(() => {
			for (const el of lit) el.removeAttribute('data-flash');
		}, 1500);
		return () => {
			clearTimeout(timer);
			for (const el of lit) el.removeAttribute('data-flash');
		};
	});
</script>

{#snippet marker(item: Item)}<Cite refs={cited.refs} itemId={item.id} />{/snippet}
{#snippet inline(item: Item)}<Cite refs={cited.refs} itemId={item.id} reserve={false} />{/snippet}

<div bind:this={column} class="dossier-guide" data-guide>
	<PrintHeader title={page.header.h1} trust={page.header.trust} />
	<DocHeader
		header={page.header}
		{slug}
		{region}
		{country}
		cited={!page.empty && cited.sources.length > 0}
		{count}
	>
		{#snippet status()}<StatusBanner status={page.status} {marker} {inline} />{/snippet}
	</DocHeader>
	{#if citations.on && citeFailed && !citeCode}
		<div
			class="mb-6 flex max-w-prose flex-wrap items-center gap-x-3 gap-y-2 rounded-md border border-changed bg-changed-bg px-4 py-3 text-sm text-ink print:hidden"
			role="alert"
			data-cite-failed
		>
			<span>{t.guide.citeFailed}</span>
			<button
				type="button"
				class="inline-flex min-h-11 items-center rounded-sm border border-line bg-bg px-4 font-medium text-ink hover:border-focus"
				onclick={() => {
					citeFailed = false;
					citeTry += 1;
				}}>{t.guide.retry}</button
			>
		</div>
	{/if}

	{#if page.empty}
		<EmptyState empty={page.empty} />
		<About name={page.empty.name || name} issuer={page.profile === 'us_state'} {country} {slug} {apiBase} />
	{:else}
		{#if page.glance.length > 0}
			<section aria-labelledby="glance-h" class="border-t border-line py-10 xl:py-16" data-glance-band>
				<div class="max-w-3xl">
					<h2 id="glance-h" class="m-0 font-brand text-h2 font-medium text-ink">{t.glance.heading}</h2>
					<div class="mt-5">
						<Glance items={page.glance} {faster} {marker} {inline} />
					</div>
					<RouteCards routes={page.routes} />
				</div>
			</section>
		{/if}

		<section class="border-t border-line py-10 xl:py-16" data-guide-band>
			<div class="xl:grid xl:grid-cols-[minmax(0,1fr)_13rem] xl:gap-12">
				<article bind:this={article} class="min-w-0 max-w-3xl">
					{#if links.length >= 3}
						<div class="mb-10 xl:hidden">
							<Contents {links} {current} />
						</div>
					{/if}
					{#each page.sections as section (section.id)}
						{#if section.id === 'fees'}
							<GuideSection {section} {marker}>
								{#snippet body()}<FeeBlock
										{section}
										items={values}
										thin={page.thin}
										{marker}
										bind:docs
										bind:speed
										bind:estimate
									/>{/snippet}
							</GuideSection>
						{:else if section.id === 'form'}
							<GuideSection {section} {marker}>
								{#snippet body()}<FormBlock {section} {forms} {slug} {marker} />{/snippet}
							</GuideSection>
						{:else if section.id === 'send' && !page.thin}
							<GuideSection {section} {marker}>
								{#snippet body()}<SendBlock
										{section}
										items={values}
										offices={page.offices}
										closures={page.closures}
										{officeSet}
										{options}
										{evidence}
										{slug}
										{us}
										thin={page.thin}
										empty={!!page.empty}
										{noticeActive}
										{now}
										{country}
										{issuer}
										{repeats}
										{marker}
										{inline}
									/>{/snippet}
							</GuideSection>
						{:else}
							<GuideSection
								{section}
								{marker}
								missing={section.id === 'contact' ? missing : []}
								note={section.id === 'after' ? afterNote : null}
							/>
						{/if}
					{/each}
					{#if showUpdates}
						<section id="updates" aria-labelledby="updates-h" class="guide-step scroll-mt-20" data-section="updates">
							<h2 id="updates-h" class="m-0 font-brand text-h2 font-medium text-ink">{t.guide.updates}</h2>
							<div class="mt-5">
								<Updates items={page.updates} changes={changes ?? null} {slug} {apiBase} {marker} />
							</div>
						</section>
					{/if}
					<div bind:this={sentinel} aria-hidden="true"></div>
				</article>
				{#if links.length >= 3}
					<div class="hidden xl:block">
						<div class="sticky top-20">
							<Contents {links} {current} />
						</div>
					</div>
				{/if}
			</div>
		</section>

		{#if citeCode}<citeCode.Endnotes sources={cited.sources} />{/if}
		<Made
			records={page.records}
			{changes}
			{models}
			pages={pagesPhrase(page.header.trust, guide.lang)}
			{provenance}
			{planVersion}
			{planAcceptedAt}
			{slug}
			{apiBase}
			{lang}
			{translations}
			{pathname}
		/>
		<About name={name || page.header.h1} issuer={issuing} {country} {fee} {inline} {slug} {apiBase} />
	{/if}
	<p class="sr-only" role="status" aria-live="polite" aria-atomic="true">{announce}</p>
	{#if citeCode}<citeCode.CitationLayer sources={cited.sources} container={column} />{/if}
</div>

<style>
	/* Keyboard focus and in-page links stop below the sticky site
	   masthead (WCAG 2.4.11; plan §3 scroll-padding-top), going back up
	   the page as well as down. */
	:global(html:has([data-guide])) {
		scroll-padding-top: 4.5rem;
	}

	.dossier-guide :global([data-flash]) {
		background: var(--verified-bg);
		box-shadow: inset 3px 0 0 var(--verified);
		transition:
			background-color 300ms ease-out,
			box-shadow 300ms ease-out;
	}

	@media (prefers-reduced-motion: reduce) {
		.dossier-guide :global([data-flash]) {
			transition: none;
		}
	}

	/* Paper: the bands keep a thin rhythm, headings shrink to print sizes
	   (the print header above carries the title, URL and dates). */
	@media print {
		:global(html:has([data-print-header])) {
			font-size: 12px;
		}

		.dossier-guide > :global(section),
		.dossier-guide > :global(header) {
			padding-block: 6pt;
		}

		.dossier-guide :global(.max-w-3xl),
		.dossier-guide :global(.max-w-prose) {
			max-width: none;
		}

		.dossier-guide :global(h1) {
			font-size: 20pt;
			line-height: 1.15;
		}

		.dossier-guide :global(h2) {
			font-size: 14pt;
			line-height: 1.25;
		}

		.dossier-guide :global(h3) {
			font-size: 11.5pt;
		}

		.dossier-guide :global(.guide-step + .guide-step) {
			margin-top: 7pt;
			padding-top: 7pt;
		}

		/* Paper is dense (D8: at most 4 Letter pages, 8 with the sources):
		   body text at a print size (the screen's body is a fixed 16px, so
		   the smaller root above never reached it), and rows, gaps, cards
		   and the step badges tighten; the reading order and every value
		   stay. */
		.dossier-guide {
			font-size: 10pt;
			line-height: 1.3;
		}

		.dossier-guide > :global(header .mt-6),
		.dossier-guide > :global(header .mt-5),
		.dossier-guide :global(.guide-step > div),
		.dossier-guide :global([data-glance-band] .mt-5) {
			margin-top: 4pt;
		}

		.dossier-guide :global(.guide-step > h2 > span[aria-hidden='true']) {
			width: auto;
			height: auto;
			background: none;
			color: inherit;
			font-size: inherit;
		}

		.dossier-guide :global(dl > div[data-item]),
		.dossier-guide :global([data-glance]) {
			padding-block: 2pt;
		}

		/* The eyebrow repeats the letterhead. PrintHeader opens every
		   <details> before printing, so a summary prints as a plain line
		   without its 44 px target. */
		.dossier-guide > :global(header .text-eyebrow) {
			display: none;
		}

		.dossier-guide :global(details > summary) {
			min-height: 0;
		}

		/* A posting branch may run onto the next page between its blocks;
		   its address, conflict box and rows each stay whole. */
		.dossier-guide :global([data-route][data-print-keep]) {
			break-inside: auto;
		}

		.dossier-guide :global([data-glance] ul),
		.dossier-guide :global(.guide-step > div > div),
		.dossier-guide :global([data-route]),
		.dossier-guide :global(div:has(> [data-office])) {
			row-gap: 5pt;
		}

		.dossier-guide :global([data-office]),
		.dossier-guide :global([data-conflict]) {
			padding: 5pt 6pt;
		}

		.dossier-guide :global([data-office] .mt-3),
		.dossier-guide :global([data-conflict] .mt-3) {
			margin-top: 3pt;
		}

		.dossier-guide :global([data-conflict] .pt-3) {
			padding-top: 3pt;
		}

		.dossier-guide :global([data-conflict] li) {
			padding-block: 1.5pt;
		}

		.dossier-guide :global(#sources ol) {
			row-gap: 5pt;
			margin-top: 4pt;
		}

		.dossier-guide :global(#sources li) {
			line-height: 1.3;
		}

		.dossier-guide :global(#sources li > div) {
			margin-top: 2pt;
		}

		.dossier-guide :global(#sources a[role='doc-backlink']) {
			min-height: 0;
			min-width: 0;
		}
	}
</style>
