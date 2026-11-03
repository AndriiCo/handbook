<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { toast } from 'svelte-sonner';
	import Heading from '#lib/components/brand/Heading.svelte';
	import Notice from '#lib/public-ui/Notice.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Kbd } from '#lib/components/ui/kbd/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import AuthorityGroups from '#lib/admin/AuthorityGroups.svelte';
	import { fetchProposedFacts, type QueueItem } from '#lib/admin/review-client.js';
	import { fetchFactChecks, type FactCheck } from '#lib/admin/proposals.js';

	// Triage = every proposed fact grouped per authority, decided in
	// bulk. It reads the approval API itself (`GET /facts`, whose
	// citations are exactly the accept gate) rather than the enriched
	// queue, and shares the group view with /review?view=authority.
	// A refused read shows the server's answer verbatim (the 403 body
	// names the detected IP).
	const api = '/apostille/api/v1';

	let items = $state<QueueItem[]>([]);
	let checks = $state<Record<number, FactCheck>>({});
	let decided = $state<Record<number, string>>({});
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let reduceMotion = $state(false);

	async function loadChecks() {
		const slugs = [...new Set(items.map((i) => i.authority_slug))];
		const found = await Promise.allSettled(slugs.map((slug) => fetchFactChecks(slug)));
		const next: Record<number, FactCheck> = {};
		for (const r of found) {
			if (r.status !== 'fulfilled') continue;
			for (const c of r.value) next[c.fact_id] = c;
		}
		checks = next;
	}

	async function load(quiet = false) {
		if (!quiet) {
			loadState = 'loading';
			loadError = null;
		}
		try {
			const out = await fetchProposedFacts(api, { limit: 200 });
			items = out.items;
			loadState = 'ready';
			void loadChecks();
		} catch (err) {
			const message =
				err instanceof TypeError
					? 'The admin host is unreachable — is the API server running?'
					: err instanceof Error
						? err.message
						: 'Proposed facts are unreachable.';
			// A refresh after a stale batch keeps the groups on screen.
			if (quiet) {
				toast.error(message);
				return;
			}
			loadError = message;
			loadState = 'failed';
		}
	}

	async function loadAll(slug: string) {
		const out = await fetchProposedFacts(api, { authority: slug, limit: 200 });
		const known = new Set(items.map((i) => i.id));
		items = [...items, ...out.items.filter((i) => !known.has(i.id))];
		void loadChecks();
	}

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		void load();
	});
</script>

<svelte:head>
	<title>Triage — Admin — Apostille Handbook</title>
</svelte:head>

<div class="mt-6 flex flex-col gap-6 pb-12">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<Heading level={1} eyebrow="Content">Proposed facts by authority</Heading>
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Accept every verified proposal of an authority as one audited decision, or reject a batch
				with one reason. Uncheck a row to hold it back. The detail pane with quotes, diffs and edits
				lives in <a class="font-medium text-accent-strong underline underline-offset-4" href={resolve('/(admin)/review')}>Review</a>.
			</p>
		</div>
		<p class="m-0 hidden items-center gap-1 text-xs text-ink-muted md:flex">
			<Kbd>j</Kbd><Kbd>k</Kbd> move <span aria-hidden="true">·</span> <Kbd>x</Kbd> check <span aria-hidden="true">·</span>
			<Kbd>a</Kbd> accept <span aria-hidden="true">·</span> <Kbd>Shift</Kbd><Kbd>A</Kbd> accept group
			<span aria-hidden="true">·</span> <Kbd>r</Kbd> reject
		</p>
	</header>

	{#if loadState === 'loading'}
		<div class="flex flex-col gap-4" role="status" aria-label="Loading proposed facts">
			{#each [0, 1] as i (i)}
				<Skeleton class="h-48 rounded-lg" />
			{/each}
		</div>
	{:else if loadState === 'failed'}
		<Notice variant="warning" heading="Queue unavailable.">
			<p class="m-0">{loadError}</p>
			<p class="mt-3 mb-0"><Button variant="outline" onclick={() => load()}>Try again</Button></p>
		</Notice>
	{:else}
		<AuthorityGroups
			{items}
			{checks}
			bind:decided
			{reduceMotion}
			onLoadAll={loadAll}
			onRefresh={() => void load(true)}
		/>
	{/if}
</div>
