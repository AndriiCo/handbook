<script lang="ts">
	import * as Popover from '../components/ui/popover/index.js';
	import { domain, markQuote } from './guide.js';

	type Citation = { quote?: string; source_url?: string; captured_at?: string };

	let {
		citation = null,
		value = '',
		draftedBy = 'unknown'
	}: { citation: Citation | null; value: string; draftedBy: string } = $props();

	const host = $derived(typeof citation?.source_url === 'string' ? domain(citation.source_url) : null);
	const when = $derived(typeof citation?.captured_at === 'string' ? String(citation.captured_at).split('T')[0] : '');
	const quote = $derived(String(citation?.quote ?? ''));
</script>

{#if citation && quote}
	<Popover.Root>
		<Popover.Trigger
			class="inline-flex min-h-11 cursor-pointer items-center font-medium text-accent-strong underline decoration-dotted underline-offset-4"
		>
			Citation
		</Popover.Trigger>
		<Popover.Content class="max-w-80" align="start">
			<blockquote class="font-quote m-0 border-l-2 border-verified pl-3 italic">
				{@html markQuote(quote, value)}
			</blockquote>
			<p class="mt-2 mb-0 text-xs text-ink-muted">
				{#if host}<span class="font-mono">{host}</span> · {/if}
				{#if when}<span>captured {when}</span> · {/if}
				<span>Drafted by {draftedBy} · approved by staff</span>
			</p>
		</Popover.Content>
	</Popover.Root>
{/if}
