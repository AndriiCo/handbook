<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Notice from '#lib/public-ui/Notice.svelte';
	import Heading from '#lib/components/brand/Heading.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { csrfHeaders } from '#lib/admin/config-client.js';
	import { authError, readErrorBody } from '#lib/admin/http-errors.js';

	// SA-14 source history (P394): every capture of one watched source,
	// newest first, and the line diff of the selected capture against
	// the one before it. Same-sha256 pairs are extractor re-reads
	// (P360), never page changes. "Draft from added lines" asks Muse
	// Spark about the added lines only; drafts land in /review.
	const api = (path: string) => `/apostille/api/v1${path}`;
	const sourceId = $derived(page.params.id ?? '');

	interface Source {
		id: number;
		authority: string;
		kind: string;
		uri: string;
	}
	interface Capture {
		id: number;
		capturedAt: number;
		kind: string;
		trust: string;
		sha: string;
		bytes: number;
		previousId: number | null;
		reread: boolean;
		added: number | null;
		removed: number | null;
	}
	interface Line {
		op: 'same' | 'added' | 'removed' | 'gap';
		text: string;
		lines: number;
	}
	interface Diff {
		previousId: number | null;
		previousAt: number | null;
		reread: boolean;
		missingSidecar: boolean;
		added: number;
		removed: number;
		lines: Line[];
	}

	let source = $state<Source | null>(null);
	let captures = $state<Capture[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let selected = $state<number | null>(null);
	let diff = $state<Diff | null>(null);
	let diffState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let diffError = $state<string | null>(null);
	let drafting = $state(false);
	let draftNote = $state<string | null>(null);

	function stamp(unix: number | null): string {
		if (!unix) return '—';
		return new Date(unix * 1000).toISOString().replace('T', ' ').slice(0, 16) + ' UTC';
	}

	async function failure(res: Response, what: string): Promise<string> {
		return authError(res.status, await readErrorBody(res)) ?? `${what} failed (HTTP ${res.status}).`;
	}

	function normalizeCapture(row: any): Capture {
		return {
			id: Number(row?.id ?? 0),
			capturedAt: Number(row?.captured_at ?? 0),
			kind: String(row?.kind ?? ''),
			trust: String(row?.trust ?? ''),
			sha: String(row?.sha256 ?? ''),
			bytes: Number(row?.bytes ?? 0),
			previousId: typeof row?.previous_id === 'number' ? row.previous_id : null,
			reread: row?.reread === true,
			added: typeof row?.added === 'number' ? row.added : null,
			removed: typeof row?.removed === 'number' ? row.removed : null
		};
	}

	async function load(id: string) {
		loadState = 'loading';
		loadError = null;
		try {
			const res = await fetch(api(`/admin/sources/${encodeURIComponent(id)}/captures`), {
				credentials: 'same-origin'
			});
			if (!res.ok) {
				loadError = res.status === 404 ? `No source #${id}.` : await failure(res, 'Loading the history');
				loadState = 'failed';
				return;
			}
			const body = await res.json();
			source = {
				id: Number(body?.source?.id ?? 0),
				authority: String(body?.source?.authority ?? ''),
				kind: String(body?.source?.kind ?? ''),
				uri: String(body?.source?.uri ?? '')
			};
			captures = (Array.isArray(body?.captures) ? body.captures : []).map(normalizeCapture);
			loadState = 'ready';
			const wanted = Number(page.url.searchParams.get('capture'));
			const first = captures.find((c) => c.id === wanted) ?? captures.find((c) => c.previousId !== null);
			if (first) void select(first.id);
		} catch {
			loadError = 'The admin host could not be reached.';
			loadState = 'failed';
		}
	}

	async function select(id: number) {
		selected = id;
		draftNote = null;
		diffState = 'loading';
		diffError = null;
		try {
			const res = await fetch(api(`/admin/captures/${id}/diff`), { credentials: 'same-origin' });
			if (!res.ok) {
				diffError = await failure(res, 'Loading the diff');
				diffState = 'failed';
				return;
			}
			const body = await res.json();
			diff = {
				previousId: typeof body?.previous?.id === 'number' ? body.previous.id : null,
				previousAt: typeof body?.previous?.captured_at === 'number' ? body.previous.captured_at : null,
				reread: body?.reread === true,
				missingSidecar: body?.missing_sidecar === true,
				added: Number(body?.added ?? 0),
				removed: Number(body?.removed ?? 0),
				lines: (Array.isArray(body?.lines) ? body.lines : []).map((l: any) => ({
					op: ['same', 'added', 'removed', 'gap'].includes(l?.op) ? l.op : 'same',
					text: String(l?.text ?? ''),
					lines: Number(l?.lines ?? 0)
				}))
			};
			diffState = 'ready';
		} catch {
			diffError = 'The admin host could not be reached.';
			diffState = 'failed';
		}
	}

	async function draftChanges() {
		if (selected === null) return;
		drafting = true;
		draftNote = null;
		try {
			const res = await fetch(api(`/admin/captures/${selected}/draft-changes`), {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				credentials: 'same-origin',
				body: '{}'
			});
			if (!res.ok) {
				draftNote = await failure(res, 'Drafting');
				return;
			}
			const body = await res.json();
			draftNote =
				body?.outcome === 'drafted'
					? `Muse Spark read ${body.added_lines} added line(s): ${body.proposed} proposed, ${body.rejected} refused by the quote check, ${body.suppressed} suppressed.`
					: `Nothing drafted: ${body?.reason ?? 'no change to read'}.`;
		} catch {
			draftNote = 'The admin host could not be reached.';
		} finally {
			drafting = false;
		}
	}

	$effect(() => {
		const id = sourceId;
		if (id) void load(id);
	});

	const current = $derived(captures.find((c) => c.id === selected) ?? null);
</script>

<svelte:head>
	<title>Source history — Apostille staff</title>
</svelte:head>

<div class="mx-auto flex w-full max-w-5xl flex-col gap-6 px-4 py-8 sm:px-6">
	<div>
		<p class="text-eyebrow m-0 font-semibold tracking-wide text-ink-muted uppercase">
			<a class="text-ink-muted" href={resolve('/(admin)/workers')}>Workers</a> · source #{sourceId}
		</p>
		<Heading level={1}>Source history</Heading>
		{#if source}
			<p class="m-0 mt-2 font-mono text-sm wrap-anywhere">{source.uri}</p>
			<p class="m-0 mt-1 text-sm text-ink-muted">{source.authority} · {source.kind.replaceAll('_', ' ')}</p>
		{/if}
	</div>

	{#if loadState === 'loading'}
		<div role="status" aria-label="Loading history" class="flex flex-col gap-2">
			<Skeleton class="h-24 w-full rounded-lg" />
			<Skeleton class="h-24 w-full rounded-lg" />
		</div>
	{:else if loadState === 'failed'}
		<Notice variant="warning" heading="History could not load."><p class="m-0">{loadError}</p></Notice>
	{:else if captures.length === 0}
		<Notice variant="info" heading="No captures yet."><p class="m-0">The watch has not stored this source.</p></Notice>
	{:else}
		<div class="grid gap-6 lg:grid-cols-12">
			<section aria-label="Captures" class="min-w-0 lg:col-span-4">
				<ul class="m-0 flex list-none flex-col gap-2 p-0">
					{#each captures as c (c.id)}
						<li class="m-0 p-0">
							<button
								type="button"
								onclick={() => select(c.id)}
								aria-current={selected === c.id ? 'true' : undefined}
								class={`flex w-full cursor-pointer flex-col gap-1 rounded-lg border p-3 text-left ${
									selected === c.id ? 'border-focus bg-surface' : 'border-line bg-card'
								}`}
							>
								<span class="flex flex-wrap items-center gap-2 text-sm">
									<span class="font-mono">#{c.id}</span>
									<span class="text-ink-muted">{stamp(c.capturedAt)}</span>
								</span>
								<span class="text-xs text-ink-muted">
									{#if c.previousId === null}
										first capture
									{:else if c.reread}
										extractor re-read of #{c.previousId} (same bytes)
									{:else if c.added !== null}
										+{c.added} / −{c.removed ?? 0} lines vs #{c.previousId}
									{:else}
										vs #{c.previousId}
									{/if}
									· sha <span class="font-mono">{c.sha.slice(0, 10)}</span>
								</span>
							</button>
						</li>
					{/each}
				</ul>
			</section>

			<section aria-label="Diff" class="min-w-0 lg:col-span-8">
				{#if diffState === 'loading'}
					<Skeleton class="h-48 w-full rounded-lg" />
				{:else if diffState === 'failed'}
					<Notice variant="warning" heading="Diff could not load."><p class="m-0">{diffError}</p></Notice>
				{:else if diff && current}
					{#if diff.previousId === null}
						<Notice variant="info" heading="First capture of this source.">
							<p class="m-0">Nothing to compare against yet.</p>
						</Notice>
					{:else}
						<div class="flex flex-wrap items-center gap-3">
							<p class="m-0 text-sm">
								#{diff.previousId} ({stamp(diff.previousAt)}) → #{current.id} ({stamp(current.capturedAt)}):
								<span class="font-mono">+{diff.added} −{diff.removed}</span>
							</p>
							<Button
								class="ml-auto"
								onclick={draftChanges}
								disabled={drafting || diff.reread || diff.added === 0}
							>
								{drafting ? 'Drafting…' : 'Draft from added lines'}
							</Button>
						</div>
						{#if diff.reread}
							<div class="mt-3">
								<Notice variant="info" heading="Extractor re-read, not a page change.">
									<p class="m-0">
										Same bytes as #{diff.previousId}; a newer text extractor read them differently
										(P360). Existing facts keep citing the older capture.
									</p>
								</Notice>
							</div>
						{/if}
						{#if diff.missingSidecar}
							<div class="mt-3">
								<Notice variant="warning" heading="A text sidecar is missing.">
									<p class="m-0">The diff below compares against empty text.</p>
								</Notice>
							</div>
						{/if}
						{#if draftNote}
							<p class="m-0 mt-3 rounded-md border border-line bg-surface px-3 py-2 text-sm" role="status">
								{draftNote}
								<a href={resolve('/(admin)/review')}>Open Review</a>
							</p>
						{/if}
						<ol class="m-0 mt-4 list-none overflow-x-auto rounded-lg border border-line bg-card p-0 font-mono text-xs">
							{#each diff.lines as line, i (i)}
								{#if line.op === 'gap'}
									<li class="m-0 border-b border-line px-3 py-1 text-ink-muted">… {line.lines} unchanged line{line.lines === 1 ? '' : 's'}</li>
								{:else}
									<li
										class={`m-0 border-l-2 px-3 py-1 break-words ${
											line.op === 'added'
												? 'border-verified bg-verified-bg'
												: line.op === 'removed'
													? 'border-changed bg-changed-bg'
													: 'border-line'
										}`}
									>
										<span aria-hidden="true">{line.op === 'added' ? '+' : line.op === 'removed' ? '−' : ' '}</span>
										<span class="sr-only">{line.op === 'same' ? '' : `${line.op}: `}</span>{line.text}
									</li>
								{/if}
							{/each}
						</ol>
					{/if}
				{/if}
			</section>
		</div>
	{/if}
</div>
