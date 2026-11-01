<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';
	import { subscribe, type BusMessage } from '../../../lib/live.svelte.js';

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

	interface DraftRun {
		id: number;
		capture_id: number;
		slug: string;
		status: string;
		proposed: number;
		rejected: number;
		model: string;
		input_tokens: number;
		output_tokens: number;
		error: string;
	}

	// Live draft runs by capture id. The server streams steps to
	// `run:{id}` on the admin bus; this console subscribes — no polling.
	let drafts = $state<Record<number, DraftRun | { status: 'starting' }>>({});

	// One `run:{id}` subscription per started draft (unsubscribed on
	// terminal steps, on timeout, and when the page goes away).
	const runSubs = new Map<number, () => void>();
	const runTimers = new Map<number, ReturnType<typeof setTimeout>>();

	function clearRunWatch(captureId: number) {
		runSubs.get(captureId)?.();
		runSubs.delete(captureId);
		const timer = runTimers.get(captureId);
		if (timer !== undefined) clearTimeout(timer);
		runTimers.delete(captureId);
	}

	function applyFinished(captureId: number, runId: number, src: BusMessage) {
		drafts[captureId] = {
			status: 'finished',
			id: runId,
			capture_id: captureId,
			slug: typeof src?.slug === 'string' ? src.slug : '',
			proposed: typeof src?.proposed === 'number' ? src.proposed : 0,
			rejected: typeof src?.rejected === 'number' ? src.rejected : 0,
			model: typeof src?.model === 'string' ? src.model : '',
			input_tokens: typeof src?.input_tokens === 'number' ? src.input_tokens : 0,
			output_tokens: typeof src?.output_tokens === 'number' ? src.output_tokens : 0,
			error: ''
		};
		clearRunWatch(captureId);
	}

	// Reconcile against the server's run registry: a `run_finished`
	// or `run_failed` step emitted while the socket was down never
	// arrives, so every `subscribed` hello re-reads the run. A 404
	// means the registry is gone (server restarted) — fail fast
	// instead of waiting out the budget. Anything else unreadable
	// keeps waiting; the client timeout still guards.
	async function reconcileRun(captureId: number, runId: number) {
		let res: Response;
		try {
			res = await fetch(api(`/draft-runs/${runId}`));
		} catch {
			return;
		}
		if (res.status === 404) {
			failRun(captureId, runId, 'run not found on server (it may have restarted)');
			return;
		}
		if (!res.ok) return;
		let run: any;
		try {
			run = await res.json();
		} catch {
			return;
		}
		if (!run || run.id !== runId) return;
		if (run.status === 'finished') applyFinished(captureId, runId, run);
		else if (run.status === 'failed')
			failRun(captureId, runId, typeof run.error === 'string' && run.error ? run.error : 'draft run failed');
	}

	function failRun(captureId: number, runId: number, error: string) {
		drafts[captureId] = {
			status: 'failed',
			id: runId,
			capture_id: captureId,
			slug: '',
			proposed: 0,
			rejected: 0,
			model: '',
			input_tokens: 0,
			output_tokens: 0,
			error
		};
		clearRunWatch(captureId);
	}

	function watchRun(captureId: number, runId: number) {
		clearRunWatch(captureId);
		const unsub = subscribe(`run:${runId}`, (msg) => {
			// The server re-sends its hello on every (re)connect and
			// carries no run_id: reconcile instead of ignoring it,
			// so a terminal step missed during a blink still lands.
			if (msg?.type === 'subscribed') {
				reconcileRun(captureId, runId);
				return;
			}
			if (msg?.run_id !== runId) return;
			if (msg?.type === 'run_finished') {
				applyFinished(captureId, runId, msg);
			} else if (msg?.type === 'run_failed') {
				failRun(
					captureId,
					runId,
					typeof msg?.error === 'string' && msg.error ? msg.error : 'draft run failed'
				);
			}
		});
		runSubs.set(captureId, unsub);
		// The hello above reconciles the common case at once; this
		// timer is the backstop. A run that never reports back must
		// not spin the button forever: fail it after 330 s (the
		// model budget is 300 s, plus margin for the live steps).
		runTimers.set(
			captureId,
			setTimeout(() => {
				const current = drafts[captureId];
				if (current && (current.status === 'starting' || current.status === 'running')) {
					failRun(captureId, runId, 'run timed out waiting for live steps');
				} else {
					clearRunWatch(captureId);
				}
			}, 330_000)
		);
	}

	async function draftCapture(captureId: number) {
		const prior = drafts[captureId];
		if (prior && prior.status !== 'failed') return;
		drafts[captureId] = { status: 'starting' };
		try {
			// Double-submit CSRF: the admin csrf_guard 403s mutations
			// without the `hb_csrf` cookie echoed in `x-csrf`.
			const res = await fetch(api(`/captures/${captureId}/draft`), {
				method: 'POST',
				headers: { ...csrfHeaders() }
			});
			if (!res.ok) throw new Error(`draft -> ${res.status}`);
			const body = await res.json();
			const runId = Number(body?.run_id);
			if (!Number.isInteger(runId) || runId <= 0) throw new Error('bad run id');
			watchRun(captureId, runId);
		} catch (e) {
			drafts[captureId] = {
				status: 'failed',
				id: 0,
				capture_id: captureId,
				slug: '',
				proposed: 0,
				rejected: 0,
				model: '',
				input_tokens: 0,
				output_tokens: 0,
				error: e instanceof Error ? e.message : 'draft failed'
			};
		}
	}

	function draftLabel(captureId: number): string {
		const d = drafts[captureId];
		if (!d) return 'Draft from this capture';
		if (d.status === 'starting' || d.status === 'running') return 'Drafting…';
		if (d.status === 'finished') {
			const r = d as DraftRun;
			return `Done — ${r.proposed} proposed, ${r.rejected} rejected (${r.model || 'model?'}, ${r.input_tokens ?? 0}/${r.output_tokens ?? 0} tok)`;
		}
		return `Draft failed — ${(d as DraftRun).error || 'retry'}`;
	}

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

	onMount(() => {
		reload();
		return () => {
			for (const captureId of [...runSubs.keys()]) clearRunWatch(captureId);
		};
	});
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
				<p class="m-0 mt-2">
					<button
						class={BTN}
						type="button"
						onclick={() => draftCapture(t.id)}
						disabled={!!drafts[t.id] && drafts[t.id].status !== 'failed'}
					>
						{draftLabel(t.id)}
					</button>
				</p>
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
