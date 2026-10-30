<script lang="ts">
	import { INVOICES } from '../../../../lib/mock/fixtures';
</script>

<svelte:head>
	<title>Mock billing — Apostille Handbook</title>
</svelte:head>

<h1>Billing</h1>

<table class="mt-4 block w-full border-collapse md:table">
	<caption class="pb-2 text-left text-sm text-ink-muted">Fixture invoices ({INVOICES.length})</caption>
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
			<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
				Status
			</th>
		</tr>
	</thead>
	<tbody class="block w-full md:table-row-group">
		{#each INVOICES as row (row.id)}
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
				<td
					data-label="Status"
					class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
					>{row.status}</td
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
