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

<table>
	<caption>Layout preview — placeholder rows, not real orders</caption>
	<thead>
		<tr>
			<th scope="col">Order</th>
			<th scope="col">Authority</th>
			<th scope="col">Status</th>
		</tr>
	</thead>
	<tbody>
		{#each preview as row (row.id)}
			<tr>
				<td data-label="Order" class="mono">{row.id}</td>
				<td data-label="Authority">{row.authority}</td>
				<td data-label="Status">{row.status}</td>
			</tr>
		{/each}
	</tbody>
</table>

<style>
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
