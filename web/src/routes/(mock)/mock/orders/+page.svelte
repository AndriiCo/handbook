<script lang="ts">
	import { resolve } from '$app/paths';
	import { AUTHORITIES, ORDERS } from '../../../../lib/mock/fixtures';

	type Filter = 'All' | 'Draft' | 'Submitted' | 'In review' | 'Ready to file' | 'Completed';
	let filter = $state<Filter>('All');

	const visible = $derived(filter === 'All' ? ORDERS : ORDERS.filter((o) => o.status === filter));
	const filters: Filter[] = ['All', 'Draft', 'Submitted', 'In review', 'Ready to file', 'Completed'];
	// Full class strings per state: the active border/weight must
	// never co-occur with the idle ones in the cascade.
	const filterCls = (isActive: boolean) =>
		`min-h-11 cursor-pointer rounded-sm border bg-surface px-[0.9rem] text-ink [font:inherit] ${
			isActive ? 'border-accent font-bold' : 'border-line'
		}`;
</script>

<svelte:head>
	<title>Mock orders — Apostille Handbook</title>
</svelte:head>

<h1>Orders</h1>
<p><a href={resolve('/(mock)/mock/order')}>Start a new order</a></p>

<div class="my-4 flex flex-wrap gap-[0.4rem]" role="group" aria-label="Filter by status">
	{#each filters as f (f)}
		<button class={filterCls(filter === f)} aria-pressed={filter === f} onclick={() => (filter = f)}>
			{f}
		</button>
	{/each}
</div>

{#if visible.length === 0}
	<p role="status">No {filter} orders in the fixture set.</p>
{:else}
	<table class="mt-4 block w-full border-collapse md:table">
		<caption class="pb-2 text-left text-sm text-ink-muted">Fixture orders ({visible.length})</caption>
		<thead class="sr-only md:not-sr-only">
			<tr>
				<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
					Order
				</th>
				<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
					Authority
				</th>
				<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
					Document
				</th>
				<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
					Status
				</th>
				<th class="min-h-11 border border-line bg-surface px-[0.8rem] py-[0.6rem] text-left" scope="col">
					Updated
				</th>
			</tr>
		</thead>
		<tbody class="block w-full md:table-row-group">
			{#each visible as row (row.id)}
				<tr class="mb-3 block rounded-sm border border-line even:bg-surface md:mb-0 md:table-row md:rounded-none md:border-0">
					<td
						data-label="Order"
						class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left font-mono text-[0.9rem] last:border-b-0 md:table-cell md:border"
						>{row.id}</td
					>
					<td
						data-label="Authority"
						class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
						>{row.authority}</td
					>
					<td
						data-label="Document"
						class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
						>{row.docType}</td
					>
					<td
						data-label="Status"
						class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left last:border-b-0 md:table-cell md:border"
						><span class="inline-block rounded-xs border border-line bg-surface-2 px-2 py-[0.1rem] text-sm">
							{row.status}</span
						></td
					>
					<td
						data-label="Updated"
						class="block min-h-11 border-0 border-b border-line px-[0.8rem] py-[0.6rem] text-left font-mono text-[0.9rem] last:border-b-0 md:table-cell md:border"
						>{row.updated}</td
					>
				</tr>
			{/each}
		</tbody>
	</table>
{/if}

<p class="text-[0.8rem] text-ink-muted">
	Destinations in this mock: {AUTHORITIES.map((a) => a.name).join(' · ')}
</p>

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
