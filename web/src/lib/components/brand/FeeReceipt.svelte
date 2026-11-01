<script lang="ts">
	import { parseFee } from '../../public-ui/authority-meta.js';

	// Fee receipt promoted from the mock flow: one line per published fee
	// fact (topic `fees`), amounts parsed from the statements, mono line
	// items, one large Vogun total. No fee facts → an honest empty state,
	// never invented numbers.
	let { facts = [] }: { facts?: any[] } = $props();

	const lines = $derived(
		(facts ?? [])
			.filter((f) => f?.topic === 'fees')
			.map((f) => ({ fact: f, fee: parseFee(f?.statement) }))
			.filter((l) => l.fee !== null)
	);

	function cents(amount: string): number {
		return Math.round(parseFloat(amount.replace(/[$,]/g, '')) * 100);
	}

	const total = $derived.by(() => {
		if (lines.length === 0) return null;
		const amounts = lines.map((l) => l.fee!.amount);
		if (amounts.length === 1) return amounts[0];
		const values = amounts.map(cents);
		const min = Math.min(...values);
		return `from $${(min / 100).toFixed(min % 100 === 0 ? 0 : 2)}`;
	});
</script>

<div class="rounded-lg border border-line bg-surface p-4" aria-label="Fee receipt">
	<p class="m-0 text-sm font-semibold text-ink">Fee receipt</p>
	{#if lines.length === 0}
		<p class="mt-2 mb-0 text-sm text-ink-muted">
			No fees published for this destination yet — they appear here once staff approve them.
		</p>
	{:else}
		<dl class="m-0 mt-2 flex flex-col gap-1">
			{#each lines as l (l.fact.id)}
				<div class="flex items-baseline justify-between gap-3">
					<dt class="font-mono text-xs text-ink-muted">{l.fee!.label}</dt>
					<dd class="m-0 font-mono text-sm text-ink">{l.fee!.amount}</dd>
				</div>
			{/each}
		</dl>
		<p class="mt-3 mb-0 flex items-baseline justify-between gap-3 border-t border-line pt-3">
			<span class="text-sm font-medium text-ink-muted">Published total</span>
			<span class="font-brand text-h2 font-medium text-ink">{total}</span>
		</p>
	{/if}
</div>
