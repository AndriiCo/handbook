<script lang="ts">
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';

	let locale = $state<Locale>(resolveLocale());

	const t = $derived(STRINGS[locale]);
	const preview = [
		{ id: 'INV-0001 (placeholder)', date: '2026-01-01', amount: 'USD $0.00' },
		{ id: 'INV-0002 (placeholder)', date: '2026-01-02', amount: 'USD $0.00' }
	];
</script>

<svelte:head>
	<title>{t.billingLabel} — Apostille Handbook</title>
</svelte:head>

<h1>{t.billingLabel}</h1>

<p class="rounded-sm border border-line bg-surface px-4 py-3" role="status">
	No invoices yet. Real invoices will appear here once billing is wired.
</p>

<table class="mt-4 block w-full border-collapse md:table">
	<caption class="pb-2 text-left text-sm text-ink-muted">
		Layout preview — placeholder rows, not real invoices
	</caption>
	<thead class="sr-only md:not-sr-only">
		<tr>
			<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
				Invoice
			</th>
			<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
				Date
			</th>
			<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
				Amount
			</th>
		</tr>
	</thead>
	<tbody class="block w-full md:table-row-group">
		{#each preview as row (row.id)}
			<tr class="mb-3 block rounded-sm border border-line even:bg-surface md:mb-0 md:table-row md:rounded-none md:border-0">
				<td
					data-label="Invoice"
					class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left font-mono text-[0.9rem] last:border-b-0 md:table-cell md:border"
					>{row.id}</td
				>
				<td
					data-label="Date"
					class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left font-mono text-[0.9rem] last:border-b-0 md:table-cell md:border"
					>{row.date}</td
				>
				<td
					data-label="Amount"
					class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left font-mono text-[0.9rem] last:border-b-0 md:table-cell md:border"
					>{row.amount}</td
				>
			</tr>
		{/each}
	</tbody>
</table>

<style>
	/* Mobile-only label injection has no clean utility (attr() +
	   string concat); the desktop kill inverts the old max-width
	   query mobile-first. */
	td::before {
		content: attr(data-label) ': ';
		font-weight: 700;
		font-family: var(--font-sans);
	}
	@media (min-width: 720px) {
		td::before {
			content: none;
		}
	}
</style>
