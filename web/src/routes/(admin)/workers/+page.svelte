<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';

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
				fetch(api('/admin/workers/status'), { credentials: 'same-origin' }),
				fetch(api('/admin/workers/authorities'), { credentials: 'same-origin' }),
				fetch(api(`/admin/workers/captures${query}`), { credentials: 'same-origin' })
			]);
			if ([statusRes, authRes, capRes].some(authFailed)) {
				loadError = 'Staff session required — log in on the admin host first.';
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
			const res = await fetch(api(`/admin/workers/captures/${item.id}/retry`), {
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
			const res = await fetch(api('/admin/workers/fetch'), {
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
			const res = await fetch(api('/admin/workers/sweep'), {
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
<p class="lede">Manual drive over the URL download/analysis ingest paths. Every fetch runs now, inside the request — there is no background scheduler.</p>

{#if loadState === 'loading'}
	<Notice variant="info" heading="Loading…"><p>Fetching worker status.</p></Notice>
{:else if loadState === 'failed'}
	<Notice variant="warning" heading="Workers unavailable."><p>{loadError}</p></Notice>
	<p><button onclick={load}>Retry</button></p>
{:else if status}
	<section aria-label="Status">
		<h2>Status</h2>
		<dl class="facts">
			<div><dt>Mode</dt><dd>{status.mode}</dd></div>
			<div><dt>Fetch backend</dt><dd class="mono">{status.backend}</dd></div>
			<div><dt>Fetch timeout</dt><dd>{status.timeoutSecs}s</dd></div>
			<div><dt>Body cap</dt><dd>{status.maxBody} bytes</dd></div>
			<div><dt>Captures</dt><dd>{status.captures}</dd></div>
			<div><dt>Sources</dt><dd>{status.sources}</dd></div>
			<div><dt>Open fetch failures</dt><dd>{status.failures}</dd></div>
			<div>
				<dt>Last sweep</dt>
				<dd>
					{#if status.lastSweep}
						{stamp(status.lastSweep.at)} by {status.lastSweep.actor} — {status.lastSweep.summary}
					{:else}
						never run
					{/if}
				</dd>
			</div>
		</dl>
	</section>

	<section aria-label="Authorities">
		<h2>Authorities</h2>
		{#if authorities.length === 0}
			<p>No authorities yet.</p>
		{:else}
			<table>
				<thead>
					<tr>
						<th scope="col">Authority</th>
						<th scope="col">Sources</th>
						<th scope="col">Captures</th>
						<th scope="col">Last capture</th>
						<th scope="col">Fetch failures</th>
						<th scope="col">Last analysis</th>
					</tr>
				</thead>
				<tbody>
					{#each authorities as row (row.slug)}
						<tr>
							<th scope="row">{row.name} <span class="mono dim">{row.slug}</span></th>
							<td>{row.sources}</td>
							<td>{row.captures}</td>
							<td>{stamp(row.lastCaptureAt)}</td>
							<td>{row.failures}</td>
							<td>
								{#if row.lastAnalysisAt}{stamp(row.lastAnalysisAt)} ({row.lastPromptVersion ?? '?'}){:else}—{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		{/if}
	</section>

	<section aria-label="Captures">
		<h2>Captures</h2>
		<form
			class="filters"
			onsubmit={(event) => {
				event.preventDefault();
				load();
			}}
		>
			<label>Authority <input type="text" bind:value={filterAuthority} placeholder="slug" /></label>
			<label>Kind <input type="text" bind:value={filterKind} placeholder="warc" /></label>
			<label>Trust <input type="text" bind:value={filterTrust} placeholder="official" /></label>
			<button type="submit">Apply</button>
		</form>
		{#if captures.length === 0}
			<p>No captures match.</p>
		{:else}
			<ul class="queue">
				{#each captures as item (item.id)}
					<li>
						<div class="item-head">
							<span class="kind">{item.authority} · {item.kind} · {item.trust}</span>
							<span class="status mono">#{item.id}</span>
						</div>
						<p class="uri mono">{item.uri ?? '(unattributed — nothing to re-fetch)'}</p>
						<p class="meta">{stamp(item.capturedAt)} · {item.bytes} bytes · sha {item.sha.slice(0, 16)}…</p>
						{#if item.note}
							<p class="note" role="status">{item.note}</p>
						{/if}
						<p class="actions">
							<button onclick={() => retry(item)} disabled={item.busy || !item.uri}>
								{item.busy ? 'Fetching…' : 'Retry'}
							</button>
						</p>
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<section aria-label="Fetch">
		<h2>Fetch one URL</h2>
		<form
			class="stack"
			onsubmit={(event) => {
				event.preventDefault();
				runFetch();
			}}
		>
			<label>Authority slug <input type="text" bind:value={fetchSlug} placeholder="usa-wa-sos" required /></label>
			<label>URL <input type="url" bind:value={fetchUrl} placeholder="https://…" required /></label>
			<div class="row">
				<label>
					Source kind
					<select bind:value={fetchKind}>
						<option value="website_official">website_official</option>
						<option value="website_thirdparty">website_thirdparty</option>
					</select>
				</label>
				<label>
					Trust
					<select bind:value={fetchTrust}>
						<option value="official">official</option>
						<option value="staff_verified">staff_verified</option>
						<option value="community">community</option>
					</select>
				</label>
			</div>
			<p class="actions"><button type="submit" disabled={fetchBusy}>{fetchBusy ? 'Fetching…' : 'Fetch now'}</button></p>
		</form>
		{#if fetchNote}
			<p class="note" role="status">{fetchNote}</p>
		{/if}
	</section>

	<section aria-label="Sweep">
		<h2>Watch sweep</h2>
		<p>Runs up to 10 fetches sequentially, right now. One per line:</p>
		<pre class="example">usa-wa-sos | https://example.com/synthetic/1</pre>
		<form
			class="stack"
			onsubmit={(event) => {
				event.preventDefault();
				runSweep();
			}}
		>
			<label>
				Sweep items
				<textarea bind:value={sweepText} rows="6" placeholder="slug | https://…"></textarea>
			</label>
			<p class="actions"><button type="submit" disabled={sweepBusy}>{sweepBusy ? 'Sweeping…' : 'Run sweep'}</button></p>
		</form>
		{#if sweepNote}
			<p class="note" role="status">{sweepNote}</p>
		{/if}
		{#if sweepSummary}
			<p class="note" role="status">{sweepSummary}</p>
			<ul class="results">
				{#each sweepResults as row, index (`${row.slug}-${row.url}-${index}`)}
					<li>
						<span class="mono">{row.outcome}</span>
						{row.slug} · {row.url}
						{#if row.captureId !== null}<span class="mono">→ #{row.captureId}</span>{/if}
						{#if row.detail}<span class="dim">{row.detail}</span>{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</section>
{/if}

<style>
	.lede {
		color: var(--text-muted);
		max-width: 60ch;
	}
	section {
		margin-top: 2rem;
		border-top: 1px solid var(--border);
		padding-top: 1rem;
	}
	h2 {
		font-size: 1.1rem;
		margin: 0 0 0.75rem;
	}
	.facts {
		display: grid;
		gap: 0.4rem;
		margin: 0;
	}
	.facts div {
		display: grid;
		grid-template-columns: 12rem 1fr;
		gap: 1rem;
	}
	.facts dt {
		color: var(--text-muted);
	}
	.facts dd {
		margin: 0;
	}
	.mono {
		font-family: var(--font-mono);
	}
	.dim {
		color: var(--text-muted);
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.9rem;
	}
	th,
	td {
		text-align: left;
		padding: 0.5rem 0.75rem;
		border-bottom: 1px solid var(--border);
	}
	tbody th {
		font-weight: 600;
	}
	.queue {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0;
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
	.uri {
		overflow-wrap: anywhere;
		margin: 0.5rem 0 0;
	}
	.meta {
		color: var(--text-muted);
		font-size: 0.875rem;
		margin: 0.25rem 0 0;
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
	button:disabled {
		opacity: 0.55;
		cursor: not-allowed;
	}
	.filters {
		display: flex;
		flex-wrap: wrap;
		align-items: end;
		gap: 0.75rem;
	}
	.stack {
		display: grid;
		gap: 0.75rem;
		max-width: 40rem;
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem;
	}
	.row label {
		flex: 1 1 12rem;
	}
	label {
		display: grid;
		gap: 0.3rem;
		font-size: 0.875rem;
		color: var(--text-muted);
	}
	input[type='text'],
	input[type='url'],
	select,
	textarea {
		font: inherit;
		color: var(--text);
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.5rem 0.75rem;
		min-height: 44px;
	}
	textarea {
		font-family: var(--font-mono);
		font-size: 0.875rem;
	}
	.example {
		font-family: var(--font-mono);
		font-size: 0.875rem;
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.6rem 0.8rem;
		overflow-x: auto;
	}
	.results {
		list-style: none;
		margin: 0.75rem 0 0;
		padding: 0;
		display: grid;
		gap: 0.4rem;
		font-size: 0.9rem;
	}
	.results li {
		border-bottom: 1px solid var(--border);
		padding-bottom: 0.4rem;
		display: flex;
		flex-wrap: wrap;
		gap: 0.25rem 0.75rem;
	}
</style>
