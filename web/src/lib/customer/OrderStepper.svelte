<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import { site } from '../locale.svelte.js';
	import { ORDER_COPY } from './order-copy.js';

	// Order progress. The current step carries aria-current="step" (the
	// GP4 scoreboard row and the shots ready text "Destination" read it).
	// Completed steps are buttons back to that step. One row at every
	// width: at 390 px the labels drop to the small size.
	let {
		steps,
		current,
		onpick
	}: {
		steps: readonly { id: string; label: string }[];
		current: number;
		onpick: (index: number) => void;
	} = $props();

	const c = $derived(ORDER_COPY[site.locale]);
	const progress = $derived(steps.length > 1 ? current / (steps.length - 1) : 0);
</script>

<ol class="relative m-0 flex list-none items-start p-0" aria-label={c.stepperLabel}>
	<span aria-hidden="true" class="absolute top-3.5 right-6 left-3.5 h-0.5 bg-surface-2"></span>
	<span
		aria-hidden="true"
		class="absolute top-3.5 right-6 left-3.5 h-0.5 origin-left scale-x-(--p) bg-accent-fill transition-transform duration-300 motion-reduce:transition-none"
		style={`--p: ${progress};`}
	></span>
	{#each steps as s, i (s.id)}
		{@const isCurrent = i === current}
		{@const isPast = i < current}
		<li
			class="relative flex min-w-0 flex-1 flex-col items-start gap-1 pt-8"
			aria-current={isCurrent ? 'step' : undefined}
		>
			<span
				aria-hidden="true"
				class={`absolute top-0 left-0 flex size-7 flex-none items-center justify-center rounded-full border text-sm font-bold transition-colors duration-200 ${
					isCurrent
						? 'border-accent-fill bg-accent-fill text-accent-ink shadow-md'
						: isPast
							? 'border-verified bg-verified text-bg'
							: 'border-line bg-bg text-ink-muted'
				}`}
			>
				{#if isPast}
					<CheckIcon class="size-4" />
				{:else}
					{i + 1}
				{/if}
			</span>
			{#if isPast}
				<button
					type="button"
					class="-mt-3 min-h-11 max-w-full cursor-pointer truncate border-0 bg-transparent p-0 text-left text-xs text-ink underline decoration-line underline-offset-4 hover:decoration-ink sm:text-sm"
					onclick={() => onpick(i)}
				>
					{s.label}
				</button>
			{:else}
				<span
					class={`max-w-full truncate text-xs sm:text-sm ${isCurrent ? 'font-bold text-ink' : 'text-ink-muted'}`}
				>
					{s.label}
				</span>
			{/if}
		</li>
	{/each}
</ol>
