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

<ul class="queue">
	{#each TRIAGE as fact (fact.id)}
		{@const item = stateOf(fact.id)}
		{#if item}
			<li>
				<div class="item-head">
					<span class="kind">{fact.authority} · {fact.topic}</span>
					{#if fact.conflict}
						<span class="conflict" role="alert">conflict</span>
					{/if}
					<span class="status mono">{item.status}</span>
				</div>
				<p><strong>#{fact.id}</strong> {fact.statement}</p>
				{#if fact.conflicts.length > 0}
					<ul class="conflicts">
						{#each fact.conflicts as other (other.id)}
							<li>
								<span class="mono">#{other.id} ({other.status})</span>
								{other.statement}
							</li>
						{/each}
					</ul>
				{/if}
				{#if item.note}
					<p class="note" role="status">{item.note}</p>
				{/if}
				<p>
					<label>
						Reason (required to reject, optional to accept)
						<input
							type="text"
							bind:value={item.reason}
							maxlength="1024"
							placeholder="audit reason"
						/>
					</label>
				</p>
				<p class="actions">
					<button onclick={() => decide(fact.id, 'accept')}>Accept</button>
					<button class="secondary" onclick={() => decide(fact.id, 'reject')}>Reject</button>
				</p>
			</li>
		{/if}
	{/each}
</ul>

<style>
	.queue {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0 0 2rem;
		display: grid;
		gap: 1rem;
	}
	.queue > li {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.9rem 1rem;
	}
	.item-head {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.25rem 0.75rem;
	}
	.kind {
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-muted);
	}
	.status {
		margin-left: auto;
		font-size: 0.875rem;
	}
	.mono {
		font-family: var(--font-mono);
	}
	.note {
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.6rem 0.8rem;
	}
	.conflict {
		font-size: 0.8rem;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--warning);
		border: 1px solid var(--warning);
		border-radius: var(--radius-xs);
		padding: 0.1rem 0.5rem;
	}
	.conflicts {
		list-style: none;
		margin: 0.5rem 0;
		padding: 0.6rem 0.8rem;
		border-left: 3px solid var(--warning);
		background: var(--warning-bg);
		display: grid;
		gap: 0.4rem;
		font-size: 0.9rem;
	}
	label {
		display: grid;
		gap: 0.3rem;
		font-size: 0.875rem;
		color: var(--text-muted);
	}
	input[type='text'] {
		font: inherit;
		color: var(--text);
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.5rem 0.75rem;
		min-height: 44px;
	}
	.actions {
		display: flex;
		gap: 0.75rem;
		margin-bottom: 0;
	}
	button {
		min-height: 44px;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
		padding: 0 1.25rem;
	}
	:root[data-theme='dark'] button:not(.secondary) {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	button.secondary {
		color: var(--text);
		background: var(--surface);
		border-color: var(--border);
	}
</style>
