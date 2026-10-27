<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
</script>

<svelte:head>
	<title>Admin dashboard — Apostille Handbook</title>
</svelte:head>

<h1>Admin dashboard</h1>

<Notice variant="warning" heading="Thin slice.">
	<p>
		Queues below are shells until the approval and job lanes land.
		Counts are placeholders, not live data.
	</p>
</Notice>

<ul class="ledger">
	<li>
		<a href={resolve('/(admin)/admin/triage')}>Fact / task triage</a>
		<span class="kind">0 awaiting review (placeholder)</span>
	</li>
	<li>
		<a href={resolve('/(admin)/admin/console')}>Call console</a>
		<span class="kind">no active calls (placeholder)</span>
	</li>
	<li>
		<a href={resolve('/(admin)/admin/config')}>Configuration</a>
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
