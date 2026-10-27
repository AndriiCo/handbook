<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';

	// Document-photo precheck status. WS client is lazy-imported AFTER
	// mount so it never lands in an entry chunk (spa-separation §1.2).
	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;
	const wsUrl = () =>
		`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}${apiRoot}/ws`;

	type Live = 'connecting' | 'live' | 'reconnecting' | 'offline';
	let live = $state<Live>('connecting');
	let snapshot = $state<string | null>(null);
	let error = $state<string | null>(null);
	let closeWs: (() => void) | null = null;

	onMount(() => {
		let cancelled = false;
		(async () => {
			try {
				const { createWsClient } = await import('../../../lib/ws-client.js');
				if (cancelled) return;
				const client = createWsClient(wsUrl(), {
					onStatus: (next: Live) => {
						live = next;
					},
					onMessage: (msg: { type: string; topic: string }) => {
						snapshot = `${msg.type} on ${msg.topic}`;
					}
				});
				closeWs = () => client.close();
			} catch {
				if (!cancelled) live = 'offline';
			}
		})();
		return () => {
			cancelled = true;
			closeWs?.();
		};
	});

	async function refresh() {
		error = null;
		try {
			const res = await fetch(api('/precheck'), { credentials: 'same-origin' });
			if (res.status === 404) {
				error = 'Precheck endpoint not found (404) — the server lane has not landed yet.';
				return;
			}
			if (!res.ok) throw new Error(String(res.status));
			const data = await res.json();
			snapshot = typeof data?.status === 'string' ? data.status : JSON.stringify(data);
		} catch {
			error = 'Precheck service unreachable — is the API server running?';
		}
	}
</script>

<svelte:head>
	<title>Precheck — Apostille Handbook</title>
</svelte:head>

<h1>Document precheck</h1>

<p class="pill" data-live={live} aria-live="polite">
	{#if live === 'live'}Live
	{:else if live === 'reconnecting'}Reconnecting…
	{:else if live === 'offline'}Offline
	{:else}Connecting…{/if}
</p>

{#if snapshot}
	<Notice variant="info" heading="Latest status."><p>{snapshot}</p></Notice>
{:else}
	<Notice variant="info" heading="No precheck yet.">
		<p>Upload a document photo first — its photo check will appear here.</p>
	</Notice>
{/if}
{#if error}
	<Notice variant="warning" heading="Snapshot unavailable."><p>{error}</p></Notice>
{/if}

<p><button onclick={refresh}>Refresh snapshot</button></p>
<p><a href={resolve('/(customer)/uploads')}>Go to uploads</a></p>

<style>
	.pill {
		display: inline-block;
		border-radius: var(--radius-xs);
		padding: 0.25rem 0.75rem;
		font-weight: 700;
		font-size: 0.875rem;
		background: var(--surface-2);
		color: var(--text-muted);
		border: 1px solid var(--border);
	}
	.pill[data-live='live'] {
		background: var(--success-bg);
		color: var(--success);
		border-color: var(--success);
	}
	.pill[data-live='reconnecting'] {
		background: var(--warning-bg);
		color: var(--warning);
		border-color: var(--warning);
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
</style>
