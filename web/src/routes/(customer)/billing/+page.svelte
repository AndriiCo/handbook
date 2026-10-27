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

<p class="notice" role="status">No invoices yet. Real invoices will appear here once billing is wired.</p>

<table>
	<caption>Layout preview — placeholder rows, not real invoices</caption>
	<thead>
		<tr>
			<th scope="col">Invoice</th>
			<th scope="col">Date</th>
			<th scope="col">Amount</th>
		</tr>
	</thead>
	<tbody>
		{#each preview as row (row.id)}
			<tr>
				<td data-label="Invoice" class="mono">{row.id}</td>
				<td data-label="Date" class="mono">{row.date}</td>
				<td data-label="Amount" class="mono">{row.amount}</td>
			</tr>
		{/each}
	</tbody>
</table>

<style>
	.notice {
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
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
