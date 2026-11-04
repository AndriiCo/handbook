<script lang="ts">
	import * as Popover from '../components/ui/popover/index.js';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import { citationOf, hostOf, safeHref, type Fact } from './orders';
	import { site } from '../locale.svelte.js';
	import { ORDER_COPY, fmtDay } from './order-copy.js';

	// Evidence one tap away: the accepted fact behind a disabled option
	// or a plan line, with its quote, source and capture date.
	let { factId, fact = null }: { factId: number; fact?: Fact | null } = $props();

	const c = $derived(ORDER_COPY[site.locale]);
	const cite = $derived(citationOf(fact));
	const host = $derived(hostOf(cite?.source_url));
	const when = $derived(cite?.captured_at ? fmtDay(cite.captured_at, site.locale) : '');
	const src = $derived(safeHref(cite?.source_url));
	const model = $derived(fact?.drafted_by ?? fact?.created_by ?? null);
</script>

<Popover.Root>
	<Popover.Trigger
		class="inline-flex min-h-11 cursor-pointer items-center gap-1 rounded-full px-1 text-sm font-medium text-accent-strong underline decoration-dotted underline-offset-4"
		aria-label={c.showFact(factId)}
	>
		<ShieldCheckIcon class="size-4 flex-none text-verified" aria-hidden="true" />
		<span>{c.factNo(factId)}</span>
	</Popover.Trigger>
	<Popover.Content class="w-80 max-w-[calc(100vw-2rem)]" align="start">
		{#if fact}
			<p class="m-0 text-sm text-ink">{fact.statement}</p>
			{#if cite}
				<blockquote class="font-quote mt-3 mb-0 border-l-2 border-verified pl-3 text-sm italic">
					“{cite.quote}”
				</blockquote>
			{/if}
			<p class="mt-3 mb-0 text-xs text-ink-muted">
				{#if host}<span class="font-mono">{host}</span>{/if}
				{#if when}<span> · {c.captured(when)}</span>{/if}
				{#if model}<span> · {c.draftedBy(model)}</span>{/if}
				{#if src}
					· <a class="underline underline-offset-4" href={src} target="_blank" rel="noopener noreferrer">{c.source}</a>
				{/if}
			</p>
		{:else}
			<p class="m-0 text-sm text-ink-muted">{c.factFallback(factId)}</p>
		{/if}
	</Popover.Content>
</Popover.Root>
