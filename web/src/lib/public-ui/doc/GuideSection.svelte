<script lang="ts">
	import type { Snippet } from 'svelte';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import CircleQuestionMarkIcon from '@lucide/svelte/icons/circle-question-mark';
	import PhoneIcon from '@lucide/svelte/icons/phone';
	import MailIcon from '@lucide/svelte/icons/mail';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import ValueRow from './ValueRow.svelte';
	import { telHref } from './postal.js';
	import { traceAttrs, usable } from './items.js';
	import type { Item, Section } from './contract.js';

	// One step of the guide (plan §3 rows 3–9): a region named by its h2
	// ("Pay the fees"), the step number in front on profiled pages. The
	// body is the block a page passes in (fees, form, send), or the rows
	// themselves: every value through ValueRow, the issuing country's
	// own-use rule as warning text, a phone with a call link, an email
	// with a mail link, a cited page with its link; then the rows the
	// registry doesn't name, under "Also on the office's pages". Hops of
	// the legalization chain (kind `chain`) read as a numbered list, in
	// order. The contact step can close with the "Not confirmed yet" list.
	//
	// Props:
	// - section: a `page.sections` entry (present.js).
	// - marker: snippet `marker(item)` from the citations layer.
	// - body: snippet replacing the rows (FeeBlock, FormBlock, SendBlock).
	// - missing: labels of rows that read "Not confirmed yet" (contact).
	// - note: one line of advice under the rows (template copy, e.g.
	//   "Allow extra time for mail both ways."), or null.
	let {
		section,
		marker,
		body,
		missing = [],
		note = null
	}: { section: Section; marker?: Snippet<[Item]>; body?: Snippet; missing?: string[]; note?: string | null } = $props();

	const headingId = $derived(`${section.id}-h`);
	const WARNING_SLOT = 'authentication.domestic_validity';

	const warnings = $derived(section.items.filter((i) => i.slot === WARNING_SLOT && i.state !== 'missing'));
	const chain = $derived(section.items.filter((i) => i.kind === 'chain'));
	const rows = $derived(section.items.filter((i) => !warnings.includes(i) && !chain.includes(i)));

	/** The first https page a value cites (for link rows). */
	function pageLink(item: Item): string | null {
		if (!usable(item)) return null;
		for (const c of item.cites) {
			const s = c.source;
			if (s.kind && !['web', 'pdf', 'upload'].includes(s.kind)) continue;
			if (typeof s.url === 'string' && s.url.startsWith('https://')) return s.url.split('#')[0];
		}
		return null;
	}

	function emailIn(text: string): string | null {
		const m = /[^\s@<>()",;]+@[^\s@<>()",;]+\.[a-z]{2,}/i.exec(text);
		return m ? m[0] : null;
	}
</script>

