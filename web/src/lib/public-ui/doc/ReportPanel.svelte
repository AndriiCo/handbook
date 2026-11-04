<script lang="ts">
	import { onMount } from 'svelte';
	import MinusIcon from '@lucide/svelte/icons/minus';
	import XIcon from '@lucide/svelte/icons/x';
	import MessageSquareWarning from '@lucide/svelte/icons/message-square-warning';
	import ReportForm from '../ReportForm.svelte';

	// "Report a change" docked beside the dossier, not over it: a
	// non-modal panel on the right at desktop widths (the page shifts
	// left to stay fully readable) and a bottom sheet on phones that
	// folds down to a bar, so the visitor reads the office's page while
	// writing what changed. Closing or folding never loses the note
	// (ReportForm keeps it in localStorage until sent). Loaded only
	// when the visitor opens it.
	//
	// Props: slug, name (the office), apiBase, loginHref, onclose.
	let {
		slug,
		name,
		apiBase = '',
		loginHref,
		onclose
	}: {
		slug: string;
		name: string;
		apiBase?: string;
		loginHref: string;
		onclose: () => void;
	} = $props();

	const WIDE = '(min-width: 64rem)';
	const PANEL = '28rem';

	let folded = $state(false);
	let wide = $state(false);
	let heading = $state<HTMLElement | null>(null);
	let page = $state<HTMLElement | null>(null);
	// Client-only (opened on a click), so `document` is there.
	const before = document.body.style.paddingRight;

	onMount(() => {
		page = document.querySelector('main') ?? document.body;
		heading?.focus();
		const mq = window.matchMedia(WIDE);
		const fit = () => (wide = mq.matches);
		fit();
		mq.addEventListener('change', fit);
		return () => {
			mq.removeEventListener('change', fit);
			document.body.style.paddingRight = before;
		};
	});

	// Full class strings per state (never stacked): open docks to the
	// right edge at desktop widths, folded shrinks to a corner card.
	const panelCls = (f: boolean) =>
		f
			? `fixed inset-x-0 bottom-0 z-40 flex flex-col rounded-t-xl border border-line bg-bg shadow-2xl print:hidden lg:inset-x-auto lg:right-4 lg:bottom-4 lg:w-80 lg:rounded-xl`
			: `fixed inset-x-0 bottom-0 z-40 flex max-h-[80dvh] flex-col rounded-t-xl border border-line bg-bg shadow-2xl print:hidden lg:inset-x-auto lg:top-0 lg:right-0 lg:max-h-none lg:w-[28rem] lg:rounded-none lg:border-y-0 lg:border-r-0`;

	// At desktop widths the page makes room for the panel instead of
	// hiding under it.
	$effect(() => {
		document.body.style.paddingRight = wide && !folded ? PANEL : before;
	});
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape' && document.activeElement?.closest('[data-report-panel]')) onclose();
	}}
/>

<aside
	data-report-panel
	aria-labelledby="report-panel-h"
	class={panelCls(folded)}
>
	<header class="flex items-center gap-2 border-b border-line px-4 py-2.5">
		<MessageSquareWarning class="size-5 flex-none text-accent-strong" aria-hidden="true" />
		<button
			type="button"
			class="min-w-0 flex-1 text-left"
			onclick={() => (folded = !folded)}
			aria-expanded={!folded}
			aria-controls="report-panel-body"
		>
			<h2
				id="report-panel-h"
				bind:this={heading}
				tabindex="-1"
				class="m-0 truncate text-base font-semibold text-ink outline-none"
			>
				Report a change
			</h2>
			{#if folded}
				<span class="block truncate text-xs text-ink-muted">Draft kept · tap to keep writing</span>
			{/if}
		</button>
		<button
			type="button"
			onclick={() => (folded = !folded)}
			class="inline-flex size-9 items-center justify-center rounded-sm hover:bg-surface"
			aria-label={folded ? 'Open the report form' : 'Fold the report form away to read the page'}
		>
			<MinusIcon class="size-4" aria-hidden="true" />
		</button>
		<button
			type="button"
			onclick={onclose}
			class="inline-flex size-9 items-center justify-center rounded-sm hover:bg-surface"
			aria-label="Close (your draft is kept)"
		>
			<XIcon class="size-4" aria-hidden="true" />
		</button>
	</header>
	<div id="report-panel-body" class="overflow-y-auto overscroll-contain px-4 py-4" hidden={folded}>
		<p class="mt-0 mb-4 text-sm text-ink-muted">
			Spotted a changed fee, address or step at {name}? Keep this page open beside the form, compare,
			and tell us what is different now.
		</p>
		<ReportForm {slug} officeName={name} {apiBase} {loginHref} quoteFrom={page} />
	</div>
</aside>
