<script lang="ts">
	import { untrack } from 'svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import { domain } from '#lib/public-ui/guide.js';
	import type { BusMessage } from '#lib/live.svelte.js';
	import AuthorityPicker from './AuthorityPicker.svelte';
	import Considerations from './Considerations.svelte';
	import RetireFact from './RetireFact.svelte';
	import { fetchAcceptedFacts, inPublicConflict, slotFor, slotLabel, type PublishedFact } from './evidence-client.js';
	import { ago } from './format.js';
	import BadgeCheckIcon from '@lucide/svelte/icons/badge-check';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

	// Published tab of /review: the facts an office's public guide shows
	// now, one drawer per fact with its considerations (staff notes) and
	// "Retire this fact…". A retired row stays in view, marked Retired
	// with its reason, until the page reloads. Lists re-read when the
	// office's channel reports a retire (the drawer's panel listens).
	let { active = false, initialSlug = '' }: { active?: boolean; initialSlug?: string } = $props();

	const API = '/apostille/api/v1';
	let slug = $state(untrack(() => initialSlug));
	let rows = $state<PublishedFact[]>([]);
	let loadState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let loadError = $state<string | null>(null);
	let selectedId = $state<number | null>(null);
	let retired = $state<Record<number, { fact: PublishedFact; reason: string }>>({});
	// The fact a retire request is out for: it stays in view (and its
	// drawer open) even if the `fact_retired` re-read lands first.
	let pending = $state<PublishedFact | null>(null);
	let liveNote = $state<string | null>(null);

	const list = $derived.by(() => {
		const out = rows.filter((f) => !retired[f.id]);
		for (const r of Object.values(retired)) if (r.fact.authority_slug === slug) out.push(r.fact);
		if (pending && pending.authority_slug === slug && !out.some((f) => f.id === pending?.id)) out.push(pending);
		return out.sort((a, b) => a.id - b.id);
	});
	const selected = $derived(list.find((f) => f.id === selectedId) ?? null);
	const nowSec = () => Math.floor(Date.now() / 1000);

	async function load(s: string, quiet = false) {
		if (!quiet) {
			loadState = 'loading';
			loadError = null;
		}
		try {
			const facts = await fetchAcceptedFacts(API, s);
			if (s !== slug) return;
			// Staff-only call scripts are accepted but never public.
			rows = facts.filter((f) => f.topic !== 'phone_script');
			loadState = 'ready';
			loadError = null;
			if (selectedId === null || !list.some((f) => f.id === selectedId)) selectedId = list[0]?.id ?? null;
		} catch (err) {
			if (s !== slug) return;
			loadError = err instanceof Error ? err.message : 'Published facts failed to load.';
			if (!quiet || loadState !== 'ready') loadState = 'failed';
		}
	}

	// Read the office's published facts while the tab is open: a new
	// office starts empty; coming back to the tab re-reads quietly.
	let loadedSlug = '';
	$effect(() => {
		if (!active || !slug) return;
		const s = slug;
		untrack(() => {
			const same = s === loadedSlug;
			if (!same) {
				rows = [];
				selectedId = null;
				liveNote = null;
				loadedSlug = s;
			}
			void load(s, same && loadState === 'ready');
		});
	});

	function markRetired(fact: PublishedFact, reason: string) {
		retired[fact.id] = { fact, reason };
		liveNote = null;
	}

	// The drawer's panel heard `fact_retired`: re-read the list.
	function onLive(msg: BusMessage) {
		if (msg.type !== 'fact_retired') return;
		const id = Number(msg.fact_id);
		if (Number.isInteger(id) && !retired[id] && pending?.id !== id && rows.some((f) => f.id === id)) {
			liveNote = `Fact #${id} was retired elsewhere; the list is up to date.`;
		}
		void load(slug, true);
	}

	function capturedOn(iso: string): string {
		const d = new Date(iso);
		return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
	}
</script>

