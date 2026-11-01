<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../../lib/components/ui/skeleton/index.js';
	import * as HoverCard from '../../../../lib/components/ui/hover-card/index.js';
	import { subscribe } from '../../../../lib/live.svelte.js';
	import {
		factCount,
		groupFactsByTopic,
		publicError,
		readDossier,
		topicLabel
	} from '../../../../lib/public-ui/handbook.js';
	import Heading from '../../../../lib/components/brand/Heading.svelte';

	// Slice A: live dossier over GET /api/v1/authorities/{slug}/facts
	// (accepted-only + conflict banners). Same-origin, no auth.
	// No prices render (pricing table open): the rail shows counts
	// only, never a fee total.
	const slug = $derived(page.params.slug ?? '');

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	let facts = $state<any[] | null>(null);
	let loadError = $state<string | null>(null);
	let notFound = $state(false);
	let tab = $state('');
	let authorityName = $state<string | null>(null);

	const groups = $derived(groupFactsByTopic(facts ?? []));


	// Full class strings per state (not stacked class: directives):
	// the active border/text colors must never co-occur with the
	// idle ones in the cascade.
	const tabCls = (isActive: boolean) =>
		`inline-flex min-h-11 cursor-pointer items-center border-0 border-b-[3px] bg-transparent px-[0.9rem] py-2 ${
			isActive ? 'border-accent-fill font-bold text-ink' : 'border-transparent text-ink-muted'
		}`;
	const topicId = (topic: string) =>
		`topic-${topic.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;

	async function reload(current: string) {
		const res = await readDossier(current, { get });
		if (current !== slug) return; // stale slug, ignore
		if (!res.ok) {
			if (res.status === 404) notFound = true;
			else loadError = publicError(res.status, 'dossier');
			return;
		}
		const rows = res.body?.facts;
		const list = Array.isArray(rows) ? rows : [];
		facts = list;
		tab = groupFactsByTopic(list)[0]?.topic ?? tab;
	}

	async function resolveName(current: string) {
		try {
			const res = await get('/authorities');
			if (current !== slug || !res.ok) return;
			const rows = res.body?.authorities;
			const found = Array.isArray(rows) ? rows.find((a) => a?.slug === current) : null;
			if (found && typeof found.name === 'string' && found.name) authorityName = found.name;
		} catch {
			// The slug stays the heading; the index is a nicety, not a gate.
		}
	}

	$effect(() => {
		const current = slug;
		facts = null;
		loadError = null;
		notFound = false;
		tab = '';
		authorityName = null;
		reload(current).catch(() => {
			if (current !== slug) return;
			loadError = publicError(500, 'dossier');
		});
		void resolveName(current);
		// Live ledger: an approval in /review re-renders this dossier
		// over WS `authority:{slug}` — no polling.
		return subscribe(`authority:${current}`, (msg) => {
			if (current !== slug) return;
			if (msg?.type === 'fact_accepted') void reload(current).catch(() => {});
		});
	});
</script>

<svelte:head>
	<title>{authorityName ?? slug} — Apostille Handbook</title>
</svelte:head>

{#if notFound}
	<Notice variant="warning" heading="Unknown authority “{slug}”.">
		<p>
			This slug is not in the handbook index.
			<a href={resolve('/(public)')}>Back to the authority index</a>.
		</p>
	</Notice>
{:else if loadError}
	<Notice variant="error" heading="This dossier is unavailable."><p>{loadError}</p></Notice>
{:else if facts === null}
	<div role="status" aria-label="Loading dossier" class="flex flex-col gap-2 pt-6">
		<span class="sr-only">Loading dossier</span>
		<Skeleton class="h-9 w-2/3" />
		<Skeleton class="h-4 w-1/3" />
		<Skeleton class="h-24 w-full" />
	</div>
{:else}
	<header class="mb-4 pt-6 pb-4">
		<p class="kicker">Authority dossier</p>
		<Heading level={1}>{authorityName ?? slug}</Heading>
	</header>

	{#if facts.length === 0}
		<Notice variant="info" heading="No published facts yet.">
			<p>
				This authority is in the index, but no accepted facts have been
				published. General information, not legal advice.
			</p>
		</Notice>
	{:else}
		<div class="mt-4 grid gap-8 pb-8 md:grid-cols-[3fr_2fr]">
			<div>
				<nav class="flex flex-wrap gap-1 border-b border-line" aria-label="Dossier sections">
					{#each groups as g (g.topic)}
						<a
							href={`#${topicId(g.topic)}`}
							class={tabCls(tab === g.topic)}
							aria-current={tab === g.topic ? 'true' : undefined}
							onclick={() => (tab = g.topic)}
						>
							{topicLabel(g.topic)}
						</a>
					{/each}
				</nav>
				{#each groups as g (g.topic)}
					<section aria-labelledby={topicId(g.topic)}>
						<h2 id={topicId(g.topic)} class="mt-6 mb-0 text-lg">{topicLabel(g.topic)}</h2>
						<ul class="m-0 mt-4 list-none p-0">
							{#each g.facts as fact (fact.id)}
								<li class="border-b border-line py-3">
									<p class="m-0 mb-2">{fact.statement}</p>
									{#if Array.isArray(fact.citations) && fact.citations.length > 0}
										{@const cite = fact.citations[0]}
										<HoverCard.Root>
											<HoverCard.Trigger
												class="cursor-help text-sm text-ink-muted underline decoration-dotted underline-offset-4"
											>
												Verified source
											</HoverCard.Trigger>
											<HoverCard.Content class="max-w-80" align="start">
												<blockquote class="m-0 border-l-2 border-verified pl-3 font-quote italic">
													{cite.quote}
												</blockquote>
												<p class="mt-2 mb-0 text-xs text-ink-muted">
													{#if typeof cite.source_url === 'string' && cite.source_url.startsWith('https://')}
														<a
															href={cite.source_url}
															target="_blank"
															rel="noopener noreferrer">Source</a
														>
													{:else}
														<span>Source unavailable</span>
													{/if}
													· captured {String(cite.captured_at ?? '').split('T')[0]}
													· drafted by {fact.drafted_by ?? fact.created_by ?? 'unknown'}
												</p>
											</HoverCard.Content>
										</HoverCard.Root>
									{/if}
									{#if fact.conflict}
										<Notice variant="warning" heading="Conflicting reports for this item.">
											{#each fact.conflicts ?? [] as sib (sib.id)}
												<p>{sib.statement} <span class="font-mono">[{sib.status}]</span></p>
											{/each}
										</Notice>
									{/if}
								</li>
							{/each}
						</ul>
					</section>
				{/each}
			</div>

			<aside class="flex flex-col gap-4" aria-label="About this dossier">
				<section class="rounded-md border border-line bg-surface p-4" aria-labelledby="about">
					<h2 id="about" class="m-0 mb-2 text-base">About this dossier</h2>
					<p>{factCount(facts.length)} across {groups.length} topic{groups.length === 1 ? '' : 's'}.</p>
					<p class="mb-0 text-[0.8rem] text-ink-muted">General information, not legal advice. No fee totals — the pricing table is still open.</p>
				</section>
				<p><a href={resolve('/(customer)/login')}>Report a change (login required)</a></p>
			</aside>
		</div>
	{/if}
{/if}
