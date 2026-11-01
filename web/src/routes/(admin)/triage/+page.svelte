<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';

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
			const res = await fetch(api('/facts?status=proposed&limit=100'), {
				credentials: 'same-origin'
			});
			if (res.status === 401) {
				loadError = 'Staff session missing or expired — log in on the admin host again.';
			} else if (res.status === 403) {
				loadError =
					'Logged in, but the admin host refused the request — the browser must present your enrolled client certificate.';
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
			const res = await fetch(api(`/facts/${item.id}/${decision}`), {
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
	<p>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
			onclick={load}>Retry</button
		>
	</p>
{:else if items.length === 0}
	<Notice variant="info" heading="Queue empty."><p>No proposed facts right now.</p></Notice>
{:else}
	<ul class="mt-4 mb-0 grid list-none gap-4 p-0 pb-8">
		{#each items as item (item.id)}
			<li class="rounded-sm border border-line px-4 py-[0.9rem]">
				<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
					<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">
						{item.authority} · {item.topic}
					</span>
					{#if item.conflict}
						<span
							class="rounded-xs border border-warning px-2 py-[0.1rem] text-[0.8rem] font-bold tracking-[0.08em] text-warning uppercase"
							role="alert">conflict</span
						>
					{/if}
					<span class="ml-auto font-mono text-sm">{item.status}</span>
				</div>
				<p><strong>#{item.id}</strong> {item.statement}</p>
				{#if item.conflicts.length > 0}
					<ul
						class="my-2 grid list-none gap-[0.4rem] border-l-[3px] border-warning bg-warning-bg px-[0.8rem] py-[0.6rem] text-[0.9rem]"
					>
						{#each item.conflicts as other (other.id)}
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
							class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
							type="text"
							bind:value={item.reason}
							maxlength="1024"
							placeholder="audit reason"
							disabled={item.busy}
						/>
					</label>
				</p>
				<p class="mb-0 flex gap-3">
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
						onclick={() => decide(item, 'accept')}
						disabled={item.busy}
					>
						{item.busy ? 'Working…' : 'Accept'}
					</button>
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink disabled:cursor-not-allowed disabled:opacity-55"
						onclick={() => decide(item, 'reject')}
						disabled={item.busy}
					>
						{item.busy ? 'Working…' : 'Reject'}
					</button>
				</p>
			</li>
		{/each}
	</ul>
{/if}
