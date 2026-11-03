<script lang="ts">
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import type { Guard } from './proposals.js';

	// Deterministic gate report: one row per check, pass or needs-staff.
	let { guards, label }: { guards: Guard[]; label: string } = $props();

	const NAMES: Record<string, string> = {
		fact_ids_accepted: 'Every block cites accepted facts',
		numbers_in_citations: 'Numbers appear in the cited text',
		hours_guard: 'Hours are consistent',
		no_urls_html: 'No links or markup in copy',
		entailment: 'Muse Wonder entailment',
		address_ok: 'Address is in the cited fact',
		hours_ok: 'Hours parse cleanly',
		po_box_from_text: 'PO Box is quoted',
		timezone: 'Timezone',
		label_in_span: 'Labels found in the PDF text',
		names_from_acroform: 'Field names come from the AcroForm'
	};
</script>

<ul class="m-0 flex list-none flex-col gap-2 p-0" aria-label={label}>
	{#each guards as g (g.id)}
		<li class="flex items-start gap-2 text-sm">
			{#if g.ok}
				<CircleCheckIcon class="mt-0.5 size-4 flex-none text-verified" aria-hidden="true" />
			{:else}
				<TriangleAlertIcon class="mt-0.5 size-4 flex-none text-changed" aria-hidden="true" />
			{/if}
			<span class="flex min-w-0 flex-col">
				<span class="font-medium">
					{NAMES[g.id] ?? g.id}
					<span class="sr-only">{g.ok ? '(passed)' : '(needs staff)'}</span>
				</span>
				{#if g.detail}
					<span class="text-xs text-ink-muted">{g.detail}</span>
				{/if}
			</span>
		</li>
	{/each}
</ul>
