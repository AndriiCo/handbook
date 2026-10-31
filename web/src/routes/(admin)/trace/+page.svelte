<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';

	// Trace console: every watched page's capture sha256 + short text,
	// newest first. Batches of 25 through before_id paging — never one
	// giant article. Admin listener only (same /apostille root).
	const api = (path: string) => `/apostille/api/v1${path}`;
	const PAGE = 25;
	const BTN =
		'min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55';

	interface Trace {
		id: number;
		authority_slug: string;
		source_uri: string | null;
		captured_at: number;
		kind: string;
		bytes_raw: number;
		sha256: string;
		trust: string;
		excerpt: string;
	}

	let traces = $state<Trace[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let loadingMore = $state(false);
	let exhausted = $state(false);
	let authority = $state('');

	function normalize(c: any): Trace | null {
		if (!c || typeof c.id !== 'number' || typeof c.sha256 !== 'string') return null;
		return {
			id: c.id,
			authority_slug: typeof c.authority_slug === 'string' ? c.authority_slug : '?',
			source_uri: typeof c.source_uri === 'string' ? c.source_uri : null,
			captured_at: typeof c.captured_at === 'number' ? c.captured_at : 0,
			kind: typeof c.kind === 'string' ? c.kind : '?',
			bytes_raw: typeof c.bytes_raw === 'number' ? c.bytes_raw : 0,
			sha256: c.sha256,
			trust: typeof c.trust === 'string' ? c.trust : '?',
			excerpt: typeof c.excerpt === 'string' ? c.excerpt : ''
		};
	}

	function day(ts: number): string {
		if (!ts) return '—';
		return new Date(ts * 1000).toISOString().split('T')[0] ?? '—';
	}

	async function loadBatch(after: number | null, reset: boolean) {
		const params = new URLSearchParams({ limit: String(PAGE) });
		if (after !== null) params.set('before_id', String(after));
		const slug = authority.trim();
		if (slug) params.set('authority', slug);
		const res = await fetch(api(`/workers/captures?${params}`));
		if (!res.ok) throw new Error(`captures -> ${res.status}`);
		const body = await res.json();
		const batch: Trace[] = Array.isArray(body?.captures)
			? body.captures.map(normalize).filter((t: Trace | null) => t !== null)
			: [];
		if (reset) traces = batch;
		else traces = [...traces, ...batch];
		if (batch.length < PAGE) exhausted = true;
	}

	async function reload() {
		loadState = 'loading';
		loadError = null;
		exhausted = false;
		try {
			await loadBatch(null, true);
			loadState = 'ready';
		} catch (e) {
			loadState = 'failed';
			loadError = e instanceof Error ? e.message : 'load failed';
		}
	}

	async function more() {
		if (loadingMore || exhausted || traces.length === 0) return;
		loadingMore = true;
		try {
			await loadBatch(traces[traces.length - 1].id, false);
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'load failed';
		} finally {
			loadingMore = false;
		}
	}

	onMount(reload);
</script>

<svelte:head>
	<title>Trace console — Admin — Apostille Handbook</title>
</svelte:head>

<h1>Trace console</h1>
<p class="text-ink-muted">
	Every watched page, newest first: capture sha256 with the text it carried. Batches of {PAGE}.
</p>

<form
	onsubmit={(e) => {
		e.preventDefault();
		reload();
	}}
	class="mb-4 flex max-w-[28rem] flex-col gap-2"
>
	<label class="font-bold" for="trace-authority">Authority slug (optional)</label>
	<input
		id="trace-authority"
		class="min-h-11 rounded-xs border border-line bg-bg px-3 text-ink"
		type="text"
		bind:value={authority}
		placeholder="vietnam"
		autocomplete="off"
	/>
	<p class="m-0"><button class={BTN} type="submit">Filter</button></p>
</form>

{#if loadState === 'loading'}
	<ul class="m-0 list-none p-0" aria-label="Captures">
		{#each [0, 1, 2, 3] as i (i)}
			<li class="border-b border-line py-3" aria-hidden="true">
				<Skeleton class="mb-2 h-4 w-2/3" />
				<Skeleton class="h-4 w-full" />
			</li>
		{/each}
	</ul>
{:else if loadState === 'failed'}
	<Notice variant="error" heading="Trace failed to load.">
		<p>{loadError}</p>
		<p><button class={BTN} type="button" onclick={reload}>Retry</button></p>
	</Notice>
{:else if traces.length === 0}
	<Notice variant="info" heading="No captures yet.">
		<p>Run a fetch sweep first — captures appear here newest-first.</p>
	</Notice>
{:else}
	<ul class="m-0 list-none p-0">
		{#each traces as t (t.id)}
			<li class="border-b border-line py-3">
				<p class="m-0 mb-1 font-bold">
					#{t.id} · {t.authority_slug} · {day(t.captured_at)}
					<span class="font-mono font-normal text-ink-muted">· {t.kind} · {t.bytes_raw} B</span>
				</p>
				<p class="m-0 mb-1 font-mono text-sm break-all" title={t.sha256}>
					sha256:{t.sha256.slice(0, 16)}…
				</p>
				<p class="m-0 text-sm text-ink-muted">{t.excerpt || '— no text extracted —'}</p>
			</li>
		{/each}
	</ul>
	{#if loadError}
		<Notice variant="warning" heading="Older captures failed to load.">
			<p>{loadError}</p>
		</Notice>
	{/if}
	{#if !exhausted}
		<p>
			<button class={BTN} type="button" onclick={more} disabled={loadingMore}>
				{loadingMore ? 'Working…' : 'Load older captures'}
			</button>
		</p>
	{/if}
{/if}
