<script lang="ts">
	import CitationMark from './CitationMark.svelte';
	import { draftedBy, parseStatement } from './guide.js';

	// Read-only legalization chain: one hop per staff-accepted
	// `legalization_chain` fact, in acceptance (id) order, each with its
	// quote one tap away. Runtime-only — no bundled pack; with no such
	// facts the component renders nothing and the page hides it.
	let { facts = [] }: { facts?: any[] } = $props();

	const hops = $derived(
		(facts ?? [])
			.filter((f) => f?.status === 'accepted' && f?.topic === 'legalization_chain')
			.sort((a, b) => (a?.id ?? 0) - (b?.id ?? 0))
	);
</script>

{#if hops.length > 0}
	<ol class="m-0 list-none p-0">
		{#each hops as hop, i (hop.id ?? i)}
			{@const value = parseStatement(hop.statement).value}
			<li class="relative m-0 flex gap-4 pb-6 last:pb-0">
				{#if i < hops.length - 1}
					<span aria-hidden="true" class="absolute top-10 bottom-0 left-4 w-px bg-line"></span>
				{/if}
				<span
					aria-hidden="true"
					class="flex size-9 shrink-0 items-center justify-center rounded-full border border-line bg-bg font-mono text-sm font-semibold text-ink"
				>
					{i + 1}
				</span>
				<div class="min-w-0 flex-1 pt-1">
					<p class="m-0 break-words text-ink">{value}</p>
					<div class="mt-1 flex flex-wrap items-center gap-x-3 text-sm">
						<CitationMark
							citation={hop.citations?.[0] ?? null}
							value={value}
							draftedBy={draftedBy(hop)}
						/>
					</div>
				</div>
			</li>
		{/each}
	</ol>
{/if}
