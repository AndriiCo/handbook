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

	// Full class strings per state: exactly one background wins.
	const liCls = (state: string) =>
		`flex items-start gap-3 rounded-sm border border-line px-4 py-3 ${
			state === 'pass' ? 'bg-success-bg' : state === 'warn' ? 'bg-warning-bg' : 'bg-error-bg'
		}`;
	const badgeCls = (state: string) =>
		`inline-flex size-7 flex-none items-center justify-center rounded-xs font-bold text-accent-ink ${
			state === 'pass' ? 'bg-success' : state === 'warn' ? 'bg-warning' : 'bg-error'
		}`;

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

<p class="flex flex-wrap items-center gap-x-3 gap-y-1" role="status">
	Connection: <strong>{live}</strong>
	<button
		class="min-h-11 cursor-pointer border-0 bg-transparent p-0 text-accent underline [font:inherit]"
		onclick={cycle}>Simulate next state</button
	>
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

<ul class="mt-4 mb-8 grid list-none gap-3 p-0">
	{#each PRECHECK as check (check.label)}
		<li class={liCls(check.state)} data-state={check.state}>
			<span class={badgeCls(check.state)}
				>{check.state === 'pass' ? '✓' : check.state === 'warn' ? '!' : '×'}</span
			>
			<div>
				<p class="m-0 font-bold">{check.label}</p>
				<p class="mt-[0.15rem] mb-0 text-[0.9rem]">{check.detail}</p>
			</div>
		</li>
	{/each}
</ul>
