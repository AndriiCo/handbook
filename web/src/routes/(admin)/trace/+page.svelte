<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';
	import { subscribe, type BusMessage } from '../../../lib/live.svelte.js';
	import Heading from '#lib/components/brand/Heading.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { fetchModelCalls, PIPELINES, type CallSummary, type ModelCall } from '#lib/admin/model-calls.js';
	import { errorText } from '#lib/admin/admin-api.js';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { clock, dateTime, fmtCompact, fmtInt, fmtMs, modelName } from '#lib/admin/format.js';

	// Trace console: Meta model calls with latency and time to first
	// token per attempt, then every watched page's capture sha256 +
	// short text, newest first. Batches of 25 through before_id paging
	// — never one giant article. Admin listener only (same /apostille
	// root).
	const api = (path: string) => `/apostille/api/v1${path}`;
	const PAGE = 25;

	let calls = $state<ModelCall[]>([]);
	let summary = $state<CallSummary | null>(null);
	let callsState = $state<'loading' | 'ready' | 'failed'>('loading');
	let callsError = $state<string | null>(null);
	let pipeline = $state<string | null>(null);

	async function loadCalls(quiet = false) {
		if (!quiet) callsState = 'loading';
		try {
			const out = await fetchModelCalls(pipeline);
			calls = out.calls;
			summary = out.summary;
			callsState = 'ready';
		} catch (e) {
			callsError = errorText(e, 'Model calls failed to load.');
			if (!quiet) callsState = 'failed';
		}
	}

	function pick(next: string | null) {
		pipeline = next;
		void loadCalls();
	}

	const maxLatency = $derived(Math.max(1, ...calls.map((c) => c.latency_ms ?? 0)));
	const OUTCOME: Record<string, string> = {
		ok: 'ok',
		refusal: 'refused',
		length: 'length cap',
		http_4xx: 'HTTP 4xx',
		http_5xx: 'HTTP 5xx',
		timeout: 'timed out',
		parse_error: 'parse error'
	};

	function runCalls(runId: number): ModelCall[] {
		return calls.filter((c) => c.run_id === runId);
	}

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
		cached_tokens: number;
		reasoning_tokens: number;
		error: string;
		attempts: Attempt[];
	}

	interface Attempt {
		n: number;
		drafted: number;
		proposed: number;
		rejected: number;
		kinds: string[];
		input_tokens: number;
		output_tokens: number;
		cached_tokens: number;
		reasoning_tokens: number;
	}

	function tokenCount(src: any, name: string): number {
		return typeof src?.[name] === 'number' ? src[name] : 0;
	}

	// Attempts from a `run_finished` step or the run registry read-back
	// (the demo socket sends none — the chain simply stays hidden).
	function attemptList(src: any): Attempt[] {
		if (!Array.isArray(src?.attempts)) return [];
		const out: Attempt[] = [];
		for (const a of src.attempts) {
			if (!a || typeof a?.n !== 'number') continue;
			out.push({
				n: a.n,
				drafted: tokenCount(a, 'drafted'),
				proposed: tokenCount(a, 'proposed'),
				rejected: tokenCount(a, 'rejected'),
				kinds: Array.isArray(a?.kinds)
					? a.kinds.filter((k: unknown): k is string => typeof k === 'string')
					: [],
				input_tokens: tokenCount(a, 'input_tokens'),
				output_tokens: tokenCount(a, 'output_tokens'),
				cached_tokens: tokenCount(a, 'cached_tokens'),
				reasoning_tokens: tokenCount(a, 'reasoning_tokens')
			});
		}
		return out.sort((x, y) => x.n - y.n);
	}

	// "attempt 1: 2 rejected (value not in quote) → retry → attempt 2:
	// 3 proposed" — null when the run predates attempt recording.
	function attemptLine(run: DraftRun): string | null {
		if (!run.attempts || run.attempts.length === 0) return null;
		return run.attempts
			.map((a) =>
				a.rejected > 0
					? `attempt ${a.n}: ${a.rejected} rejected${a.kinds.length ? ` (${a.kinds.join(', ')})` : ''}`
					: `attempt ${a.n}: ${a.proposed} proposed`
			)
			.join(' → retry → ');
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
			input_tokens: tokenCount(src, 'input_tokens'),
			output_tokens: tokenCount(src, 'output_tokens'),
			cached_tokens: tokenCount(src, 'cached_tokens'),
			reasoning_tokens: tokenCount(src, 'reasoning_tokens'),
			error: '',
			attempts: attemptList(src)
		};
		clearRunWatch(captureId);
		// The run's per-attempt latency rows land with it: re-read them.
		void loadCalls(true);
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
			cached_tokens: 0,
			reasoning_tokens: 0,
			error,
			attempts: []
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
				cached_tokens: 0,
				reasoning_tokens: 0,
				error: e instanceof Error ? e.message : 'draft failed',
				attempts: []
			};
		}
	}

	function draftLabel(captureId: number): string {
		const d = drafts[captureId];
		if (!d) return 'Draft from this capture';
		if (d.status === 'starting' || d.status === 'running') return 'Drafting…';
		if (d.status === 'finished') {
			const r = d as DraftRun;
			return `Done — ${r.proposed} proposed, ${r.rejected} rejected (${r.model || 'model?'}, ${r.input_tokens ?? 0}/${r.output_tokens ?? 0} tok, ${r.cached_tokens ?? 0} cached, ${r.reasoning_tokens ?? 0} reasoning)`;
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
		void loadCalls();
		return () => {
			for (const captureId of [...runSubs.keys()]) clearRunWatch(captureId);
		};
	});
