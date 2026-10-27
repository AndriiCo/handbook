<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { csrfHeaders } from '../../../../lib/admin/config-client.js';

	// Proposed-fact triage against the approval API: list proposed
	// facts with conflict banners, accept (banner policy: conflicts do
	// not block) or reject each with an audit reason. Same literal
	// /apostille root as the config page (spec §1.5).
	const api = (path: string) => `/apostille/api/v1${path}`;

	interface Conflict {
		id: number;
		statement: string;
		status: string;
	}

	interface Item {
		id: number;
		authority: string;
		topic: string;
		statement: string;
		status: string;
		conflict: boolean;
		conflicts: Conflict[];
		reason: string;
		note: string | null;
		busy: boolean;
	}

	let items = $state<Item[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);

	function isItem(value: Item | null): value is Item {
		return value !== null;
	}

	function normalize(f: any): Item | null {
		if (!f || typeof f.id !== 'number' || typeof f.statement !== 'string') return null;
		return {
			id: f.id,
			authority: typeof f.authority_slug === 'string' ? f.authority_slug : '?',
			topic: typeof f.topic === 'string' ? f.topic : '?',
			statement: f.statement,
			status: typeof f.status === 'string' ? f.status : 'proposed',
			conflict: f.conflict === true,
			conflicts: Array.isArray(f.conflicts) ? f.conflicts : [],
			reason: '',
			note: null,
			busy: false
		};
	}

	async function load() {
		loadState = 'loading';
		loadError = null;
		try {
			const res = await fetch(api('/admin/facts?status=proposed&limit=100'), {
				credentials: 'same-origin'
			});
			if (res.status === 401 || res.status === 403) {
				loadError = 'Staff session required — log in on the admin host first.';
			} else if (!res.ok) {
				throw new Error(`HTTP ${res.status}`);
			} else {
				const data = await res.json();
				const rows = Array.isArray(data?.facts) ? data.facts : [];
				items = rows.map(normalize).filter(isItem);
			}
			loadState = loadError ? 'failed' : 'ready';
		} catch {
			loadError = 'Review queue unreachable — is the API server running?';
			loadState = 'failed';
		}
	}

	onMount(load);

	async function decide(item: Item, decision: 'accept' | 'reject') {
		if (decision === 'reject' && !item.reason.trim()) {
			item.note = 'Rejection needs a reason (the server requires 1–1024 chars).';
			return;
		}
		item.busy = true;
		item.note = null;
		try {
			const res = await fetch(api(`/admin/facts/${item.id}/${decision}`), {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				credentials: 'same-origin',
				body: JSON.stringify({ reason: item.reason.trim() || null })
			});
			if (res.status === 401 || res.status === 403) {
				item.note = 'Staff session expired — log in again, then retry.';
			} else if (res.status === 422) {
				item.note = 'Rejected by the server (422) — reason or state invalid.';
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

<h1>Fact triage</h1>

{#if loadState === 'loading'}
	<Notice variant="info" heading="Loading queue…"><p>Fetching proposed facts.</p></Notice>
{:else if loadState === 'failed'}
	<Notice variant="warning" heading="Queue unavailable."><p>{loadError}</p></Notice>
	<p><button onclick={load}>Retry</button></p>
{:else if items.length === 0}
	<Notice variant="info" heading="Queue empty."><p>No proposed facts right now.</p></Notice>
{:else}
	<ul class="queue">
		{#each items as item (item.id)}
			<li>
				<div class="item-head">
					<span class="kind">{item.authority} · {item.topic}</span>
					{#if item.conflict}
						<span class="conflict" role="alert">conflict</span>
					{/if}
					<span class="status mono">{item.status}</span>
				</div>
				<p><strong>#{item.id}</strong> {item.statement}</p>
				{#if item.conflicts.length > 0}
					<ul class="conflicts">
						{#each item.conflicts as other (other.id)}
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
							disabled={item.busy}
						/>
					</label>
				</p>
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
{/if}

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
