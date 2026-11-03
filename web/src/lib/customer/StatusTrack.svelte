<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import { stageReached, stagesFor, statusLabel, type OrderKind } from './orders';

	// The order's progress through its kind's stages, drawn from the
	// order's own status (side states dim the track).
	let { kind, status }: { kind: OrderKind; status: string } = $props();

	const stages = $derived(stagesFor(kind));
	const reached = $derived(stageReached(kind, status));
	const off = $derived(status === 'refunded' || status === 'canceled');
</script>

<ol
	class={`m-0 grid list-none gap-2 p-0 ${off ? 'opacity-60' : ''}`}
	style={`grid-template-columns: repeat(${stages.length}, minmax(0, 1fr));`}
	aria-label={`Order progress: ${statusLabel(status)}`}
	data-graphic="status-track"
>
	{#each stages as s, i (s)}
		{@const done = i <= reached}
		{@const current = i === reached}
		<li class="flex min-w-0 flex-col gap-2" aria-current={current ? 'step' : undefined}>
			<span
				aria-hidden="true"
				class={`h-1.5 rounded-full transition-colors duration-300 ${done ? 'bg-verified' : 'bg-surface-2'}`}
			></span>
			<span class={`flex items-start gap-1 text-xs sm:text-sm ${current ? 'font-semibold text-ink' : done ? 'text-ink' : 'text-ink-muted'}`}>
				{#if done && !current}<CheckIcon class="size-3.5 flex-none text-verified" aria-hidden="true" />{/if}
				<span class="leading-tight">{statusLabel(s)}</span>
			</span>
		</li>
	{/each}
</ol>