</script>

<svelte:head>
	<title>Trace console — Admin — Apostille Handbook</title>
</svelte:head>

<div class="mt-6 flex flex-col gap-8 pb-12">
	<header class="flex flex-col gap-1">
		<Heading level={1} eyebrow="System">Trace console</Heading>
		<p class="m-0 max-w-prose text-sm text-ink-muted">
			Every Meta model attempt with its latency and time to first token, then every watched page:
			capture sha256 with the text it carried. Draft from any capture and watch the run land live.
		</p>
	</header>

	<section aria-labelledby="calls" class="flex flex-col gap-3">
		<div class="flex flex-wrap items-baseline justify-between gap-2">
			<h2 id="calls" class="m-0 text-h3 font-semibold">Model calls</h2>
			{#if summary && summary.since}
				<p class="m-0 text-xs text-ink-muted">Since {dateTime(summary.since)} · one row per HTTP attempt</p>
			{/if}
		</div>
		{#if summary}
			<dl class="m-0 grid grid-cols-2 gap-3 sm:grid-cols-4">
				<div class="rounded-md border border-line bg-card px-4 py-3 shadow-sm">
					<dt class="text-xs text-ink-muted">Calls</dt>
					<dd class="m-0 text-h3 font-semibold tabular-nums">{fmtInt(summary.calls)}</dd>
				</div>
				<div class="rounded-md border border-line bg-card px-4 py-3 shadow-sm">
					<dt class="text-xs text-ink-muted">Latency p50 / p95</dt>
					<dd class="m-0 text-h3 font-semibold tabular-nums">{fmtMs(summary.p50_ms)} / {fmtMs(summary.p95_ms)}</dd>
				</div>
				<div class="rounded-md border border-line bg-card px-4 py-3 shadow-sm">
					<dt class="text-xs text-ink-muted">Input · cached</dt>
					<dd class="m-0 text-h3 font-semibold tabular-nums">
						{fmtCompact(summary.input_tokens)} · {fmtCompact(summary.cached_tokens)}
					</dd>
				</div>
				<div class="rounded-md border border-line bg-card px-4 py-3 shadow-sm">
					<dt class="text-xs text-ink-muted">Output · reasoning</dt>
					<dd class="m-0 text-h3 font-semibold tabular-nums">
						{fmtCompact(summary.output_tokens)} · {fmtCompact(summary.reasoning_tokens)}
					</dd>
				</div>
			</dl>
		{/if}
		<div class="-mx-4 overflow-x-auto px-4 md:mx-0 md:px-0">
			<div class="flex w-max gap-1" role="group" aria-label="Filter by pipeline">
				<Button size="sm" variant={pipeline === null ? 'default' : 'outline'} aria-pressed={pipeline === null} onclick={() => pick(null)}>
					All
				</Button>
				{#each PIPELINES as p (p)}
					<Button size="sm" variant={pipeline === p ? 'default' : 'outline'} aria-pressed={pipeline === p} onclick={() => pick(p)}>
						{p}
					</Button>
				{/each}
			</div>
		</div>
		{#if callsState === 'loading'}
			<div class="flex flex-col gap-2" role="status" aria-label="Loading model calls">
				{#each [0, 1, 2] as i (i)}
					<Skeleton class="h-12 rounded-sm" />
				{/each}
			</div>
		{:else if callsState === 'failed'}
			<p class="m-0 text-sm text-ink-muted" role="status">{callsError}</p>
			<div><Button variant="outline" size="sm" onclick={() => loadCalls()}>Try again</Button></div>
		{:else if calls.length === 0}
			<p class="m-0 text-sm text-ink-muted">No calls{pipeline ? ` for ${pipeline}` : ''} yet. Draft from a capture below to start one.</p>
		{:else}
			<div class="overflow-hidden rounded-md border border-line">
				<table class="block w-full border-collapse text-sm md:table">
					<caption class="sr-only">Model calls, newest first</caption>
					<thead class="sr-only bg-surface md:not-sr-only">
						<tr class="text-left text-xs text-ink-muted">
							<th scope="col" class="px-3 py-2 font-medium">Call</th>
							<th scope="col" class="px-3 py-2 font-medium">Latency</th>
							<th scope="col" class="px-3 py-2 text-right font-medium">TTFT</th>
							<th scope="col" class="px-3 py-2 text-right font-medium">Tokens in / cached / out / reasoning</th>
							<th scope="col" class="px-3 py-2 text-right font-medium">Outcome</th>
						</tr>
					</thead>
					<tbody class="block md:table-row-group">
						{#each calls as c (c.id)}
							<tr class="flex flex-col gap-1 border-t border-line px-3 py-2 md:table-row md:px-0 md:py-0">
								<td class="md:px-3 md:py-2">
									<span class="font-semibold">{modelName(c.model)}</span>
									<span class="text-ink-muted"> · {c.pipeline}</span>
									<span class="block font-mono text-xs text-ink-muted">
										{clock(c.at)}{c.run_id ? ` · run ${c.run_id}` : ''} · attempt {c.attempt}
									</span>
								</td>
								<td class="md:w-48 md:px-3 md:py-2">
									<span class="flex items-center gap-2">
										<span class="h-2 flex-1" aria-hidden="true">
											<span
												class={`block h-full rounded-r-xs ${c.outcome === 'ok' ? 'bg-accent-strong' : 'bg-changed'}`}
												style={`width:${Math.max(2, ((c.latency_ms ?? 0) / maxLatency) * 100)}%`}
											></span>
										</span>
										<span class="w-14 text-right font-mono text-xs tabular-nums">{fmtMs(c.latency_ms)}</span>
									</span>
								</td>
								<td class="font-mono text-xs tabular-nums md:px-3 md:py-2 md:text-right">
									<span class="text-ink-muted md:sr-only">TTFT </span>{fmtMs(c.ttft_ms)}
								</td>
								<td class="font-mono text-xs tabular-nums md:px-3 md:py-2 md:text-right">
									{fmtCompact(c.input_tokens)} / {fmtCompact(c.cached_tokens)} / {fmtCompact(c.output_tokens)} / {fmtCompact(c.reasoning_tokens)}
								</td>
								<td class="md:px-3 md:py-2 md:text-right">
									<Badge variant="outline" class={c.outcome === 'ok' ? 'border-verified text-verified' : 'border-changed text-changed'}>
										{OUTCOME[c.outcome] ?? c.outcome}
									</Badge>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</section>

	<section aria-labelledby="captures" class="flex flex-col gap-3">
		<h2 id="captures" class="m-0 text-h3 font-semibold">Captures</h2>
		<form
			onsubmit={(e) => {
				e.preventDefault();
				reload();
			}}
			class="flex max-w-md flex-wrap items-end gap-2"
		>
			<label class="flex min-w-0 flex-1 flex-col gap-1 text-xs font-medium text-ink-muted" for="trace-authority">
				Authority slug (optional)
				<Input id="trace-authority" class="h-9" bind:value={authority} autocomplete="off" />
			</label>
			<Button type="submit" variant="outline" size="lg">Filter</Button>
		</form>

		{#if loadState === 'loading'}
			<ul class="m-0 flex list-none flex-col gap-2 p-0" aria-label="Captures">
				{#each [0, 1, 2, 3] as i (i)}
					<li aria-hidden="true">
						<Skeleton class="h-24 rounded-md" />
					</li>
				{/each}
			</ul>
		{:else if loadState === 'failed'}
			<Notice variant="error" heading="Trace failed to load.">
				<p class="m-0">{loadError}</p>
				<p class="mt-3 mb-0"><Button variant="outline" onclick={reload}>Try again</Button></p>
			</Notice>
		{:else if traces.length === 0}
			<Notice variant="info" heading="No captures yet.">
				<p class="m-0">Run a fetch sweep first — captures appear here newest-first.</p>
			</Notice>
		{:else}
			<ul class="m-0 flex list-none flex-col gap-2 p-0">
				{#each traces as t (t.id)}
					{@const d = drafts[t.id]}
					{@const chain = d && d.status === 'finished' ? attemptLine(d as DraftRun) : null}
					{@const runRows = d && d.status === 'finished' ? runCalls((d as DraftRun).id) : []}
					<li>
						<Card.Root class="gap-2 py-3 shadow-sm">
							<Card.Content class="flex flex-col gap-2">
								<p class="m-0 flex flex-wrap items-baseline gap-x-2 text-sm">
									<span class="font-semibold">#{t.id} · {t.authority_slug}</span>
									<span class="font-mono text-xs text-ink-muted">{day(t.captured_at)} · {t.kind} · {t.bytes_raw} B · {t.trust}</span>
								</p>
								<p class="m-0 font-mono text-xs break-all text-ink-muted" title={t.sha256}>
									sha256:{t.sha256.slice(0, 16)}…
								</p>
								<p class="m-0 text-sm">{t.excerpt || '— no text extracted —'}</p>
								<div class="flex flex-wrap items-center gap-2">
									{#if d && d.status === 'finished'}
										<p class="m-0 inline-flex items-center gap-2 rounded-sm bg-verified-bg px-3 py-2 text-sm font-semibold text-verified" role="status">
											<CheckIcon class="size-4 flex-none" aria-hidden="true" />{draftLabel(t.id)}
										</p>
									{:else}
										<Button
											class="h-auto min-h-9 text-left whitespace-normal"
											size="lg"
											onclick={() => draftCapture(t.id)}
											disabled={!!d && d.status !== 'failed'}
										>
											{draftLabel(t.id)}
										</Button>
									{/if}
								</div>
								{#if chain}
									<p class="m-0 font-mono text-xs text-ink-muted">{chain}</p>
								{/if}
								{#if runRows.length > 0}
									<ul class="m-0 flex list-none flex-col gap-1 p-0" aria-label={`Model calls for run ${(d as DraftRun).id}`}>
										{#each runRows as c (c.id)}
											<li class="font-mono text-xs text-ink-muted">
												{modelName(c.model)} attempt {c.attempt}: {fmtMs(c.latency_ms)} total, first token {fmtMs(c.ttft_ms)}
											</li>
										{/each}
									</ul>
								{/if}
							</Card.Content>
						</Card.Root>
					</li>
				{/each}
			</ul>
			{#if loadError}
				<Notice variant="warning" heading="Older captures failed to load.">
					<p class="m-0">{loadError}</p>
				</Notice>
			{/if}
			{#if !exhausted}
				<div>
					<Button variant="outline" onclick={more} disabled={loadingMore}>
						{loadingMore ? 'Working…' : 'Load older captures'}
					</Button>
				</div>
			{/if}
		{/if}
	</section>
</div>
