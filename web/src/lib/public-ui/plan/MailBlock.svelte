<script lang="ts">
	import MailIcon from '@lucide/svelte/icons/mail';
	import TagIcon from '@lucide/svelte/icons/tag';
	import TruckIcon from '@lucide/svelte/icons/truck';
	import PackageIcon from '@lucide/svelte/icons/package';
	import CitationChips from './CitationChips.svelte';
	import type { Office, PlanFact } from '../plan.js';

	// Where to mail: the accepted office's mailing address, a USPS-only
	// badge when that row says so (its fact one tap away), and the three
	// ways to send it through /order.
	let {
		body = '',
		mail = null,
		facts,
		factIds = [],
		orderHref
	}: {
		body?: string;
		mail?: Office['mail'] | null;
		facts: Map<number, PlanFact>;
		factIds?: number[];
		orderHref: (kind: string) => string;
	} = $props();

	const lines = $derived(
		mail
			? [
					mail.line1 || mail.po_box,
					[mail.city, [mail.region, mail.postal_code].filter(Boolean).join(' ')].filter(Boolean).join(', ')
				].filter((l): l is string => !!l)
			: []
	);
	const ctas = $derived([
		{ kind: 'address_label', label: 'Free address label', icon: TagIcon },
		{ kind: 'label_usps', label: 'Prepaid USPS label', icon: TruckIcon },
		...(mail?.usps_only ? [] : [{ kind: 'label_fedex', label: 'Prepaid FedEx label', icon: TruckIcon }]),
		{ kind: 'mail_kit', label: 'Mailing kit', icon: PackageIcon }
	]);
</script>

<div class="rounded-xl border border-line bg-bg p-5 shadow-sm">
	{#if body}<p class="m-0 text-ink">{body}</p>{/if}
	{#if lines.length > 0}
		<div class="mt-4 flex flex-wrap items-start gap-4">
			<address class="min-w-0 flex-1 rounded-lg border border-dashed border-line bg-surface p-4 font-mono text-sm break-words text-ink not-italic">
				<MailIcon class="mb-2 size-4 text-ink-muted" aria-hidden="true" />
				{#each lines as line, i (i)}<span class="block">{line}</span>{/each}
			</address>
			{#if mail?.usps_only}
				<span
					class="inline-flex min-h-7 items-center rounded-full border border-changed bg-changed-bg px-3 text-xs font-semibold text-changed"
				>
					USPS only
				</span>
			{/if}
		</div>
	{/if}
	<ul class="m-0 mt-4 grid list-none gap-2 p-0 sm:grid-cols-2">
		{#each ctas as cta (cta.kind)}
			{@const Icon = cta.icon}
			<li class="m-0 p-0">
				<a
					href={orderHref(cta.kind)}
					class="flex min-h-11 items-center gap-2 rounded-lg border border-line bg-bg px-4 text-sm font-medium text-ink no-underline transition-colors duration-200 hover:border-focus hover:bg-surface"
				>
					<Icon class="size-4 flex-none text-accent-strong" aria-hidden="true" />
					{cta.label}
				</a>
			</li>
		{/each}
	</ul>
	<div class="mt-4">
		<CitationChips ids={[...factIds, mail?.fact_id]} {facts} />
	</div>
</div>
