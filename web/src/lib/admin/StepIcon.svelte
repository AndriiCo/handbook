<script lang="ts">
	import CircleIcon from '@lucide/svelte/icons/circle';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import CircleXIcon from '@lucide/svelte/icons/circle-x';
	import CircleDashedIcon from '@lucide/svelte/icons/circle-dashed';
	import CirclePauseIcon from '@lucide/svelte/icons/circle-pause';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';

	// Run-timeline status icon shared by the Compose and Compile cards.
	// `active` and `running` are the same state (Compose names it the
	// first way, the Compile bus the second).
	let {
		state
	}: {
		state: 'pending' | 'active' | 'running' | 'done' | 'failed' | 'skipped' | 'stopped';
	} = $props();
</script>

{#if state === 'active' || state === 'running'}
	<LoaderCircleIcon class="mt-0.5 size-4 flex-none text-accent-strong motion-safe:animate-spin" aria-hidden="true" />
{:else if state === 'done'}
	<CircleCheckIcon class="mt-0.5 size-4 flex-none text-verified" aria-hidden="true" />
{:else if state === 'failed'}
	<CircleXIcon class="mt-0.5 size-4 flex-none text-error" aria-hidden="true" />
{:else if state === 'stopped'}
	<CirclePauseIcon class="mt-0.5 size-4 flex-none text-changed" aria-hidden="true" />
{:else if state === 'skipped'}
	<CircleDashedIcon class="mt-0.5 size-4 flex-none text-ink-muted" aria-hidden="true" />
{:else}
	<CircleIcon class="mt-0.5 size-4 flex-none text-ink-muted" aria-hidden="true" />
{/if}
