<script lang="ts">
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import CitationChips from './CitationChips.svelte';
	import type { PlanFact, PlanForm } from '../plan.js';

	// Forms the accepted plan names: the official PDF (outbound) and,
	// for a staff-accepted field map, the guided online fill.
	let {
		forms = [],
		facts,
		factIds = [],
		fillHref
	}: {
		forms?: PlanForm[];
		facts: Map<number, PlanFact>;
		factIds?: number[];
		fillHref: (form: PlanForm) => string;
	} = $props();

	const official = (url: unknown) => (typeof url === 'string' && url.startsWith('https://') ? url : null);
</script>

<ul class="m-0 flex list-none flex-col gap-3 p-0">
	{#each forms as form (form.id)}
		{@const pdf = official(form.official_url)}
		<li class="m-0 rounded-xl border border-line bg-bg p-5 shadow-sm">
			<div class="flex items-start gap-3">
				<span
					class="flex size-10 flex-none items-center justify-center rounded-lg bg-surface text-accent-strong"
					aria-hidden="true"
				>
					<FileTextIcon class="size-5" />
				</span>
				<div class="min-w-0 flex-1">
					<p class="m-0 font-semibold break-words text-ink">{form.title}</p>
					{#if Array.isArray(form.fields) && form.fields.length > 0}
						<p class="mt-0.5 mb-0 text-xs text-ink-muted">
							{form.fields.length} field{form.fields.length === 1 ? '' : 's'} mapped from the official PDF
						</p>
					{/if}
				</div>
			</div>
			<div class="mt-4 flex flex-wrap gap-2">
				{#if form.fillable}
					<a
						href={fillHref(form)}
						class="inline-flex min-h-11 items-center rounded-sm bg-accent-fill px-5 text-sm font-medium text-accent-ink no-underline transition-colors hover:bg-accent-hover"
					>
						Fill it online
					</a>
				{/if}
				{#if pdf}
					<a
						href={pdf}
						target="_blank"
						rel="noopener noreferrer"
						class="inline-flex min-h-11 items-center gap-1.5 rounded-sm border border-line bg-bg px-5 text-sm font-medium text-ink no-underline transition-colors hover:border-focus"
					>
						Official form
						<ExternalLinkIcon class="size-4" aria-hidden="true" />
						<span class="sr-only">(opens in a new tab)</span>
					</a>
				{/if}
			</div>
			<div class="mt-4">
				<CitationChips ids={[...factIds, form.source_fact_id]} {facts} />
			</div>
		</li>
	{/each}
</ul>
