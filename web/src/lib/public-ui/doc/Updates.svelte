<script lang="ts">
	import type { Snippet } from 'svelte';
	import { changePanels, domain } from '../guide.js';
	import { traceAttrs } from './items.js';
	import { VISIBLE_UPDATES, shortDay, updateEntries, type UpdateEntry } from './updates.js';
	import type { Item } from './contract.js';

	// "Updates" (plan §3 section 10): resolved service notices and the
	// office pages' change notes, newest first; three visible, older ones
	// in a details element. A change too large to list (a re-captured
	// page whose diff runs past fifty lines) is one line: "Page
	// re-captured {date}; change too large to list".
	//
	// Change notes come from `GET /authorities/{slug}/changes`, read once
	// and lazily: the first time this block nears the viewport. A 404, an
	// empty or an unreadable body just leaves the notices (no console
	// error). Pass `changes` to render a body you already have instead.
	//
	// Props:
	// - items: `page.updates` (resolved notices and other dated Items).
	// - changes: a `/changes` body already read; skips the fetch.
	// - slug, apiBase: where to read `/changes` (apiBase as the page's
	//   `apiRoot`, e.g. "/apostille").
	// - marker: snippet `marker(item)` from the citations layer.
	let {
		items = [],
		changes = undefined,
		slug = '',
		apiBase = '',
		marker
	}: {
		items?: Item[];
		changes?: unknown;
		slug?: string;
		apiBase?: string;
		marker?: Snippet<[Item]>;
	} = $props();

	let root = $state<HTMLElement | null>(null);
	let fetched = $state<unknown>(undefined);
	let settled = $state(false);

	const body = $derived(changes !== undefined ? changes : fetched);
	const panels = $derived(body ? changePanels(body) : []);
	const entries = $derived(updateEntries({ items, panels }));
	const shown = $derived(entries.slice(0, VISIBLE_UPDATES));
	const folded = $derived(entries.slice(VISIBLE_UPDATES));
	const ready = $derived(changes !== undefined || settled || !slug);

	$effect(() => {
		const current = slug;
		const el = root;
		if (changes !== undefined || !current || !el) return;
		let cancelled = false;
		fetched = undefined;
		settled = false;
		const load = () => {
			fetch(`${apiBase}/api/v1/authorities/${encodeURIComponent(current)}/changes`, { credentials: 'same-origin' })
				.then((res) => (res.ok ? res.json().catch(() => null) : null))
				.then((json) => {
					if (cancelled) return;
					fetched = json;
					settled = true;
				})
				.catch(() => {
					if (cancelled) return;
					fetched = null;
					settled = true;
				});
		};
		if (typeof IntersectionObserver === 'undefined') {
			load();
			return () => {
				cancelled = true;
			};
		}
		const io = new IntersectionObserver(
			(seen) => {
				if (!seen.some((e) => e.isIntersecting)) return;
				io.disconnect();
				load();
			},
			{ rootMargin: '600px 0px' }
		);
		io.observe(el);
		return () => {
			cancelled = true;
			io.disconnect();
		};
	});
</script>

{#snippet entryRow(entry: UpdateEntry)}
	<li
		class="m-0 grid grid-cols-[minmax(0,1fr)_minmax(3.5ch,max-content)] gap-x-3 gap-y-1 border-b border-line py-3 sm:grid-cols-[minmax(9rem,14rem)_minmax(0,1fr)_minmax(3.5ch,max-content)]"
		data-update={entry.kind}
	>
		<p class="col-span-2 m-0 text-sm font-semibold text-ink-muted sm:col-span-1">
			{#if entry.date}<time datetime={entry.date}>{shortDay(entry.date)}</time>{/if}
		</p>
		{#if entry.kind === 'notice'}
			<div class="min-w-0">
				<p class="m-0 text-sm font-medium text-ink-muted">{entry.item.label}</p>
				<p class="m-0 mt-0.5 break-words text-ink [overflow-wrap:anywhere]" {...traceAttrs(entry.item)}>
					{entry.item.display}
				</p>
			</div>
			<span class="text-right text-sm leading-6">{@render marker?.(entry.item)}</span>
		{:else if entry.degenerate}
			<p class="m-0 min-w-0 break-words text-ink">
				Page re-captured {shortDay(entry.date)}; change too large to list
				<span class="text-sm text-ink-muted">· {domain(entry.panel.sourceUrl)}</span>
			</p>
			<span></span>
		{:else}
			<div class="min-w-0">
				<p class="m-0 text-sm font-medium text-ink-muted">Changes on {domain(entry.panel.sourceUrl)}</p>
				<ul class="m-0 mt-1 list-none p-0">
					{#each entry.panel.added as line, i (`a-${i}`)}
						<li class="m-0 border-l-2 border-verified py-0.5 pl-3 text-sm break-words text-ink [overflow-wrap:anywhere]">
							<span class="sr-only">Added: </span><span aria-hidden="true" class="font-mono">+&nbsp;</span>{line}
						</li>
					{/each}
					{#each entry.panel.removed as line, i (`r-${i}`)}
						<li
							class="m-0 border-l-2 border-changed py-0.5 pl-3 text-sm break-words text-ink-muted line-through [overflow-wrap:anywhere]"
						>
							<span class="sr-only">Removed: </span><span aria-hidden="true" class="font-mono">−&nbsp;</span>{line}
						</li>
					{/each}
				</ul>
				{#if entry.panel.more > 0}
					<p class="m-0 mt-1 text-sm text-ink-muted">
						{entry.panel.more === 1 ? 'One more changed line' : `${entry.panel.more} more changed lines`} on the page.
					</p>
				{/if}
			</div>
			<span></span>
		{/if}
	</li>
{/snippet}

<div bind:this={root} class="flex flex-col gap-2" data-updates>
	{#if entries.length > 0}
		<ol class="m-0 list-none border-t border-line p-0">
			{#each shown as entry (entry.key)}{@render entryRow(entry)}{/each}
		</ol>
		{#if folded.length > 0}
			<details>
				<summary class="inline-flex min-h-11 cursor-pointer items-center font-medium text-accent-strong">
					Older updates
				</summary>
				<ol class="m-0 list-none border-t border-line p-0">
					{#each folded as entry (entry.key)}{@render entryRow(entry)}{/each}
				</ol>
			</details>
		{/if}
	{:else if ready}
		<p class="m-0 text-ink-muted">No changes to list yet.</p>
	{/if}
</div>
