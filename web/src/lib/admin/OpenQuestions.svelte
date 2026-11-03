<script lang="ts">
	import { fly } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import QuestionMeta from './QuestionMeta.svelte';
	import type { OpenQuestionsState } from './first-source.svelte.js';
	import { MAX_CALL_QUESTIONS, TOP_N, hasQuestion, questionLines } from './outreach-client.js';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ListPlusIcon from '@lucide/svelte/icons/list-plus';
	import BuildingIcon from '@lucide/svelte/icons/building-2';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';

	// Ranked open questions for the picked office (/calls): what the
	// fact store still cannot answer, strongest first. "Add" appends one
	// line to the call script, "Add top 6" the strongest six (the
	// TOPICS.md batching rule). Live: an approval in /review re-reads
	// the list over `authority:{slug}`.
	let {
		state,
		name,
		script,
		motion = 200,
		onAdd,
		onPick,
		onDefault
	}: {
		state: OpenQuestionsState;
		name: string;
		script: string;
		motion?: number;
		onAdd: (lines: string[]) => void;
		onPick: () => void;
		onDefault: () => void;
	} = $props();

	const lines = $derived(questionLines(script).length);
	const full = $derived(lines >= MAX_CALL_QUESTIONS);
	const top = $derived(state.questions.slice(0, TOP_N).map((q) => q.question));
	const topLeft = $derived(top.filter((q) => !hasQuestion(script, q)).length);
</script>

<Card.Root class="min-w-0">
	<Card.Header>
		<Card.Title class="flex flex-wrap items-center gap-2">
			<h2 class="m-0 text-h3 font-semibold">Open questions</h2>
			{#if state.phase === 'ready' && state.questions.length > 0}
				<span class="min-w-6 rounded-full bg-surface-2 px-2 text-center text-xs font-semibold text-ink-muted tabular-nums">
					{state.questions.length}
				</span>
			{/if}
			{#if state.live}
				<span class="inline-flex items-center gap-1 text-xs font-normal text-ink-muted" title="Approvals in Review update this list live">
					<span class="size-2 rounded-full bg-verified" aria-hidden="true"></span>
					Live
				</span>
			{/if}
		</Card.Title>
		<Card.Description class="text-ink-muted">
			{#if state.slug}
				What {name} still has to confirm, strongest first.
			{:else}
				Pick an office to rank what it still has to confirm.
			{/if}
		</Card.Description>
		{#if state.phase === 'ready' && state.questions.length > 0}
			<Card.Action>
				<Button
					variant="outline"
					size="sm"
					disabled={topLeft === 0 || full}
					onclick={() => onAdd(top)}
				>
					<ListPlusIcon aria-hidden="true" />
					Add top {Math.min(TOP_N, state.questions.length)}
				</Button>
			</Card.Action>
		{/if}
	</Card.Header>
	<Card.Content>
		{#if !state.slug}
			<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line px-6 py-10 text-center">
				<BuildingIcon class="size-8 text-ink-muted" aria-hidden="true" />
				<p class="m-0 max-w-prose text-sm text-ink-muted">
					Each office's list comes from its facts: contradicted by the source, still unconfirmed,
					missing a core topic, or over a year old.
				</p>
				<Button variant="outline" onclick={onPick}>Choose an office</Button>
			</div>
		{:else if state.phase === 'loading'}
			<div class="flex flex-col divide-y divide-line" role="status" aria-label="Loading open questions">
				{#each [0, 1, 2, 3] as i (i)}
					<div class="flex gap-3 py-3">
						<div class="flex min-w-0 flex-1 flex-col gap-2">
							<Skeleton class="h-5 w-40 rounded-full" />
							<Skeleton class="h-4 w-full rounded-xs" />
							<Skeleton class="h-3 w-2/3 rounded-xs" />
						</div>
						<Skeleton class="h-7 w-16 rounded-md" />
					</div>
				{/each}
			</div>
		{:else if state.phase === 'failed'}
			<Notice variant="error" heading="Open questions could not load.">
				<p class="m-0">{state.error}</p>
				<p class="mt-3 mb-0"><Button variant="outline" onclick={() => state.reload()}>Try again</Button></p>
			</Notice>
		{:else if state.questions.length === 0}
			<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line px-6 py-10 text-center">
				<CircleCheckIcon class="size-8 text-verified" aria-hidden="true" />
				<p class="m-0 text-h3 font-semibold">Nothing open for this office</p>
				<p class="m-0 max-w-prose text-sm text-ink-muted">
					Every core topic has a current, accepted fact. A general check-in call can still catch changes.
				</p>
				<Button variant="outline" onclick={onDefault}>Load the default script</Button>
			</div>
		{:else}
			<ol class="m-0 flex list-none flex-col divide-y divide-line p-0" aria-label={`Open questions for ${name}`}>
				{#each state.questions as q, i (q.question)}
					{@const added = hasQuestion(script, q.question)}
					<li
						class="flex flex-col gap-1.5 py-3 first:pt-0 last:pb-0"
						in:fly={{ y: -6, duration: motion }}
						animate:flip={{ duration: motion }}
					>
						<div class="flex items-center gap-2">
							<span class="w-5 flex-none text-right font-mono text-xs text-ink-muted tabular-nums" aria-hidden="true">
								{i + 1}
							</span>
							<span class="min-w-0 flex-1"><QuestionMeta question={q} /></span>
							<Button
								variant={added ? 'ghost' : 'outline'}
								size="sm"
								class="flex-none"
								disabled={added || full}
								aria-label={added ? `Added question ${i + 1} to the call script` : `Add question ${i + 1} to the call script`}
								onclick={() => onAdd([q.question])}
							>
								{#if added}
									<CheckIcon aria-hidden="true" />
									Added
								{:else}
									<PlusIcon aria-hidden="true" />
									Add
								{/if}
							</Button>
						</div>
						<p class="m-0 pl-7 text-sm text-ink">{q.question}</p>
						<p class="m-0 pl-7 text-xs text-ink-muted">
							{q.why}
							{#if q.fact_ids.length > 0}
								<span class="font-mono whitespace-nowrap"> · fact {q.fact_ids.map((id) => `#${id}`).join(', ')}</span>
							{/if}
						</p>
					</li>
				{/each}
			</ol>
			{#if state.refreshing}
				<p class="m-0 mt-3 text-xs text-ink-muted" role="status">Updating after an approval…</p>
			{:else if state.error}
				<p class="m-0 mt-3 text-xs text-ink-muted" role="status">Last refresh failed: {state.error}</p>
			{/if}
		{/if}
	</Card.Content>
</Card.Root>
