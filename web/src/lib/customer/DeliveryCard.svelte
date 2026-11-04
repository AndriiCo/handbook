<script lang="ts">
	import PrinterIcon from '@lucide/svelte/icons/printer';
	import StampIcon from '@lucide/svelte/icons/stamp';
	import PackageIcon from '@lucide/svelte/icons/package';
	import TagIcon from '@lucide/svelte/icons/tag';
	import CheckIcon from '@lucide/svelte/icons/check';
	import FactChip from './FactChip.svelte';
	import type { DeliveryOption, Fact } from './orders';
	import { site } from '../locale.svelte.js';
	import { ORDER_COPY } from './order-copy.js';
	import { money } from './orders';

	// One delivery option from GET /p2/orders/options. Title, price and
	// the disabled reason are the API's; a disabled card cites the
	// accepted fact that rules it out.
	let {
		option,
		currency,
		selected,
		fact = null,
		onselect
	}: {
		option: DeliveryOption;
		currency: string;
		selected: boolean;
		fact?: Fact | null;
		onselect: (kind: DeliveryOption['kind']) => void;
	} = $props();

	const ICONS = { label_fedex: PrinterIcon, label_usps: StampIcon, mail_kit: PackageIcon, address_label: TagIcon };
	const Icon = $derived(ICONS[option.kind] ?? TagIcon);
	const c = $derived(ORDER_COPY[site.locale]);
	const price = $derived(
		option.from_cents === null
			? null
			: option.from_cents === 0
				? c.free
				: c.fromPrice(money(option.from_cents, currency, { compact: true }, site.locale))
	);
	const id = $derived(`delivery-${option.kind}`);
</script>

<div
	class={`flex flex-col overflow-hidden rounded-lg border transition-[box-shadow,border-color,background-color] duration-200 has-[input:focus-visible]:ring-3 has-[input:focus-visible]:ring-focus/40 ${
		!option.enabled
			? 'border-line bg-surface'
			: selected
				? 'border-accent-fill bg-bg shadow-md ring-1 ring-accent-fill'
				: 'border-line bg-bg shadow-sm hover:border-brand-300 hover:shadow-md'
	}`}
	data-kind={option.kind}
	data-enabled={option.enabled ? 'yes' : 'no'}
>
	<label
		for={id}
		class={`flex min-h-20 items-start gap-3 p-4 ${option.enabled ? 'cursor-pointer' : 'cursor-not-allowed'}`}
	>
		<input
			class="sr-only"
			type="radio"
			name="delivery"
			{id}
			value={option.kind}
			checked={selected}
			disabled={!option.enabled}
			onchange={() => onselect(option.kind)}
		/>
		<span
			aria-hidden="true"
			class={`flex size-10 flex-none items-center justify-center rounded-md ${
				option.enabled ? 'bg-surface-2 text-accent-strong' : 'bg-surface-2 text-ink-muted'
			}`}
		>
			<Icon class="size-5" />
		</span>
		<span class="flex min-w-0 flex-1 flex-col gap-1">
			<span class={`font-semibold ${option.enabled ? 'text-ink' : 'text-ink-muted'}`}>{option.title}</span>
			{#if price && option.enabled}
				<span class="text-sm text-ink-muted">{price}</span>
			{:else if !option.enabled}
				<span class="text-sm text-ink-muted">{c.unavailableHere}</span>
			{/if}
		</span>
		{#if option.enabled}
			<span
				aria-hidden="true"
				class={`mt-1 flex size-5 flex-none items-center justify-center rounded-full border transition-colors duration-150 ${
					selected ? 'border-accent-fill bg-accent-fill text-accent-ink' : 'border-line bg-bg'
				}`}
			>
				{#if selected}<CheckIcon class="size-3.5" />{/if}
			</span>
		{/if}
	</label>
	{#if !option.enabled && option.reason}
		<div class="flex flex-wrap items-center gap-x-2 border-t border-line px-4 py-2 text-sm text-ink">
			<span class="min-w-0 flex-1 basis-48">{option.reason}</span>
			{#if option.fact_id !== null}
				<FactChip factId={option.fact_id} {fact} />
			{/if}
		</div>
	{/if}
</div>