{#snippet linkFor(item: Item)}
	{#if item.kind === 'phone' && usable(item) && telHref(item.display)}
		<a
			href={telHref(item.display)}
			class="mt-2 inline-flex min-h-8 items-center gap-1.5 rounded-sm border border-line px-2.5 text-sm font-medium text-ink no-underline transition-colors hover:border-focus hover:bg-surface print:hidden"
		>
			<PhoneIcon class="size-4 text-ink-muted" aria-hidden="true" />Call this number
		</a>
	{:else if item.kind === 'email' && usable(item) && emailIn(item.display)}
		<a
			href={`mailto:${emailIn(item.display)}`}
			class="mt-2 inline-flex min-h-8 items-center gap-1.5 rounded-sm border border-line px-2.5 text-sm font-medium text-ink no-underline transition-colors hover:border-focus hover:bg-surface print:hidden"
		>
			<MailIcon class="size-4 text-ink-muted" aria-hidden="true" />Write an email
		</a>
	{:else if item.kind === 'link_from_citation' && pageLink(item)}
		<a
			href={pageLink(item)}
			target="_blank"
			rel="noopener noreferrer"
			class="mt-2 inline-flex min-h-8 items-center gap-1.5 font-medium text-accent-strong underline underline-offset-4"
			data-print-url={item.slot === 'website.verify' ? '' : undefined}
		>
			Open the office's page<ExternalLinkIcon class="size-4 flex-none" aria-hidden="true" /><span class="sr-only">
				(opens in a new tab)</span
			>
		</a>
	{/if}
{/snippet}

{#snippet rowOf(item: Item)}
	{#if item.kind === 'phone' || item.kind === 'email' || item.kind === 'link_from_citation'}
		<ValueRow {item} {marker}>{#snippet extra()}{@render linkFor(item)}{/snippet}</ValueRow>
	{:else}
		<ValueRow {item} {marker} />
	{/if}
{/snippet}

<section id={section.id} aria-labelledby={headingId} class="guide-step scroll-mt-20" data-section={section.id}>
	<h2 id={headingId} class="m-0 flex items-center gap-3 font-brand text-h2 font-medium text-ink">
		{#if section.step !== null}
			<span
				class="inline-flex size-9 flex-none items-center justify-center rounded-full bg-accent-fill font-sans text-body font-semibold text-accent-ink"
				aria-hidden="true">{section.step}</span
			><span class="sr-only">Step {section.step}:&nbsp;</span>
		{/if}
		<span class="text-balance">{section.heading}</span>
	</h2>
	<div class="mt-5">
		{#if body}
			{@render body()}
		{:else}
			{#each warnings as item (item.id)}
				<div class="mb-4 rounded-md border-l-4 border-changed bg-changed-bg px-4 py-1" data-print-keep>
					<p class="m-0 mt-2 flex items-center gap-2 text-sm font-semibold text-ink">
						<TriangleAlertIcon class="size-4 flex-none text-changed" aria-hidden="true" />
						Before you apply
					</p>
					<dl class="m-0">
						<ValueRow {item} {marker} emphasis />
					</dl>
				</div>
			{/each}
			{#if rows.length > 0}
				<dl class="m-0 border-t border-line">
					{#each rows as item (item.id)}{@render rowOf(item)}{/each}
				</dl>
			{/if}
			{#if chain.length > 0}
				<div class={rows.length > 0 || warnings.length > 0 ? 'mt-8' : ''} data-chain>
					<h3 class="text-h3 m-0 font-semibold text-ink">Legalization chain</h3>
					<p class="mt-1 mb-0 max-w-prose text-sm text-ink-muted">
						Each hop below comes from the office's own pages, in order.
					</p>
					<ol class="m-0 mt-4 list-none p-0">
						{#each chain as hop, i (hop.id)}
							<li
								class="m-0 grid grid-cols-[2.25rem_minmax(0,1fr)_minmax(3.5ch,max-content)] gap-x-3 pb-5 last:pb-0"
								data-item={hop.id}
							>
								<span
									class="flex size-9 items-center justify-center rounded-full border border-line bg-bg font-mono text-sm font-semibold text-ink"
									aria-hidden="true">{i + 1}</span
								>
								<span class="min-w-0 pt-1.5 break-words text-ink [overflow-wrap:anywhere]" {...traceAttrs(hop)}
									>{hop.display}</span
								>
								<span class="pt-1.5 text-right text-sm leading-6">{@render marker?.(hop)}</span>
							</li>
						{/each}
					</ol>
				</div>
			{/if}
			{#if section.also.length > 0}
				{#if section.items.length > 0}
					<h3 class="text-h3 mt-8 mb-0 font-semibold text-ink">Also on the office's pages</h3>
				{/if}
				<dl class={`m-0 border-t border-line ${section.items.length > 0 ? 'mt-3' : ''}`}>
					{#each section.also as item (item.id)}{@render rowOf(item)}{/each}
				</dl>
			{/if}
		{/if}
		{#if note}
			<p class="mt-4 mb-0 max-w-prose text-ink" data-note>{note}</p>
		{/if}
		{#if missing.length > 0}
			<div class="mt-6 rounded-md border border-line bg-surface p-4 sm:p-5" data-not-confirmed>
				<h3 class="m-0 flex items-center gap-2 text-body font-semibold text-ink">
					<CircleQuestionMarkIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
					Not confirmed yet
				</h3>
				<p class="m-0 mt-1 text-sm text-ink-muted">
					The office's pages don't say. Ask the office before you send documents or money:
				</p>
				<ul class="m-0 mt-2 list-disc pl-5 text-ink">
					{#each missing as label (label)}<li class="m-0 mt-1">{label}</li>{/each}
				</ul>
			</div>
		{/if}
	</div>
</section>

<style>
	/* Steps share one band: a rule and a tighter rhythm between them. */
	.guide-step + :global(.guide-step) {
		border-top: 1px solid var(--border);
		margin-top: 2.5rem;
		padding-top: 2.5rem;
	}
</style>
