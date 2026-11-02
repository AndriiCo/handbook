<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';
	import { authError, readErrorBody } from '../../../lib/admin/http-errors.js';

	// Workers management against the admin workers API (admin-workers.md
	// §8): status, per-authority ingest state, capture list with retry,
	// one-off fetch, manual watch-sweep. Admin build only; same literal
	// /apostille root as the triage page.
	const api = (path: string) => `/apostille/api/v1${path}`;

	interface Status {
		mode: string;
		backend: string;
		timeoutSecs: number;
		maxBody: number;
		captures: number;
		sources: number;
		failures: number;
		lastSweep: { at: number; actor: string; summary: string } | null;
	}

	interface Authority {
		slug: string;
		name: string;
		sources: number;
		captures: number;
		lastCaptureAt: number | null;
		failures: number;
		lastAnalysisAt: number | null;
		lastPromptVersion: string | null;
	}

	interface Capture {
		id: number;
		authority: string;
		uri: string | null;
		capturedAt: number;
		kind: string;
		bytes: number;
		sha: string;
		trust: string;
		busy: boolean;
		note: string | null;
	}

	interface SweepResult {
		slug: string;
		url: string;
		outcome: string;
		captureId: number | null;
		detail: string | null;
	}

	let status = $state<Status | null>(null);
	let authorities = $state<Authority[]>([]);
	let captures = $state<Capture[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);

	let filterAuthority = $state('');
	let filterKind = $state('');
	let filterTrust = $state('');

	let fetchSlug = $state('');
	let fetchUrl = $state('');
	let fetchKind = $state('website_official');
	let fetchTrust = $state('official');
	let fetchBusy = $state(false);
	let fetchNote = $state<string | null>(null);

	let sweepText = $state('');
	let sweepBusy = $state(false);
	let sweepNote = $state<string | null>(null);
	let sweepResults = $state<SweepResult[]>([]);
	let sweepSummary = $state<string | null>(null);

	function authFailed(res: Response): boolean {
		return res.status === 401 || res.status === 403;
	}

	function normalizeStatus(data: any): Status | null {
		if (!data || data.mode !== 'manual') return null;
		const q = data.queue ?? {};
		const f = data.fetch ?? {};
		const sweep = data.last_sweep;
		return {
			mode: 'manual',
			backend: typeof f.backend === 'string' ? f.backend : '?',
			timeoutSecs: typeof f.timeout_secs === 'number' ? f.timeout_secs : 0,
			maxBody: typeof f.max_body_bytes === 'number' ? f.max_body_bytes : 0,
			captures: typeof q.captures === 'number' ? q.captures : 0,
			sources: typeof q.sources === 'number' ? q.sources : 0,
			failures: typeof q.open_fetch_failures === 'number' ? q.open_fetch_failures : 0,
			lastSweep:
				sweep && typeof sweep.at === 'number'
					? {
							at: sweep.at,
							actor: typeof sweep.actor === 'string' ? sweep.actor : '?',
							summary: typeof sweep.summary === 'string' ? sweep.summary : ''
						}
					: null
		};
	}

	function normalizeAuthority(row: any): Authority | null {
		if (!row || typeof row.slug !== 'string') return null;
		return {
			slug: row.slug,
			name: typeof row.name === 'string' ? row.name : row.slug,
			sources: typeof row.sources === 'number' ? row.sources : 0,
			captures: typeof row.captures === 'number' ? row.captures : 0,
			lastCaptureAt: typeof row.last_capture_at === 'number' ? row.last_capture_at : null,
			failures: typeof row.open_fetch_failures === 'number' ? row.open_fetch_failures : 0,
			lastAnalysisAt: typeof row.last_analysis_at === 'number' ? row.last_analysis_at : null,
			lastPromptVersion:
				typeof row.last_prompt_version === 'string' ? row.last_prompt_version : null
		};
	}

	function normalizeCapture(row: any): Capture | null {
		if (!row || typeof row.id !== 'number') return null;
		return {
			id: row.id,
			authority: typeof row.authority_slug === 'string' ? row.authority_slug : '?',
			uri: typeof row.source_uri === 'string' ? row.source_uri : null,
			capturedAt: typeof row.captured_at === 'number' ? row.captured_at : 0,
			kind: typeof row.kind === 'string' ? row.kind : '?',
			bytes: typeof row.bytes_raw === 'number' ? row.bytes_raw : 0,
			sha: typeof row.sha256 === 'string' ? row.sha256 : '',
			trust: typeof row.trust === 'string' ? row.trust : '?',
			busy: false,
			note: null
		};
	}

	function isCapture(value: Capture | null): value is Capture {
		return value !== null;
	}

	function isAuthority(value: Authority | null): value is Authority {
		return value !== null;
	}

	async function load() {
		loadState = 'loading';
		loadError = null;
		try {
			const params = new URLSearchParams();
			if (filterAuthority.trim()) params.set('authority', filterAuthority.trim());
			if (filterKind.trim()) params.set('kind', filterKind.trim());
			if (filterTrust.trim()) params.set('trust', filterTrust.trim());
			params.set('limit', '100');
			const query = params.size > 0 ? `?${params}` : '';
			const [statusRes, authRes, capRes] = await Promise.all([
				fetch(api('/workers/status'), { credentials: 'same-origin' }),
				fetch(api('/workers/authorities'), { credentials: 'same-origin' }),
				fetch(api(`/workers/captures${query}`), { credentials: 'same-origin' })
			]);
			if ([statusRes, authRes, capRes].some(authFailed)) {
				// Auth failures show the server's answer verbatim (the
				// 403 body names the detected IP) via the shared helper.
				const first = [statusRes, authRes, capRes].find((r) => authFailed(r)) ?? statusRes;
				loadError = authError(first.status, await readErrorBody(first));
			} else if (!statusRes.ok || !authRes.ok || !capRes.ok) {
				throw new Error('one section failed to load');
			} else {
				status = normalizeStatus(await statusRes.json());
				const authData = await authRes.json();
				authorities = (Array.isArray(authData?.authorities) ? authData.authorities : [])
					.map(normalizeAuthority)
					.filter(isAuthority);
				const capData = await capRes.json();
				captures = (Array.isArray(capData?.captures) ? capData.captures : [])
					.map(normalizeCapture)
					.filter(isCapture);
				if (!status) throw new Error('unrecognized status shape');
			}
			loadState = loadError ? 'failed' : 'ready';
		} catch {
			loadError = 'Workers service unreachable — is the API server running?';
			loadState = 'failed';
		}
	}

	onMount(load);

	async function retry(item: Capture) {
		item.busy = true;
		item.note = null;
		try {
			const res = await fetch(api(`/workers/captures/${item.id}/retry`), {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				credentials: 'same-origin',
				body: JSON.stringify({})
			});
			if (authFailed(res)) {
				item.note = 'Staff session expired — log in again, then retry.';
			} else if (res.status === 409) {
				item.note = 'No source to re-fetch (unattributed capture).';
			} else if (res.status === 502) {
				const data = await res.json().catch(() => null);
				item.note = `Fetch failed — failure task recorded (${data?.error ?? 'unknown'}).`;
			} else if (!res.ok) {
				item.note = `Retry failed: HTTP ${res.status}. Nothing changed.`;
			} else {
				const data = await res.json();
				if (data?.outcome === 'skipped') {
					item.note = `Skipped: ${data?.reason ?? 'robots disallow'}.`;
				} else {
					item.note = `${data?.outcome ?? 'done'} → capture #${data?.capture_id ?? '?'}.`;
				}
			}
		} catch {
			item.note = 'Workers service unreachable — is the API server running?';
		} finally {
			item.busy = false;
		}
	}

	async function runFetch() {
		fetchBusy = true;
		fetchNote = null;
		try {
			const res = await fetch(api('/workers/fetch'), {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				credentials: 'same-origin',
				body: JSON.stringify({
					slug: fetchSlug.trim(),
					url: fetchUrl.trim(),
					source_kind: fetchKind,
					trust: fetchTrust
				})
			});
			if (authFailed(res)) {
				fetchNote = 'Staff session expired — log in again, then retry.';
			} else if (res.status === 404) {
				fetchNote = 'Unknown authority slug.';
			} else if (res.status === 422) {
				fetchNote = 'Rejected (422) — URL must be absolute http(s).';
			} else if (res.status === 502) {
				const data = await res.json().catch(() => null);
				fetchNote = `Fetch failed — failure task recorded (${data?.error ?? 'unknown'}).`;
			} else if (!res.ok) {
				fetchNote = `Fetch failed: HTTP ${res.status}. Nothing changed.`;
			} else {
				const data = await res.json();
				fetchNote =
					data?.outcome === 'skipped'
						? `Skipped: ${data?.reason ?? 'robots disallow'}.`
						: `${data?.outcome ?? 'done'} → capture #${data?.capture_id ?? '?'}.`;
				await load();
			}
		} catch {
			fetchNote = 'Workers service unreachable — is the API server running?';
		} finally {
			fetchBusy = false;
		}
	}

	function parseSweepLines(): { slug: string; url: string }[] | null {
		const items: { slug: string; url: string }[] = [];
		for (const line of sweepText.split('\n')) {
			const trimmed = line.trim();
			if (!trimmed) continue;
			const parts = trimmed.split('|').map((part) => part.trim());
			if (parts.length !== 2 || !parts[0] || !parts[1]) return null;
			items.push({ slug: parts[0], url: parts[1] });
		}
		return items;
	}

	async function runSweep() {
		sweepBusy = true;
		sweepNote = null;
		sweepResults = [];
		sweepSummary = null;
		const items = parseSweepLines();
		if (!items || items.length === 0 || items.length > 10) {
			sweepNote = 'Need 1–10 lines shaped “slug | https://…”.';
			sweepBusy = false;
			return;
		}
		try {
			const res = await fetch(api('/workers/sweep'), {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				credentials: 'same-origin',
				body: JSON.stringify({ items })
			});
			if (authFailed(res)) {
				sweepNote = 'Staff session expired — log in again, then retry.';
			} else if (res.status === 404 || res.status === 422) {
				const data = await res.json().catch(() => null);
				sweepNote = `Rejected (${res.status}): ${data?.error ?? 'bad items'}. Nothing fetched.`;
			} else if (!res.ok) {
				sweepNote = `Sweep failed: HTTP ${res.status}.`;
			} else {
				const data = await res.json();
				sweepResults = Array.isArray(data?.results)
					? data.results.map(
							(row: any): SweepResult => ({
								slug: typeof row?.slug === 'string' ? row.slug : '?',
								url: typeof row?.url === 'string' ? row.url : '?',
								outcome: typeof row?.outcome === 'string' ? row.outcome : '?',
								captureId: typeof row?.capture_id === 'number' ? row.capture_id : null,
								detail: typeof row?.detail === 'string' ? row.detail : null
							})
						)
					: [];
				const s = data?.summary;
				sweepSummary = s
					? `${s.total} items: ${s.created} created, ${s.duplicate} duplicate, ${s.failed} failed, ${s.skipped} skipped.`
					: null;
				await load();
			}
		} catch {
			sweepNote = 'Workers service unreachable — is the API server running?';
		} finally {
			sweepBusy = false;
		}
	}

	function stamp(at: number | null): string {
		if (at === null || at <= 0) return '—';
		return new Date(at * 1000).toISOString().replace('T', ' ').slice(0, 19) + 'Z';
	}
