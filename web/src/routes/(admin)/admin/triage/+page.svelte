<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';

	// Fact/task triage shell. No review endpoint exists yet — actions
	// attempt the spec'd API and degrade honestly.
	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	interface Item {
		id: string;
		kind: string;
		title: string;
		status: string;
		note: string | null;
		busy: boolean;
	}

	let items = $state<Item[]>([
		{
			id: 'item-1',
			kind: 'fact',
			title: 'Placeholder proposed fact — nothing to review yet',
			status: 'pending',
			note: null,
			busy: false
		}
	]);

	async function decide(item: Item, decision: 'accept' | 'reject') {
		item.busy = true;
		item.note = null;
		try {
			const res = await fetch(api(`/admin/review/${item.id}`), {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				credentials: 'same-origin',
				body: JSON.stringify({ decision })
			});
			if (res.status === 404) {
				item.note = 'Review endpoint not found (404) — the server lane has not landed yet.';
			} else if (!res.ok) {
				item.note = `Decision failed: HTTP ${res.status}. Nothing changed.`;
			} else {
				item.status = decision === 'accept' ? 'accepted' : 'rejected';
				item.note = `Recorded as ${item.status}.`;
			}
		} catch {
			item.note = 'Review service unreachable — is the API server running?';
		} finally {
			item.busy = false;
		}
	}
</script>

<svelte:head>
	<title>Triage — Admin — Apostille Handbook</title>
</svelte:head>

<h1>Fact / task triage</h1>

<Notice variant="info" heading="Queue is a placeholder.">
	<p>Proposed facts and open tasks will queue here with citations and diffs.</p>
</Notice>

<ul class="queue">
	{#each items as item (item.id)}
		<li>
			<div class="item-head">
				<span class="kind">{item.kind}</span>
				<strong>{item.title}</strong>
				<span class="status mono">{item.status}</span>
			</div>
			{#if item.note}
				<p class="note" role="status">{item.note}</p>
			{/if}
			<p class="actions">
				<button onclick={() => decide(item, 'accept')} disabled={item.busy}>
					{item.busy ? 'Working…' : 'Accept'}
				</button>
				<button class="secondary" onclick={() => decide(item, 'reject')} disabled={item.busy}>
					{item.busy ? 'Working…' : 'Reject'}
				</button>
			</p>
		</li>
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
	.queue li {
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
	:root[data-theme='dark'] button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	button.secondary {
		color: var(--text);
		background: var(--surface);
		border-color: var(--border);
	}
	button:disabled {
		opacity: 0.55;
		cursor: not-allowed;
	}
</style>
