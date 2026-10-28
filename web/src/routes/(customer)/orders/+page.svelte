<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';

	// Orders list shell. No orders endpoint exists yet, so this is an
	// honest empty state + layout preview, like billing.
	const preview = [
		{ id: 'ORD-0001 (placeholder)', authority: 'Placeholder Authority One', status: 'Draft' }
	];
</script>

<svelte:head>
	<title>Orders — Apostille Handbook</title>
</svelte:head>

<h1>Orders</h1>

<Notice variant="info" heading="No orders yet.">
	<p>
		Submitted orders will appear here with live status.
		<a href={resolve('/(customer)/order')}>Start a new order</a>.
	</p>
</Notice>

<table class="mt-4 block w-full border-collapse md:table">
	<caption class="pb-2 text-left text-sm text-ink-muted">
		Layout preview — placeholder rows, not real orders
	</caption>
	<thead class="sr-only md:not-sr-only">
		<tr>
			<th class="border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">Order</th>
			<th class="border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
				Authority
			</th>
			<th class="border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">Status</th>
		</tr>
	</thead>
	<tbody class="block w-full md:table-row-group">
		{#each preview as row (row.id)}
			<tr class="mb-3 block rounded-sm border border-line even:bg-surface md:mb-0 md:table-row md:rounded-none md:border-0">
				<td
					data-label="Order"
					class="block border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left font-mono text-[0.9rem] last:border-b-0 md:table-cell md:border"
					>{row.id}</td
				>
				<td
					data-label="Authority"
					class="block border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
					>{row.authority}</td
				>
				<td
					data-label="Status"
					class="block border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
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
		font-family: var(--font-body);
	}
	@media (min-width: 720px) {
		td::before {
			content: none;
		}
	}
</style>