<div class="flex flex-col gap-4">
	<p class="m-0 max-w-prose text-sm text-ink-muted">
		Facts the public guide shows now. Record what the office told you as a consideration, or retire a
		fact that was accepted by mistake. Both reach the public guide.
	</p>
	<div class="max-w-md">
		<AuthorityPicker id="published-authority" bind:value={slug} />
	</div>

	{#if liveNote}
		<p class="m-0 text-sm font-semibold" role="status">{liveNote}</p>
	{/if}

	{#if !slug}
		<p class="m-0 text-sm text-ink-muted">Pick an authority to see its published facts.</p>
	{:else}
		<div class="grid items-start gap-4 lg:grid-cols-5">
			<div class="flex min-w-0 flex-col gap-3 lg:col-span-3">
				{#if loadState === 'loading' || loadState === 'idle'}
					<div class="flex flex-col gap-2" role="status" aria-label="Loading published facts">
						{#each [0, 1, 2] as i (i)}
							<Skeleton class="h-16 rounded-sm" />
						{/each}
					</div>
				{:else if loadState === 'failed'}
					<Notice variant="error" heading="Published facts could not load.">
						<p class="m-0">{loadError}</p>
						<p class="mt-3 mb-0"><Button variant="outline" onclick={() => load(slug)}>Try again</Button></p>
					</Notice>
				{:else if list.length === 0}
					<Card.Root class="items-start px-6 py-10">
						<BadgeCheckIcon class="size-6 text-ink-muted" aria-hidden="true" />
						<p class="m-0 text-h3 font-semibold">Nothing published yet</p>
						<p class="m-0 text-sm text-ink-muted">Accepted facts of {slug} show here.</p>
					</Card.Root>
				{:else}
					<p class="m-0 text-xs text-ink-muted">{list.length} published · {slug}</p>
					<div class="overflow-hidden rounded-md border border-line">
						<table class="block w-full border-collapse text-sm md:table">
							<caption class="sr-only">Published facts of {slug}</caption>
							<thead class="sr-only bg-surface md:not-sr-only">
								<tr class="text-left text-xs text-ink-muted">
									<th scope="col" class="px-3 py-2 font-medium">Published fact</th>
									<th scope="col" class="px-3 py-2 font-medium">Signals</th>
									<th scope="col" class="px-3 py-2 text-right font-medium">Accepted</th>
								</tr>
							</thead>
							<tbody class="block md:table-row-group">
								{#each list as fact (fact.id)}
									{@const gone = retired[fact.id]}
									<tr
										class={`flex flex-col border-t border-l-2 border-t-line transition-colors duration-150 md:table-row ${fact.id === selectedId ? 'border-l-accent-strong bg-surface' : 'border-l-transparent hover:bg-surface'}`}
										aria-current={fact.id === selectedId ? 'true' : undefined}
									>
										<td class="min-w-0 px-3 pt-2 md:py-2">
											<button
												type="button"
												class="flex w-full min-w-0 cursor-pointer flex-col gap-0.5 text-left"
												onclick={() => (selectedId = fact.id)}
											>
												<span class="block max-w-full truncate font-mono text-xs text-ink-muted">#{fact.id} · {fact.topic} · {slotFor(fact.topic, fact.statement)}</span>
												<span class={`line-clamp-2 ${gone ? 'text-ink-muted line-through' : 'text-ink'}`}>{fact.statement}</span>
											</button>
											{#if gone}
												<p class="m-0 pt-1 text-xs break-words" role="status">
													<span class="font-semibold text-error">Retired</span> · {gone.reason}
												</p>
											{/if}
										</td>
										<td class="px-3 py-1 md:py-2">
											<span class="flex flex-wrap gap-1">
												{#if gone}
													<Badge variant="outline" class="border-error text-error">Retired</Badge>
												{:else if inPublicConflict(fact)}
													<Badge variant="outline" class="border-error text-error">Disputed</Badge>
												{/if}
											</span>
										</td>
										<td class="px-3 pb-2 text-xs whitespace-nowrap text-ink-muted md:py-2 md:text-right">
											{fact.accepted_at ? ago(fact.accepted_at, nowSec()) : '—'}
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</div>

			<aside class="min-w-0 lg:sticky lg:top-20 lg:col-span-2" aria-label="Selected published fact">
				{#if selected}
					{@const fact = selected}
					{@const gone = retired[fact.id]}
					{@const slot = slotFor(fact.topic, fact.statement)}
					{@const disagreeing = fact.conflicts.filter((c) => c.status === 'accepted')}
					<Card.Root class="gap-4 shadow-md">
						<Card.Header>
							<Card.Title class="flex flex-wrap items-center gap-2">
								<h2 class="m-0 text-h3 font-semibold">Fact #{fact.id}</h2>
								<Badge variant="secondary">{fact.topic}</Badge>
								{#if gone}
									<Badge variant="outline" class="border-error text-error">Retired</Badge>
								{:else}
									<Badge variant="outline" class="border-verified text-verified">Published</Badge>
								{/if}
							</Card.Title>
							<Card.Description>
								<span class="font-mono">{fact.authority_slug}</span>{fact.accepted_at ? ` · accepted ${ago(fact.accepted_at, nowSec())}` : ''}
							</Card.Description>
						</Card.Header>
						<Card.Content class="flex flex-col gap-4">
							<p class={`m-0 text-body font-semibold break-words ${gone ? 'text-ink-muted line-through' : ''}`}>{fact.statement}</p>
							<p class="m-0 text-xs text-ink-muted">Field: {slotLabel(slot)} · <span class="font-mono">{slot}</span></p>

							<section aria-label="Citations" class="flex flex-col gap-2">
								{#if fact.citations.length === 0}
									<p class="m-0 text-sm text-ink-muted">No quote-verified citation on file.</p>
								{:else}
									{#each fact.citations as cite, ci (ci)}
										<figure class="m-0 flex flex-col gap-1 rounded-sm border-l-2 border-verified bg-surface px-3 py-2">
											<blockquote class="m-0 font-quote text-body break-words italic">{cite.quote}</blockquote>
											<figcaption class="flex flex-wrap items-center gap-x-2 text-xs text-ink-muted">
												<span>{cite.kind === 'pdf' ? 'Official PDF' : cite.kind === 'upload' ? 'Staff upload' : 'Official web page'}</span>
												{#if cite.source_url}
													<a class="underline underline-offset-2" href={cite.source_url} target="_blank" rel="noopener noreferrer">{domain(cite.source_url)}</a>
												{/if}
												{#if capturedOn(cite.captured_at)}<span>captured {capturedOn(cite.captured_at)}</span>{/if}
											</figcaption>
										</figure>
									{/each}
								{/if}
							</section>

							{#if disagreeing.length > 0}
								<div class="flex flex-col gap-1 rounded-sm bg-error-bg px-3 py-2 text-sm">
									<p class="m-0 flex items-center gap-2 font-semibold">
										<TriangleAlertIcon class="size-4 flex-none" aria-hidden="true" />Disagrees with other published facts
									</p>
									{#each disagreeing as sib (sib.id)}
										<p class="m-0 break-words"><span class="font-mono text-xs">#{sib.id}</span> {sib.statement}</p>
									{/each}
								</div>
							{/if}

							{#if gone}
								<p id={`published-retired-${fact.id}`} tabindex="-1" class="m-0 rounded-sm border border-line px-3 py-2 text-sm break-words" role="status">
									<span class="font-semibold">Retired.</span> It no longer shows on the public guide. Reason: {gone.reason}
								</p>
							{:else}
								<Considerations
									subject={{
										id: fact.id,
										authority_slug: fact.authority_slug,
										topic: fact.topic,
										statement: fact.statement,
										status: 'accepted',
										related: fact.conflicts.map((c) => c.id)
									}}
									{onLive}
								/>
								<div class="flex flex-col gap-2 border-t border-line pt-4">
									<p class="m-0 text-xs text-ink-muted">
										Accepted by mistake, or no longer what the office says? Retiring takes it off the public guide.
									</p>
									<div>
										<RetireFact
											{fact}
											onRetired={(reason) => markRetired(fact, reason)}
											onPending={(on) => (pending = on ? fact : null)}
											focusAfter={`published-retired-${fact.id}`}
										/>
									</div>
								</div>
							{/if}
						</Card.Content>
					</Card.Root>
				{:else if loadState === 'ready'}
					<Card.Root class="items-start px-6 py-10">
						<p class="m-0 text-sm text-ink-muted">Pick a published fact to see its considerations.</p>
					</Card.Root>
				{/if}
			</aside>
		</div>
	{/if}
</div>
