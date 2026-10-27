<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { TRIAGE } from '../../../../lib/mock/fixtures';

	const awaiting = $derived(TRIAGE.length);
	const conflicts = $derived(TRIAGE.filter((t) => t.conflict).length);
</script>

<svelte:head>
	<title>Mock admin dashboard — Apostille Handbook</title>
</svelte:head>

<h1>Admin dashboard</h1>

<Notice variant="warning" heading="Fixture queues.">
	<p>Counts below come from the mock fixture set, not live data.</p>
</Notice>

<ul class="ledger">
	<li>
		<a href={resolve('/(mock)/mock/admin/triage')}>Fact / task triage</a>
		<span class="kind">{awaiting} awaiting review ({conflicts} with conflict)</span>
	</li>
	<li>
		<a href={resolve('/(mock)/mock/admin/console')}>Call console</a>
		<span class="kind">1 demo transcript loaded</span>
	</li>
	<li>
		<a href={resolve('/(mock)/mock/admin/config')}>Configuration</a>
		<span class="kind">typed keys, secrets write-only</span>
	</li>
</ul>

<style>
	.ledger {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0 0 2rem;
	}
	.ledger li {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.25rem 0.75rem;
		padding: 0.7rem 0;
		border-bottom: 1px solid var(--border);
	}
	.ledger a {
		font-weight: 700;
		min-height: 44px;
		display: inline-flex;
		align-items: center;
	}
	.kind {
		color: var(--text-muted);
		font-size: 0.875rem;
	}
</style>
