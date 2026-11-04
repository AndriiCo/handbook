<script lang="ts">
	import type { Snippet } from 'svelte';
	import { resolve } from '$app/paths';
	import FileDownIcon from '@lucide/svelte/icons/file-down';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import SquarePenIcon from '@lucide/svelte/icons/square-pen';
	import ValueRow from './ValueRow.svelte';
	import { SLOT, bySlot, formLink } from './items.js';
	import type { Item, Section } from './contract.js';

	// "Fill in the request form" (plan §3 step 3, D6): the section body.
	// The form's name as printed; one way to get it: "Download the
	// official form (PDF)" when a citation of the form is a PDF,
	// otherwise "Get the form on the office's page" (outbound). While the
	// office's pages word the form differently (or disagree), the link
	// comes from the cited values (`formLink`), so the step never dead
	// ends. "Fill it in online — free" only when a `/plan` `forms[]` row
	// says `fill_supported: true` (R3: an accepted, fillable form map
	// the fill-only action can fill without an order); then what to
	// enclose and every other row, as printed. In print the PDF address
	// prints after its link.
	//
	// Props:
	// - section: the form Section (`present()`), items and `also`.
	// - forms: `/plan` `forms[]` rows; optional.
	// - slug: the authority slug (order `destination`).
	// - marker: snippet `marker(item)` from the citations layer.
	type FormRow = { fill_supported?: unknown; status?: unknown };

	let {
		section,
		forms = [],
		slug,
		marker
	}: { section: Section; forms?: FormRow[] | null; slug: string; marker?: Snippet<[Item]> } = $props();

	const form = $derived(bySlot(section.items, SLOT.form));
	// What to enclose first, then the rest as the section lists them.
	const rest = $derived([
		...section.items.filter((i) => i !== form && i.slot === SLOT.enclose),
		...section.items.filter((i) => i !== form && i.slot !== SLOT.enclose),
		...section.also
	]);

	// The form's own address: a PDF citation first, else its first web
	// or upload page (the alternatives' while it is disputed).
	const link = $derived(formLink(form));
	const url = $derived(link?.url ?? null);
	const pdf = $derived(link?.pdf === true);
	const fillable = $derived(
		(Array.isArray(forms) ? forms : []).some(
			(f) => f && f.fill_supported === true && (f.status === undefined || f.status === 'active')
		)
	);
	const fillHref = $derived(
		`${resolve('/(customer)/order')}?${new URLSearchParams({ destination: slug, path: 'general', step: 'form' })}`
	);
</script>

<div class="flex flex-col gap-6">
	{#if form}
		<dl class="m-0 border-t border-line">
			<ValueRow item={form} {marker} />
		</dl>
	{/if}
	{#if url || fillable}
		<div class="flex flex-col gap-3">
			<div class="flex flex-wrap gap-3">
				{#if url && pdf}
					<a
						href={url}
						class="inline-flex min-h-11 items-center gap-2 rounded-sm bg-accent-fill px-5 font-medium text-accent-ink no-underline transition-colors hover:bg-accent-hover print:bg-transparent print:px-0 print:text-ink"
						data-form-link="pdf"
					>
						<FileDownIcon class="size-5 flex-none print:hidden" aria-hidden="true" />
						Download the official form (PDF)
					</a>
				{:else if url}
					<a
						href={url}
						target="_blank"
						rel="noopener noreferrer"
						class="inline-flex min-h-11 items-center gap-2 rounded-sm border border-line bg-bg px-5 font-medium text-ink no-underline transition-colors hover:border-focus"
						data-form-link="page"
					>
						Get the form on the office's page
						<ExternalLinkIcon class="size-4 flex-none" aria-hidden="true" />
						<span class="sr-only">(opens in a new tab)</span>
					</a>
				{/if}
				{#if fillable}
					<a
						href={fillHref}
						class="inline-flex min-h-11 items-center gap-2 rounded-sm border border-line bg-bg px-5 font-medium text-ink no-underline transition-colors hover:border-focus print:hidden"
						data-form-link="fill"
					>
						<SquarePenIcon class="size-5 flex-none text-accent-strong" aria-hidden="true" />
						Fill it in online — free
					</a>
				{/if}
			</div>
			{#if url && pdf}
				<p class="m-0 hidden break-all font-mono text-sm text-ink print:block">{url}</p>
			{/if}
			{#if fillable}
				<p class="m-0 max-w-prose text-sm text-ink-muted print:hidden">
					We put your answers into the office's own form; you print, sign and mail it. Needs a free account.
				</p>
			{/if}
		</div>
	{/if}
	{#if rest.length > 0}
		<dl class="m-0 border-t border-line">
			{#each rest as item (item.id)}
				<ValueRow {item} {marker} />
			{/each}
		</dl>
	{/if}
</div>
