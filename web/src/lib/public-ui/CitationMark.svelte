<script lang="ts">
	import * as Popover from '../components/ui/popover/index.js';
	import QuoteIcon from '@lucide/svelte/icons/quote';
	import { domain, markQuote, sourceUrl } from './guide.js';

	type Citation = { quote?: string; source_url?: string; captured_at?: string };

	// Evidence one tap (or, with a mouse, one hover) away: the quote with
	// the cited span marked, its source and capture date. `chip` is the
	// plan's compact form (domain · date); `link` the guide's text link.
	let {
		citation = null,
		value = '',
		draftedBy = 'unknown',
		variant = 'link',
		label = ''
	}: {
		citation: Citation | null;
		value: string;
		draftedBy: string;
		variant?: 'link' | 'chip';
		label?: string;
	} = $props();

	const host = $derived(typeof citation?.source_url === 'string' ? domain(citation.source_url) : null);
	const when = $derived(typeof citation?.captured_at === 'string' ? String(citation.captured_at).split('T')[0] : '');
	const quote = $derived(String(citation?.quote ?? ''));
	const src = $derived(sourceUrl(citation?.source_url, quote));

	let open = $state(false);
	let timer: ReturnType<typeof setTimeout> | null = null;
	function later(next: boolean, ms: number) {
		if (timer) clearTimeout(timer);
		timer = setTimeout(() => {
			open = next;
			timer = null;
		}, ms);
	}
	const hoverIn = (e: PointerEvent) => {
		if (e.pointerType === 'mouse') later(true, 150);
	};
	const hoverOut = (e: PointerEvent) => {
		if (e.pointerType === 'mouse') later(false, 200);
	};
</script>

{#if citation && quote}
	<Popover.Root bind:open>
		<Popover.Trigger
			onpointerenter={hoverIn}
			onpointerleave={hoverOut}
			aria-label={variant === 'chip'
				? `Citation: ${label ? `${label}, ` : ''}${host ?? 'source'}${when ? `, captured ${when}` : ''}`
				: undefined}
			class={variant === 'chip'
				? 'inline-flex min-h-9 cursor-pointer items-center gap-1.5 rounded-full border border-line bg-bg px-3 font-mono text-xs text-ink-muted transition-colors hover:border-focus hover:text-ink data-[state=open]:border-focus data-[state=open]:text-ink'
				: 'inline-flex min-h-11 cursor-pointer items-center font-medium text-accent-strong underline decoration-dotted underline-offset-4'}
		>
			{#if variant === 'chip'}
				<QuoteIcon class="size-3 flex-none text-verified" aria-hidden="true" />
				{#if label}<span class="font-sans font-medium text-ink">{label}</span><span aria-hidden="true">·</span>{/if}
				<span>{host ?? 'source'}</span>
			{:else}
				Citation
			{/if}
		</Popover.Trigger>
		<Popover.Content
			class="max-w-80"
			align="start"
			onpointerenter={hoverIn}
			onpointerleave={hoverOut}
		>
			<blockquote class="font-quote m-0 border-l-2 border-verified pl-3 italic">
				{@html markQuote(quote, value)}
			</blockquote>
			<p class="mt-2 mb-0 text-xs text-ink-muted">
				{#if host}<span class="font-mono">{host}</span> · {/if}
				{#if when}<span>captured {when}</span> · {/if}
				<span>Drafted by {draftedBy} · approved by staff</span>
				{#if src} ·
					<a href={src} target="_blank" rel="noopener noreferrer">Source</a>
				{/if}
			</p>
		</Popover.Content>
	</Popover.Root>
{/if}