</script>

<svelte:head>
	<title>Workers — Admin — Apostille Handbook</title>
</svelte:head>

<h1>Workers</h1>
<p class="max-w-[60ch] text-ink-muted">
	Manual drive over the URL download/analysis ingest paths. Every fetch runs now, inside the
	request — there is no background scheduler.
</p>

{#if loadState === 'loading'}
	<Notice variant="info" heading="Loading…"><p>Fetching worker status.</p></Notice>
{:else if loadState === 'failed'}
	<Notice variant="warning" heading="Workers unavailable."><p>{loadError}</p></Notice>
	<p>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
			onclick={load}>Retry</button
		>
	</p>
{:else if status}
	<section class="mt-8 border-t border-line pt-4" aria-label="Status">
		<h2 class="m-0 mb-3 text-[1.1rem]">Status</h2>
		<dl class="m-0 grid gap-[0.4rem]">
			<div class="grid grid-cols-[12rem_1fr] gap-4"><dt class="text-ink-muted">Mode</dt><dd class="m-0">{status.mode}</dd></div>
			<div class="grid grid-cols-[12rem_1fr] gap-4">
				<dt class="text-ink-muted">Fetch backend</dt><dd class="m-0 font-mono">{status.backend}</dd>
			</div>
			<div class="grid grid-cols-[12rem_1fr] gap-4">
				<dt class="text-ink-muted">Fetch timeout</dt><dd class="m-0">{status.timeoutSecs}s</dd>
			</div>
			<div class="grid grid-cols-[12rem_1fr] gap-4">
				<dt class="text-ink-muted">Body cap</dt><dd class="m-0">{status.maxBody} bytes</dd>
			</div>
			<div class="grid grid-cols-[12rem_1fr] gap-4">
				<dt class="text-ink-muted">Captures</dt><dd class="m-0">{status.captures}</dd>
			</div>
			<div class="grid grid-cols-[12rem_1fr] gap-4">
				<dt class="text-ink-muted">Sources</dt><dd class="m-0">{status.sources}</dd>
			</div>
			<div class="grid grid-cols-[12rem_1fr] gap-4">
				<dt class="text-ink-muted">Open fetch failures</dt><dd class="m-0">{status.failures}</dd>
			</div>
			<div class="grid grid-cols-[12rem_1fr] gap-4">
				<dt class="text-ink-muted">Last sweep</dt>
				<dd class="m-0">
					{#if status.lastSweep}
						{stamp(status.lastSweep.at)} by {status.lastSweep.actor} — {status.lastSweep.summary}
					{:else}
						never run
					{/if}
				</dd>
			</div>
		</dl>
	</section>

	<section class="mt-8 border-t border-line pt-4" aria-label="Authorities">
		<h2 class="m-0 mb-3 text-[1.1rem]">Authorities</h2>
		{#if authorities.length === 0}
			<p>No authorities yet.</p>
		{:else}
			<table class="w-full border-collapse text-[0.9rem]">
				<thead>
					<tr>
						<th class="border-b border-line px-3 py-2 text-left" scope="col">Authority</th>
						<th class="border-b border-line px-3 py-2 text-left" scope="col">Sources</th>
						<th class="border-b border-line px-3 py-2 text-left" scope="col">Captures</th>
						<th class="border-b border-line px-3 py-2 text-left" scope="col">Last capture</th>
						<th class="border-b border-line px-3 py-2 text-left" scope="col">Fetch failures</th>
						<th class="border-b border-line px-3 py-2 text-left" scope="col">Last analysis</th>
					</tr>
				</thead>
				<tbody>
					{#each authorities as row (row.slug)}
						<tr>
							<th class="border-b border-line px-3 py-2 text-left font-semibold" scope="row">
								{row.name} <span class="font-mono text-ink-muted">{row.slug}</span>
							</th>
							<td class="border-b border-line px-3 py-2 text-left">{row.sources}</td>
							<td class="border-b border-line px-3 py-2 text-left">{row.captures}</td>
							<td class="border-b border-line px-3 py-2 text-left">{stamp(row.lastCaptureAt)}</td>
							<td class="border-b border-line px-3 py-2 text-left">{row.failures}</td>
							<td class="border-b border-line px-3 py-2 text-left">
								{#if row.lastAnalysisAt}{stamp(row.lastAnalysisAt)} ({row.lastPromptVersion ?? '?'}){:else}—{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		{/if}
	</section>

	<section class="mt-8 border-t border-line pt-4" aria-label="Captures">
		<h2 class="m-0 mb-3 text-[1.1rem]">Captures</h2>
		<form
			class="flex flex-wrap items-end gap-3"
			onsubmit={(event) => {
				event.preventDefault();
				load();
			}}
		>
			<label class="grid gap-[0.3rem] text-sm text-ink-muted"
				>Authority
				<input
					class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
					type="text"
					bind:value={filterAuthority}
					placeholder="slug"
				/></label
			>
			<label class="grid gap-[0.3rem] text-sm text-ink-muted"
				>Kind
				<input
					class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
					type="text"
					bind:value={filterKind}
					placeholder="warc"
				/></label
			>
			<label class="grid gap-[0.3rem] text-sm text-ink-muted"
				>Trust
				<input
					class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
					type="text"
					bind:value={filterTrust}
					placeholder="official"
				/></label
			>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
				type="submit">Apply</button
			>
		</form>
		{#if captures.length === 0}
			<p>No captures match.</p>
		{:else}
			<ul class="mt-4 mb-0 grid list-none gap-4 p-0">
				{#each captures as item (item.id)}
					<li class="rounded-sm border border-line px-4 py-[0.9rem]">
						<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
							<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">
								{item.authority} · {item.kind} · {item.trust}
							</span>
							<span class="ml-auto font-mono text-sm">#{item.id}</span>
						</div>
						<p class="mt-2 mb-0 font-mono wrap-anywhere">
							{item.uri ?? '(unattributed — nothing to re-fetch)'}
						</p>
						<p class="mt-1 mb-0 text-sm text-ink-muted">
							{stamp(item.capturedAt)} · {item.bytes} bytes · sha {item.sha.slice(0, 16)}…
						</p>
						{#if item.note}
							<p class="rounded-sm border border-line bg-surface px-[0.8rem] py-[0.6rem]" role="status">
								{item.note}
							</p>
						{/if}
						<p class="mb-0 flex gap-3">
							<button
								class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
								onclick={() => retry(item)}
								disabled={item.busy || !item.uri}
							>
								{item.busy ? 'Fetching…' : 'Retry'}
							</button>
						</p>
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<section class="mt-8 border-t border-line pt-4" aria-label="Fetch">
		<h2 class="m-0 mb-3 text-[1.1rem]">Fetch one URL</h2>
		<form
			class="grid max-w-[40rem] gap-3"
			onsubmit={(event) => {
				event.preventDefault();
				runFetch();
			}}
		>
			<label class="grid gap-[0.3rem] text-sm text-ink-muted"
				>Authority slug
				<input
					class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
					type="text"
					bind:value={fetchSlug}
					placeholder="usa-wa-sos"
					required
				/></label
			>
			<label class="grid gap-[0.3rem] text-sm text-ink-muted"
				>URL
				<input
					class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
					type="url"
					bind:value={fetchUrl}
					placeholder="https://…"
					required
				/></label
			>
			<div class="flex flex-wrap gap-3">
				<label class="grid flex-1 basis-48 gap-[0.3rem] text-sm text-ink-muted">
					Source kind
					<select
						class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
						bind:value={fetchKind}
					>
						<option value="website_official">website_official</option>
						<option value="website_thirdparty">website_thirdparty</option>
					</select>
				</label>
				<label class="grid flex-1 basis-48 gap-[0.3rem] text-sm text-ink-muted">
					Trust
					<select
						class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 text-ink"
						bind:value={fetchTrust}
					>
						<option value="official">official</option>
						<option value="staff_verified">staff_verified</option>
						<option value="community">community</option>
					</select>
				</label>
			</div>
			<p class="mb-0 flex gap-3">
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
					type="submit"
					disabled={fetchBusy}>{fetchBusy ? 'Fetching…' : 'Fetch now'}</button
				>
			</p>
		</form>
		{#if fetchNote}
			<p class="rounded-sm border border-line bg-surface px-[0.8rem] py-[0.6rem]" role="status">
				{fetchNote}
			</p>
		{/if}
	</section>

	<section class="mt-8 border-t border-line pt-4" aria-label="Sweep">
		<h2 class="m-0 mb-3 text-[1.1rem]">Watch sweep</h2>
		<p>Runs up to 10 fetches sequentially, right now. One per line:</p>
		<pre class="overflow-x-auto rounded-sm border border-line bg-surface px-[0.8rem] py-[0.6rem] font-mono text-sm">usa-wa-sos | https://example.com/synthetic/1</pre>
		<form
			class="grid max-w-[40rem] gap-3"
			onsubmit={(event) => {
				event.preventDefault();
				runSweep();
			}}
		>
			<label class="grid gap-[0.3rem] text-sm text-ink-muted">
				Sweep items
				<textarea
					class="min-h-11 rounded-sm border border-line bg-surface px-3 py-2 font-mono text-sm text-ink"
					bind:value={sweepText}
					rows="6"
					placeholder="slug | https://…"
				></textarea>
			</label>
			<p class="mb-0 flex gap-3">
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
					type="submit"
					disabled={sweepBusy}>{sweepBusy ? 'Sweeping…' : 'Run sweep'}</button
				>
			</p>
		</form>
		{#if sweepNote}
			<p class="rounded-sm border border-line bg-surface px-[0.8rem] py-[0.6rem]" role="status">
				{sweepNote}
			</p>
		{/if}
		{#if sweepSummary}
			<p class="rounded-sm border border-line bg-surface px-[0.8rem] py-[0.6rem]" role="status">
				{sweepSummary}
			</p>
			<ul class="mt-3 mb-0 grid list-none gap-[0.4rem] p-0 text-[0.9rem]">
				{#each sweepResults as row, index (`${row.slug}-${row.url}-${index}`)}
					<li class="flex flex-wrap gap-x-3 gap-y-1 border-b border-line pb-[0.4rem]">
						<span class="font-mono">{row.outcome}</span>
						{row.slug} · {row.url}
						{#if row.captureId !== null}<span class="font-mono">→ #{row.captureId}</span>{/if}
						{#if row.detail}<span class="text-ink-muted">{row.detail}</span>{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</section>
{/if}

