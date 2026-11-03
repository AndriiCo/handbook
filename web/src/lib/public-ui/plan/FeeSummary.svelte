<script lang="ts">
	import MinusIcon from '@lucide/svelte/icons/minus';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import CitationChips from './CitationChips.svelte';
	import { feeTotal, lineCurrency, money, type FeeLine, type PlanFact } from '../plan.js';

	// Fee lines exactly as the accepted plan lists them (each cites its
	// fact); per-document lines scale with the document count, so the
	// total is arithmetic on cited amounts, never a quoted estimate.
	let {
		lines = [],
		facts,
		factIds = [],
		docs = $bindable(1)
	}: {
		lines?: FeeLine[];
		facts: Map<number, PlanFact>;
		factIds?: number[];
		docs?: number;
	} = $props();

	const currency = $derived(
		lines.map((l) => lineCurrency(l, l.fact_id !== undefined ? facts.get(l.fact_id) : undefined)).find(Boolean) ??
			null
	);
	const perDoc = $derived(lines.some((l) => l.per === 'document'));
	const total = $derived(feeTotal(lines, docs));
</script>

<div class="rounded-xl border border-line bg-bg p-5 shadow-sm">
	<table class="w-full border-collapse text-sm">
		<caption class="sr-only">Fees</caption>
		<tbody>
			{#each lines as line, i (i)}
				<tr class="border-b border-line">
					<th scope="row" class="py-2 pr-3 text-left font-medium text-ink">
						{line.label}
						{#if line.per}<span class="block text-xs font-normal text-ink-muted">per {line.per}</span>{/if}
					</th>
					<td class="py-2 text-right font-mono text-ink">{money(line.amount_cents, currency)}</td>
				</tr>
			{/each}
		</tbody>
	</table>
	<div class="mt-4 flex flex-wrap items-end justify-between gap-4">
		{#if perDoc}
			<div>
				<p class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase" id="docs-label">Documents</p>
				<div class="mt-1 inline-flex items-center rounded-full border border-line" role="group" aria-labelledby="docs-label">
					<button
						type="button"
						class="inline-flex size-11 cursor-pointer items-center justify-center rounded-full text-ink transition-colors hover:bg-surface disabled:cursor-not-allowed disabled:opacity-40"
						aria-label="One document fewer"
						disabled={docs <= 1}
						onclick={() => (docs = Math.max(1, docs - 1))}
					>
						<MinusIcon class="size-4" aria-hidden="true" />
					</button>
					<output class="w-8 text-center font-mono text-sm text-ink" aria-live="polite">{docs}</output>
					<button
						type="button"
						class="inline-flex size-11 cursor-pointer items-center justify-center rounded-full text-ink transition-colors hover:bg-surface disabled:cursor-not-allowed disabled:opacity-40"
						aria-label="One document more"
						disabled={docs >= 20}
						onclick={() => (docs = Math.min(20, docs + 1))}
					>
						<PlusIcon class="size-4" aria-hidden="true" />
					</button>
				</div>
			</div>
		{/if}
		<p class="m-0 ml-auto text-right">
			<span class="block text-xs font-semibold tracking-wide text-ink-muted uppercase">Total</span>
			<span class="font-brand text-h2 font-medium text-ink" data-fee-total>{money(total, currency)}</span>
		</p>
	</div>
	<div class="mt-4">
		<CitationChips ids={[...factIds, ...lines.map((l) => l.fact_id)]} {facts} />
	</div>
</div>
