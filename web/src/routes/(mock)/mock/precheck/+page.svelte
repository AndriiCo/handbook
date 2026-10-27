<script lang="ts">
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { PRECHECK } from '../../../../lib/mock/fixtures';

	// Simulated live states: the toggle walks the same four beats the
	// real page shows (connecting → live → reconnecting → offline),
	// with fixture check rows below.
	type Live = 'connecting' | 'live' | 'reconnecting' | 'offline';
	const order: Live[] = ['connecting', 'live', 'reconnecting', 'offline'];
	let live = $state<Live>('live');
	let step = $state(1);

	function cycle() {
		step = (step + 1) % order.length;
		live = order[step];
	}

	const fails = $derived(PRECHECK.filter((c) => c.state === 'fail').length);
	const warns = $derived(PRECHECK.filter((c) => c.state === 'warn').length);
</script>

<svelte:head>
	<title>Mock precheck — Apostille Handbook</title>
</svelte:head>

<h1>Document precheck</h1>

<p class="live" role="status">
	Connection: <strong>{live}</strong>
	<button class="link" onclick={cycle}>Simulate next state</button>
</p>

{#if fails > 0}
	<Notice variant="error" heading="1 check needs a retake.">
		<p>Fix the failed item below, then re-upload. Nothing leaves your browser in this mock.</p>
	</Notice>
{:else if warns > 0}
	<Notice variant="warning" heading="Checks pass with warnings.">
		<p>You can file as-is, but a daylight retake would strengthen the seal shot.</p>
	</Notice>
{/if}

<ul class="checks">
	{#each PRECHECK as check (check.label)}
		<li data-state={check.state}>
			<span class="badge">{check.state === 'pass' ? '✓' : check.state === 'warn' ? '!' : '×'}</span>
			<div>
				<p class="label">{check.label}</p>
				<p class="detail">{check.detail}</p>
			</div>
		</li>
	{/each}
</ul>

<style>
	.live {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.25rem 0.75rem;
	}
	.link {
		background: none;
		border: none;
		padding: 0;
		font: inherit;
		color: var(--accent);
		text-decoration: underline;
		cursor: pointer;
		min-height: 44px;
	}
	.checks {
		list-style: none;
		margin: 1rem 0 2rem;
		padding: 0;
		display: grid;
		gap: 0.75rem;
	}
	.checks li {
		display: flex;
		gap: 0.75rem;
		align-items: flex-start;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
	.checks li[data-state='pass'] {
		background: var(--success-bg);
	}
	.checks li[data-state='warn'] {
		background: var(--warning-bg);
	}
	.checks li[data-state='fail'] {
		background: var(--error-bg);
	}
	.badge {
		flex: none;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 1.75rem;
		height: 1.75rem;
		border-radius: var(--radius-xs);
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
	}
	li[data-state='pass'] .badge {
		background: var(--success);
	}
	li[data-state='warn'] .badge {
		background: var(--warning);
	}
	li[data-state='fail'] .badge {
		background: var(--error);
	}
	.label {
		margin: 0;
		font-weight: 700;
	}
	.detail {
		margin: 0.15rem 0 0;
		font-size: 0.9rem;
	}
</style>
