<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';

	// Staff "Load packs" (P397): load the live release's jurisdiction
	// packs (curated, world, world-nonparty) into the store as the
	// service user — authorities, domains, sources and review tasks
	// only, never facts. Re-runs are no-ops; a corrected pack updates
	// (or hides) the rows it names, audited. Research results staged
	// from the Work page load the same way. Admin build only; the
	// staff API keeps the /apostille prefix on the admin origin.
	const api = () => `/apostille/api/v1/admin/packs`;

	interface PackSet {
		set: string;
		packs: number;
	}
	interface Staged {
		id: number;
		packs: number;
		files: string[];
	}
	interface Listing {
		sets: PackSet[];
		staged?: Staged[];
		authorities: number | null;
		running: boolean;
	}
	interface LoadResult {
		ok: boolean;
		set: string;
		before: number | null;
		after: number | null;
		stdout: string;
		stderr: string;
	}

	const LABEL: Record<string, string> = {
		curated: 'Curated packs (top level)',
		world: 'World — Apostille Convention parties',
		'world-nonparty': 'World — non-party countries (consular legalization)'
	};

	let listing = $state<Listing | null>(null);
	let listError = $state<string | null>(null);
	let busy = $state<string | null>(null);
	let result = $state<LoadResult | null>(null);
	let loadError = $state<string | null>(null);

	async function refresh() {
		listError = null;
		try {
			const res = await fetch(api());
			if (!res.ok) throw new Error(`packs request failed (${res.status})`);
			listing = (await res.json()) as Listing;
		} catch (err) {
			listError = err instanceof Error ? err.message : 'Packs request failed.';
		}
	}

	onMount(refresh);

	const stagedLabel = (set: string) =>
		set.startsWith('staged:') ? `Staged research result #${set.slice(7)}` : (LABEL[set] ?? set);

	async function load(set: string, staged?: number) {
		busy = staged === undefined ? set : `staged:${staged}`;
		loadError = null;
		result = null;
		try {
			const res = await fetch(`${api()}/load`, {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				body: JSON.stringify(staged === undefined ? { set } : { staged })
			});
			const body = await res.json().catch(() => ({}));
			if (res.status === 200 || res.status === 422) {
				if ('ok' in body) result = body as LoadResult;
				else throw new Error(body?.error ?? `load failed (${res.status})`);
			} else {
				throw new Error(body?.error ?? `load failed (${res.status})`);
			}
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Load failed.';
		} finally {
			busy = null;
			await refresh();
		}
	}
</script>

<svelte:head>
	<title>Packs — staff</title>
</svelte:head>

<main class="mx-auto max-w-2xl space-y-6 p-6">
	<h1 class="text-xl font-bold">Jurisdiction packs</h1>
	<p class="text-sm">
		Load the packs shipped in this release into the live store. Loading adds authorities, their
		watched sources and review tasks, and brings existing authorities in line with the pack
		(renamed or hidden rows, each audited); it never adds facts, and loading twice changes
		nothing. Load the curated set first so world rows merge into the curated dossiers.
	</p>

	{#if listError}
		<Notice variant="error" heading="Packs unavailable"><p>{listError}</p></Notice>
	{:else if listing === null}
		<div role="status" aria-label="Working" class="space-y-2">
			<div class="h-4 animate-pulse rounded bg-surface-2"></div>
			<div class="h-4 w-2/3 animate-pulse rounded bg-surface-2"></div>
		</div>
	{:else}
		<p class="text-sm">
			Authorities in the store now: <strong>{listing.authorities ?? '—'}</strong>
		</p>
		<ul class="m-0 list-none space-y-3 p-0">
			{#each listing.sets as s (s.set)}
				<li class="flex items-center gap-3 rounded border border-line p-4">
					<div class="min-w-0 flex-1">
						<p class="m-0 font-medium">{LABEL[s.set] ?? s.set}</p>
						<p class="m-0 text-sm text-ink-muted">{s.packs} pack file(s) in this release</p>
					</div>
					<button
						type="button"
						class="min-h-11 rounded border border-accent-fill bg-accent-fill px-4 py-2 text-accent-ink disabled:opacity-50"
						disabled={s.packs === 0 || busy !== null || listing.running}
						onclick={() => load(s.set)}
					>
						{busy === s.set ? 'Loading…' : 'Load'}
					</button>
				</li>
			{/each}
		</ul>
		<h2 class="text-lg font-bold">Staged by research</h2>
		{#if (listing.staged ?? []).length === 0}
			<p class="text-sm text-ink-muted">
				No research result is staged. Stage one from a finished job on the Work page.
			</p>
		{:else}
			<ul class="m-0 list-none space-y-3 p-0">
				{#each listing.staged ?? [] as s (s.id)}
					<li class="flex items-center gap-3 rounded border border-line p-4">
						<div class="min-w-0 flex-1">
							<p class="m-0 font-medium">Work item #{s.id}</p>
							<p class="m-0 text-sm break-words text-ink-muted">{s.files.join(', ')}</p>
						</div>
						<button
							type="button"
							class="min-h-11 rounded border border-accent-fill bg-accent-fill px-4 py-2 text-accent-ink disabled:opacity-50"
							disabled={busy !== null || listing.running}
							onclick={() => load('', s.id)}
						>
							{busy === `staged:${s.id}` ? 'Loading…' : 'Load'}
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	{/if}

	{#if loadError}
		<Notice variant="error" heading="Load failed"><p>{loadError}</p></Notice>
	{/if}
	{#if result}
		<section aria-label="Load result" class="space-y-2 rounded border border-line p-4">
			<h2 class={`font-bold ${result.ok ? 'text-success' : 'text-error'}`}>
				{result.ok ? 'Loaded' : 'Refused'}: {stagedLabel(result.set)}
			</h2>
			<p class="text-sm text-ink">
				Authorities: {result.before ?? '—'} → {result.after ?? '—'}
			</p>
			<pre
				class="max-h-80 overflow-auto rounded border border-line bg-surface p-2 text-xs whitespace-pre-wrap text-ink">{result.stdout}{result.stderr}</pre>
		</section>
	{/if}
</main>
