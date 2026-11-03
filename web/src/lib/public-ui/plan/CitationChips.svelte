<script lang="ts">
	import CitationMark from '../CitationMark.svelte';
	import { parseStatement } from '../guide.js';
	import { topicName, type PlanFact } from '../plan.js';

	// One evidence chip per cited, accepted fact (deduplicated, plan
	// order). Uncited or unknown ids render nothing.
	let { ids = [], facts }: { ids?: (number | undefined)[]; facts: Map<number, PlanFact> } = $props();

	const cited = $derived(
		[...new Set(ids.filter((id): id is number => typeof id === 'number'))]
			.map((id) => facts.get(id))
			.filter((f): f is PlanFact => !!f && !!f.citations?.[0]?.quote)
	);
</script>

{#if cited.length > 0}
	<ul class="m-0 flex list-none flex-wrap gap-2 p-0" aria-label="Sources">
		{#each cited as fact (fact.id)}
			<li class="m-0 p-0">
				<CitationMark
					variant="chip"
					label={topicName(String(fact.topic ?? ''))}
					citation={fact.citations?.[0] ?? null}
					value={parseStatement(String(fact.statement ?? '')).value}
					draftedBy={String(fact.drafted_by ?? fact.created_by ?? 'unknown').split(' ')[0]}
				/>
			</li>
		{/each}
	</ul>
{/if}
