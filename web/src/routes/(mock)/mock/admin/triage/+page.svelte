<script lang="ts">
	import Notice from '../../../../../lib/public-ui/Notice.svelte';
	import { TRIAGE } from '../../../../../lib/mock/fixtures';

	// Local decision state only: accept/reject flips the status label in
	// place. Conflicts never block (banner policy), rejections need a
	// reason — same contract as the real triage page.
	interface Item {
		id: number;
		status: 'proposed' | 'accepted' | 'rejected';
		reason: string;
		note: string | null;
	}

	let items = $state<Item[]>(TRIAGE.map((t) => ({ id: t.id, status: 'proposed', reason: '', note: null })));

	function decide(id: number, decision: 'accept' | 'reject') {
		const item = items.find((i) => i.id === id);
		if (!item) return;
		if (decision === 'reject' && !item.reason.trim()) {
			item.note = 'Rejection needs a reason (1–1024 chars).';
			return;
		}
		item.status = decision === 'accept' ? 'accepted' : 'rejected';
		item.note = `Recorded as ${item.status} (mock — nothing sent).`;
	}

	const stateOf = (id: number) => items.find((i) => i.id === id);
</script>

<svelte:head>
	<title>Mock triage — Apostille Handbook</title>
</svelte:head>

<h1>Fact triage</h1>

<Notice variant="info" heading="Fixture queue ({TRIAGE.length}).">
	<p>Decisions only flip the status label on this page.</p>
</Notice>

<ul class="mt-4 mb-0 grid list-none gap-4 p-0 pb-8">
	{#each TRIAGE as fact (fact.id)}
		{@const item = stateOf(fact.id)}
		{#if item}
			<li class="rounded-sm border border-line px-4 py-[0.9rem]">
				<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
					<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">
						{fact.authority} · {fact.topic}
					</span>
					{#if fact.conflict}
						<span
							class="rounded-xs border border-warning px-2 py-[0.1rem] text-[0.8rem] font-bold tracking-[0.08em] text-warning uppercase"
							role="alert">conflict</span
						>
					{/if}
					<span class="ml-auto font-mono text-sm">{item.status}</span>
				</div>
				<p><strong>#{fact.id}</strong> {fact.statement}</p>
				{#if fact.conflicts.length > 0}
					<ul
						class="my-2 grid list-none gap-[0.4rem] border-l-[3px] border-warning bg-warning-bg px-[0.8rem] py-[0.6rem] text-[0.9rem]"
					>
						{#each fact.conflicts as other (other.id)}
							<li>
								<span class="font-mono">#{other.id} ({other.status})</span>
								{other.statement}
							</li>
						{/each}
					</ul>
				{/if}
				{#if item.note}
					<p class="rounded-sm border border-line bg-surface px-[0.8rem] py-[0.6rem]" role="status">
						{item.note}
					</p>
				{/if}
				<p>
					<label class="grid gap-[0.3rem] text-sm text-ink-muted">
						Reason (required to reject, optional to accept)
						<input
							class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink [font:inherit]"
							type="text"
							bind:value={item.reason}
							maxlength="1024"
							placeholder="audit reason"
						/>
					</label>
				</p>
				<p class="mb-0 flex gap-3">
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit]"
						onclick={() => decide(fact.id, 'accept')}>Accept</button
					>
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink [font:inherit]"
						onclick={() => decide(fact.id, 'reject')}>Reject</button
					>
				</p>
			</li>
		{/if}
	{/each}
</ul>

