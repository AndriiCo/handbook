<script lang="ts">
	import InfoIcon from '@lucide/svelte/icons/info';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import CitationChips from './CitationChips.svelte';
	import type { PlanFact } from '../plan.js';

	let {
		tone = 'info',
		title = '',
		body = '',
		facts,
		factIds = []
	}: {
		tone?: 'info' | 'warning';
		title?: string;
		body?: string;
		facts: Map<number, PlanFact>;
		factIds?: number[];
	} = $props();
</script>

<div
	role="note"
	class={`flex gap-3 rounded-r-xl border-l-4 p-5 ${
		tone === 'warning' ? 'border-changed bg-changed-bg' : 'border-brand-500 bg-bg'
	}`}
>
	{#if tone === 'warning'}
		<TriangleAlertIcon class="mt-0.5 size-5 flex-none text-changed" aria-hidden="true" />
	{:else}
		<InfoIcon class="mt-0.5 size-5 flex-none text-accent-strong" aria-hidden="true" />
	{/if}
	<div class="min-w-0 flex-1">
		{#if title}<p class="m-0 font-semibold text-ink">{title}</p>{/if}
		{#if body}<p class="mt-1 mb-0 text-sm text-ink">{body}</p>{/if}
		<div class="mt-3">
			<CitationChips ids={factIds} {facts} />
		</div>
	</div>
</div>
