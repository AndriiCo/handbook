<script lang="ts">
	import { resolve } from '$app/paths';
	import PrinterIcon from '@lucide/svelte/icons/printer';
	import TagIcon from '@lucide/svelte/icons/tag';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import InlineValue from './InlineValue.svelte';
	import { localItem, partOf, usable } from './items.js';
	import { isPoBox, postalCity } from './postal.js';
	import type { Item } from './contract.js';

	// The label offer beside a posting branch (plan §6, D10). It reads
	// the `/p2/orders/options` body the page fetched; it never fetches
	// and never holds a price of its own. The free route is already
	// complete above it: this is an extra, with the same weight as
	// doing it yourself, and it always names who provides it.
	//
	// - variant "courier": an enabled `label_fedex` with a live price
	//   (`from_cents` > 0) offers "Print a prepaid FedEx label to the
	//   {city} courier address — from {price}" (the order flow at the
	//   delivery step). Every paid control names its live price (plan §6
	//   rule 2), so an offer without one is not shown as an offer: it
	//   reads like a disabled one, which says plainly that labels aren't
	//   available and points back to the courier address. Needs a usable
	//   courier address.
	// - variant "mail": a free USPS address label (`address_label`), only
	//   while the mailing address is undisputed and the option is enabled.
	//
	// Hidden on empty pages, when the per-document fee isn't `ok`, and in
	// print (`data-paid` for the courier offer, `data-print-hide` for the
	// free label). Never place it inside a warning, a conflict box or a
	// card.
	//
	// Props:
	// - variant: 'courier' | 'mail'.
	// - options: the `/p2/orders/options` body, or null while unread.
	// - slug: the authority slug (order `destination`).
	// - courier: the `mailing.courier` Item (courier variant).
	// - usps: the `mailing.usps` Item (mail variant; must be undisputed).
	// - feeOk: the `fees.per_document` Item is `ok`.
	// - empty: the page has no public values.
	// - issuer: who the service is not: "the Secretary of State" when the
	//   office is one, else "the issuing office" (the page passes it).
	type OrderOption = {
		kind: string;
		enabled: boolean;
		from_cents?: number | null;
	};
	type OrderOptions = {
		currency?: string;
		fedex_destination?: { city?: string | null; fact_id?: number | null } | null;
		options?: OrderOption[];
	};

	let {
		variant,
		options = null,
		slug,
		courier = null,
		usps = null,
		feeOk = false,
		empty = false,
		issuer = 'the issuing office'
	}: {
		variant: 'courier' | 'mail';
		options?: OrderOptions | null;
		slug: string;
		courier?: Item | null;
		usps?: Item | null;
		feeOk?: boolean;
		empty?: boolean;
		issuer?: string;
	} = $props();

	const list = $derived(Array.isArray(options?.options) ? options.options : []);
	const fedex = $derived(list.find((o) => o?.kind === 'label_fedex') ?? null);
	const freeLabel = $derived(list.find((o) => o?.kind === 'address_label') ?? null);
	const courierOk = $derived(courier && usable(courier) && !isPoBox(courier.display) ? courier : null);
	const uspsOk = $derived(usps && usable(usps) ? usps : null);

	// The city is read from the courier value itself; the order options'
	// courier city (parsed by the server from an accepted fact) is the
	// fallback.
	const city = $derived.by((): Item | null => {
		if (courierOk) {
			const name = postalCity(courierOk.display);
			if (name) return partOf(courierOk, 'city', name);
		}
		const fallback = options?.fedex_destination;
		if (typeof fallback?.city === 'string' && fallback.city.trim() && typeof fallback.fact_id === 'number') {
			return localItem({
				id: 'orders-courier-city',
				label: 'Courier city',
				display: fallback.city.trim(),
				factIds: [fallback.fact_id],
				state: 'ok'
			});
		}
		return null;
	});

	const price = $derived.by(() => {
		const cents = fedex?.from_cents;
		if (typeof cents !== 'number' || !Number.isFinite(cents) || cents <= 0) return null;
		const code = (options?.currency || 'usd').toUpperCase();
		try {
			return new Intl.NumberFormat('en-US', { style: 'currency', currency: code }).format(cents / 100);
		} catch {
			return null;
		}
	});
	// The paid link only with its live price.
	const paidOffer = $derived(fedex?.enabled === true && price !== null);

	function orderHref(kind: string): string {
		const q = new URLSearchParams({ destination: slug, path: 'general', kind, step: 'delivery' });
		return `${resolve('/(customer)/order')}?${q}`;
	}

	const visible = $derived(
		!empty &&
			feeOk &&
			!!options &&
			(variant === 'courier' ? !!courierOk && !!fedex : !!uspsOk && freeLabel?.enabled === true)
	);
</script>

{#if visible}
	<div
		class="rounded-md border border-line bg-bg p-4 sm:p-5 print:hidden"
		data-offer={variant}
		data-paid={variant === 'courier' ? '' : undefined}
		data-print-hide={variant === 'mail' ? '' : undefined}
	>
		{#if variant === 'courier'}
			{#if paidOffer}
				<a
					href={orderHref('label_fedex')}
					class="group inline-flex min-h-11 items-start gap-2 py-2 font-medium text-accent-strong"
					data-kind="label_fedex"
				>
					<PrinterIcon class="mt-0.5 size-5 flex-none" aria-hidden="true" />
					<span>
						Print a prepaid FedEx label to the {#if city}<InlineValue item={city} />&nbsp;{/if}courier
						address&nbsp;— from <span data-price>{price}</span>
					</span>
					<ArrowRightIcon class="mt-1 size-4 flex-none transition-transform group-hover:translate-x-0.5" aria-hidden="true" />
				</a>
			{:else}
				<p class="m-0 text-ink">
					Prepaid courier labels aren't available for this office right now. You can send by FedEx or UPS
					yourself — copy the courier address above.
				</p>
			{/if}
			<p class="m-0 mt-2 text-sm text-ink-muted">Paid service by the Apostille Handbook, not {issuer}.</p>
		{:else}
			<a
				href={orderHref('address_label')}
				class="group inline-flex min-h-11 items-start gap-2 py-2 font-medium text-accent-strong"
				data-kind="address_label"
			>
				<TagIcon class="mt-0.5 size-5 flex-none" aria-hidden="true" />
				<span>Print a free address label for the mailing address</span>
				<ArrowRightIcon class="mt-1 size-4 flex-none transition-transform group-hover:translate-x-0.5" aria-hidden="true" />
			</a>
			<p class="m-0 mt-2 text-sm text-ink-muted">Free service by the Apostille Handbook, not {issuer}.</p>
		{/if}
	</div>
{/if}
