<script lang="ts">
	import { resolve } from '$app/paths';
	import { AUTHORITIES, ORDERS } from '../../../../lib/mock/fixtures';

	type Filter = 'All' | 'Draft' | 'Submitted' | 'In review' | 'Ready to file' | 'Completed';
	let filter = $state<Filter>('All');

	const visible = $derived(filter === 'All' ? ORDERS : ORDERS.filter((o) => o.status === filter));
	const filters: Filter[] = ['All', 'Draft', 'Submitted', 'In review', 'Ready to file', 'Completed'];
</script>

<svelte:head>
	<title>Mock orders — Apostille Handbook</title>
</svelte:head>

<h1>Orders</h1>
<p><a href={resolve('/(mock)/mock/order')}>Start a new order</a></p>

<div class="filters" role="group" aria-label="Filter by status">
	{#each filters as f (f)}
		<button class:active={filter === f} aria-pressed={filter === f} onclick={() => (filter = f)}>
			{f}
		</button>
	{/each}
</div>

{#if visible.length === 0}
	<p role="status">No {filter} orders in the fixture set.</p>
{:else}
	<table>
		<caption>Fixture orders ({visible.length})</caption>
		<thead>
			<tr>
				<th scope="col">Order</th>
				<th scope="col">Authority</th>
				<th scope="col">Document</th>
				<th scope="col">Status</th>
				<th scope="col">Updated</th>
			</tr>
		</thead>
		<tbody>
			{#each visible as row (row.id)}
				<tr>
					<td data-label="Order" class="mono">{row.id}</td>
					<td data-label="Authority">{row.authority}</td>
					<td data-label="Document">{row.docType}</td>
					<td data-label="Status"><span class="pill">{row.status}</span></td>
					<td data-label="Updated" class="mono">{row.updated}</td>
				</tr>
			{/each}
		</tbody>
	</table>
{/if}

<p class="fine">Destinations in this mock: {AUTHORITIES.map((a) => a.name).join(' · ')}</p>

<style>
	.filters {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
		margin: 1rem 0;
	}
	.filters button {
		min-height: 44px;
		font: inherit;
		background: var(--surface);
		color: var(--text);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		cursor: pointer;
		padding: 0 0.9rem;
	}
	.filters button.active {
		font-weight: 700;
		border-color: var(--accent);
	}
	.pill {
		display: inline-block;
		background: var(--surface-2);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
		padding: 0.1rem 0.5rem;
		font-size: 0.875rem;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		margin-top: 1rem;
	}
	caption {
		text-align: left;
		color: var(--text-muted);
		font-size: 0.875rem;
		padding-bottom: 0.5rem;
	}
	th,
	td {
		border: 1px solid var(--border);
		padding: 0.6rem 0.8rem;
		text-align: left;
		min-height: 44px;
	}
	thead th {
		background: var(--surface);
	}
	tbody tr:nth-child(even) {
		background: var(--surface);
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 0.9rem;
	}
	.fine {
		font-size: 0.8rem;
		color: var(--text-muted);
	}
	@media (max-width: 719px) {
		thead {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip: rect(0 0 0 0);
		}
		table,
		tbody,
		tr,
		td {
			display: block;
			width: 100%;
		}
		tr {
			margin-bottom: 0.75rem;
			border: 1px solid var(--border);
			border-radius: var(--radius-sm);
		}
		td {
			border: none;
			border-bottom: 1px solid var(--border);
		}
		td:last-child {
			border-bottom: none;
		}
		td::before {
			content: attr(data-label) ': ';
			font-weight: 700;
			font-family: var(--font-body);
		}
	}
</style>
