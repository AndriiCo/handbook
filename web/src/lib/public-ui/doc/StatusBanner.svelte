<script lang="ts">
	import type { Snippet } from 'svelte';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import { formatDay } from '../cite.js';
	import { traceAttrs } from './items.js';
	import type { Item } from './contract.js';

	type Status = {
		active: Item[];
		resolved: Item[];
		lastRead: null | { capturedAt: string; url: string };
	};

	// Service status, right under the h1 (plan §3 row 0a, D9).
	// - An active notice (no restore yet) is one banner: a region named
	//   "Service notice", each notice verbatim with its capture day.
	// - Each resolved pair (seven days after its restore) as a muted
	//   line, the notice and the office's restore, both verbatim: under
	//   the banner while another notice is active, so every notice the
	//   page numbers is on it.
	// - With none active, those lines follow "No open service notice in
	//   the office's notice log (last read …)" when a paired notice from
	//   that log exists.
	// Text plus an icon, never colour alone. No blockquote here: the
	// quotes live in the citation card and the endnotes.
	//
	// Props:
	// - status: `page.status` (present.js).
	// - marker: snippet `marker(item)` (the row gutter), for banners.
	// - inline: snippet `inline(item)` (markers inside a sentence).
	let {
		status,
		marker,
		inline
	}: { status: Status; marker?: Snippet<[Item]>; inline?: Snippet<[Item]> } = $props();

	const uid = $props.id();

	const captured = (item: Item) => {
		const at = item.cites.map((c) => c.source.capturedAt).find((v) => !!v);
		return at ? formatDay(at) : '';
	};

	// Resolved notices pair by instance: the notice, then its restore.
	const pairs = $derived.by(() => {
		const out: { key: string; notice: Item | null; restored: Item | null }[] = [];
		for (const item of status.resolved) {
			const key = item.instance ?? item.id;
			let pair = out.find((p) => p.key === key);
			if (!pair) {
				pair = { key, notice: null, restored: null };
				out.push(pair);
			}
			if (item.slot === 'service_notice.restored') pair.restored = item;
			else pair.notice = item;
		}
		return out;
	});
	const lastRead = $derived(status.lastRead ? formatDay(status.lastRead.capturedAt) : '');
</script>

{#if status.active.length > 0}
	<section
		aria-labelledby={`${uid}-h`}
		class="mt-6 max-w-prose rounded-md border border-changed bg-changed-bg p-4 sm:p-5"
		data-status="active"
	>
		<h2 id={`${uid}-h`} class="m-0 flex items-center gap-2 text-body font-semibold text-ink">
			<TriangleAlertIcon class="size-5 flex-none text-changed" aria-hidden="true" />
			Service notice
		</h2>
		<ul class="m-0 mt-2 list-none p-0">
			{#each status.active as item (item.id)}
				{@const day = captured(item)}
				<li class="m-0 grid grid-cols-[minmax(0,1fr)_minmax(3.5ch,max-content)] gap-x-3 py-1" data-item={item.id}>
					<div class="min-w-0">
						<p class="m-0 break-words text-ink [overflow-wrap:anywhere]" {...traceAttrs(item)}>{item.display}</p>
						{#if day}
							<p class="m-0 mt-1 text-sm text-ink-muted">Captured from the office's page {day}</p>
						{/if}
					</div>
					<span class="text-right text-sm leading-6">{@render marker?.(item)}</span>
				</li>
			{/each}
		</ul>
	</section>
{/if}
{#if (lastRead && status.active.length === 0) || pairs.length > 0}
	<div class="mt-5 flex max-w-prose items-start gap-2 text-sm text-ink-muted" data-status="resolved">
		<CircleCheckIcon class="mt-0.5 size-4 flex-none text-verified" aria-hidden="true" />
		<div class="flex min-w-0 flex-col gap-1">
			{#if lastRead && status.active.length === 0}
				<p class="m-0">No open service notice in the office's notice log (last read {lastRead}).</p>
			{/if}
			{#each pairs as pair (pair.key)}
				<p class="m-0 break-words [overflow-wrap:anywhere]" data-item={pair.restored?.id ?? pair.notice?.id}>
					<span class="font-medium text-ink">Resolved:</span>
					{#if pair.notice}<span {...traceAttrs(pair.notice)}>{pair.notice.display}</span>{@render inline?.(pair.notice)}{/if}
					{#if pair.notice && pair.restored}<span aria-hidden="true">&nbsp;—</span>{/if}
					{#if pair.restored}<span {...traceAttrs(pair.restored)}>{pair.restored.display}</span>{@render inline?.(pair.restored)}{/if}
				</p>
			{/each}
		</div>
	</div>
{/if}
