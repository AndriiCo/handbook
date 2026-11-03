<script lang="ts">
	import { fly } from 'svelte/transition';
	import * as Sheet from '#lib/components/ui/sheet/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import StatusTrack from '#lib/customer/StatusTrack.svelte';
	import { KIND_LABEL, carrierName, serviceName, humanize, type OrderKind } from '#lib/customer/orders.js';
	import OrderStatusBadge from './OrderStatusBadge.svelte';
	import { staffMoves, canRefund } from './order-moves.js';
	import { staffReason, type AdminOrder, type AdminOrderRow } from './orders.js';
	import { dateTime, fmtCents } from './format.js';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import FlagIcon from '@lucide/svelte/icons/flag';
	import UndoIcon from '@lucide/svelte/icons/undo-2';
	import BanIcon from '@lucide/svelte/icons/ban';
	import TruckIcon from '@lucide/svelte/icons/truck';
	import ReceiptIcon from '@lucide/svelte/icons/receipt';
	import LoaderIcon from '@lucide/svelte/icons/loader-circle';

	// Staff order detail (W1c): timeline, shipments, payment and the
	// moves the server will accept from this state (order-moves.js).
	// The page owns data and optimistic updates; this renders them.
	let {
		open = $bindable(false),
		order,
		row,
		loadState,
		error,
		busy,
		authorityName,
		motion,
		onAdvance,
		onRefund,
		onRetry,
		onClosed
	}: {
		open?: boolean;
		order: AdminOrder | null;
		row: AdminOrderRow | null;
		loadState: 'loading' | 'ready' | 'failed';
		error: string | null;
		busy: string | null;
		authorityName: string;
		motion: number;
		onAdvance: (to: string) => void;
		onRefund: () => void;
		onRetry: () => void;
		onClosed: () => void;
	} = $props();

	let confirmOpen = $state(false);
	let content = $state<HTMLElement | null>(null);

	const id = $derived(order?.id ?? row?.id ?? '');
	const status = $derived(order?.status ?? row?.status ?? '');
	const kind = $derived(order?.kind ?? row?.kind ?? '');
	const paid = $derived(!!order?.receipt);
	const moves = $derived(order ? staffMoves({ status: order.status, kind: order.kind, paid }) : []);
	const forward = $derived(moves.filter((m) => m.role === 'primary' || m.role === 'secondary'));
	const flag = $derived(moves.find((m) => m.role === 'flag') ?? null);
	const cancel = $derived(moves.find((m) => m.role === 'cancel') ?? null);
	const refundable = $derived(order ? canRefund(order) : false);
	const reason = $derived(status === 'needs_staff' ? staffReason(order, row) : null);
	const events = $derived([...(order?.events ?? [])].reverse());
	// An order with staff keeps the track at the last stage its
	// timeline reached, so staff see where it stalled (refunded and
	// canceled orders keep their own status: the track dims).
	const trackStatus = $derived.by(() => {
		if (!order) return '';
		if (order.status !== 'needs_staff') return order.status;
		for (const ev of events) {
			if (ev.kind === 'label.ready') return 'labels_ready';
			if (ev.kind === 'kit.packed') return 'packing';
			const s = ev.kind.startsWith('order.') ? ev.kind.slice(6) : '';
			if (TRACKED.includes(s)) return s;
		}
		return order.status;
	});
	const total = $derived(order?.amount ? fmtCents(order.amount.total_cents, order.amount.currency) : row ? fmtCents(row.total_cents, row.currency) : '—');
	const isKind = (k: string): k is OrderKind => k in KIND_LABEL;
	const TRACKED = ['paid', 'label_purchasing', 'labels_ready', 'packing', 'shipped', 'in_transit', 'delivered'];

	const ROLE: Record<string, string> = {
		kit_to_customer: 'Kit to customer',
		outbound: 'Outbound to authority',
		return: 'Return to customer'
	};
	// "USPS Priority", not "USPS · USPS Priority".
	const legService = (carrier: string, service: string) => {
		const name = carrierName(carrier);
		const svc = serviceName(service, carrier);
		return svc.toLowerCase().startsWith(name.toLowerCase()) ? svc : [name, svc].filter(Boolean).join(' · ');
	};
	const legStatus = (s: string) =>
		s === 'label_ready' ? 'Label ready' : s === 'pending' ? 'Awaiting label' : humanize(s);
	const legTone = (s: string) =>
		s === 'label_ready' || s === 'delivered'
			? 'border-verified bg-verified-bg text-verified'
			: s === 'pending'
				? 'border-line bg-surface text-ink-muted'
				: 'border-line bg-surface-2 text-accent-strong';
	const dotTone = (k: string) =>
		k === 'order.needs_staff'
			? 'bg-changed'
			: k === 'order.paid' || k === 'order.delivered' || k === 'label.ready' || k === 'order.labels_ready'
				? 'bg-verified'
				: k === 'order.refunded' || k === 'order.canceled'
					? 'bg-ink-muted'
					: 'bg-accent-strong';
	const place = (o: AdminOrder) =>
		[o.return_to?.city, [o.return_to?.region, o.return_to?.postal_code].filter(Boolean).join(' '), o.return_to?.country]
			.filter((p) => typeof p === 'string' && p.trim())
			.join(', ');

	// Full width on phones, a roomy reading column from sm up.
	const SHEET = 'gap-0 overflow-y-auto p-0 outline-none data-[side=right]:w-full data-[side=right]:sm:max-w-xl';

	function focusSheet(e: Event) {
		// Land on the sheet itself, never on a state-changing button.
		// No ring on the whole panel: the next Tab shows one as usual.
		e.preventDefault();
		content?.focus({ preventScroll: true, focusVisible: false } as FocusOptions);
	}

	function confirmRefund() {
		confirmOpen = false;
		onRefund();
	}
</script>

<Sheet.Root bind:open onOpenChangeComplete={(o) => !o && onClosed()}>
	<Sheet.Content
		bind:ref={content}
		tabindex={-1}
		onOpenAutoFocus={focusSheet}
		class={SHEET}
	>
		<Sheet.Header class="gap-3 border-b border-line p-6 pr-14">
			<div class="flex flex-wrap items-center gap-2 text-xs text-ink-muted">
				<span class="font-semibold tracking-wide uppercase">{isKind(kind) ? KIND_LABEL[kind] : humanize(kind || 'order')}</span>
				{#if (order?.mode ?? row?.mode) === 'test'}
					<span class="rounded-xs border border-line px-1.5 font-mono">test mode</span>
				{:else if order?.mode ?? row?.mode}
					<span class="rounded-xs border border-changed bg-changed-bg px-1.5 font-mono text-changed">live</span>
				{/if}
			</div>
			<Sheet.Title class="font-mono text-h3 font-semibold tracking-tight break-all">{id}</Sheet.Title>
			<Sheet.Description class="m-0 flex flex-col gap-0.5 text-sm text-ink-muted">
				<span class="text-ink">{authorityName}</span>
				{#if order}<span>Placed {dateTime(order.created_at)}</span>{/if}
			</Sheet.Description>
			<div class="flex flex-wrap items-center justify-between gap-3 pt-1">
				{#if status}<OrderStatusBadge {status} size="md" />{/if}
				<span class="text-h3 font-semibold tabular-nums">{total}</span>
			</div>
		</Sheet.Header>

		{#if loadState === 'loading' && !order}
			<div class="flex flex-col gap-6 p-6" role="status" aria-label="Loading order">
				<Skeleton class="h-16 rounded-md" />
				<Skeleton class="h-10 w-2/3 rounded-md" />
				<Skeleton class="h-28 rounded-md" />
				<Skeleton class="h-40 rounded-md" />
			</div>
		{:else if loadState === 'failed' && !order}
			<div class="p-6">
				<Notice variant="error" heading="This order could not load.">
					<p class="m-0">{error}</p>
					<p class="mt-3 mb-0"><Button onclick={onRetry}>Try again</Button></p>
				</Notice>
			</div>
		{:else if order}
			{#if reason}
				<div class="flex items-start gap-3 border-b border-changed bg-changed-bg px-6 py-4" role="status">
					<TriangleAlertIcon class="mt-0.5 size-4 flex-none text-changed" aria-hidden="true" />
					<div class="flex min-w-0 flex-col gap-1">
						<p class="m-0 text-sm font-semibold text-changed">Waiting on staff</p>
						<p class="m-0 text-sm text-ink">{reason}</p>
					</div>
				</div>
			{/if}

			<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-actions">
				<h3 id="sheet-actions" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Next step</h3>
				{#if forward.length > 0 || flag || cancel || refundable}
					<div class="flex flex-wrap items-center gap-2">
						{#each forward as m (m.to)}
							<Button
								size="lg"
								variant={m.role === 'primary' ? 'default' : 'outline'}
								disabled={busy !== null}
								onclick={() => onAdvance(m.to)}
							>
								{#if busy === m.to}
									<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />
								{:else if m.role === 'primary'}
									<ArrowRightIcon aria-hidden="true" />
								{/if}
								{m.label}
							</Button>
						{/each}
						{#if flag}
							<Button size="lg" variant="ghost" disabled={busy !== null} onclick={() => onAdvance(flag.to)}>
								{#if busy === flag.to}<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<FlagIcon aria-hidden="true" />{/if}
								{flag.label}
							</Button>
						{/if}
					</div>
					{#if cancel || refundable}
						<div class="flex flex-wrap items-center gap-2 border-t border-dashed border-line pt-3">
							{#if cancel}
								<Button size="lg" variant="destructive" disabled={busy !== null} onclick={() => onAdvance(cancel.to)}>
									{#if busy === cancel.to}<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<BanIcon aria-hidden="true" />{/if}
									{cancel.label}
								</Button>
							{/if}
							{#if refundable}
								<Button size="lg" variant="destructive" disabled={busy !== null} onclick={() => (confirmOpen = true)}>
									{#if busy === 'refund'}<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<UndoIcon aria-hidden="true" />{/if}
									Refund {total}
								</Button>
							{/if}
						</div>
					{/if}
				{:else}
					<p class="m-0 text-sm text-ink-muted">
						{#if status === 'awaiting_payment'}
							The customer is in checkout. Staff moves open once the payment settles.
						{:else if status === 'delivered' || status === 'refunded' || status === 'canceled'}
							This order is closed. Nothing left to move.
						{:else}
							No staff move from this state.
						{/if}
					</p>
				{/if}
			</section>

			{#if isKind(order.kind)}
				<section class="border-b border-line px-6 py-5" aria-label="Progress">
					<StatusTrack kind={order.kind} status={trackStatus} />
				</section>
			{/if}

			<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-timeline">
				<h3 id="sheet-timeline" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Timeline</h3>
				{#if events.length === 0}
					<p class="m-0 text-sm text-ink-muted">No events recorded yet.</p>
				{:else}
					<ol class="m-0 flex list-none flex-col p-0" aria-live="polite">
						{#each events as ev, i (`${ev.at}|${ev.kind}|${ev.text}`)}
							<li class="relative flex gap-3 pb-4 pl-6 last:pb-0" in:fly={{ y: -6, duration: motion }}>
								{#if i < events.length - 1}
									<span class="absolute top-4 bottom-0 left-1 w-0.5 rounded-full bg-surface-2" aria-hidden="true"></span>
								{/if}
								<span
									class={`absolute top-1.5 left-0 size-2.5 rounded-full ring-4 ring-popover ${dotTone(ev.kind)} ${ev.pending ? 'motion-safe:animate-pulse' : ''}`}
									aria-hidden="true"
								></span>
								<div class="flex min-w-0 flex-col gap-0.5">
									<p class="m-0 text-sm text-ink">{ev.text}</p>
									<p class="m-0 flex flex-wrap items-center gap-2 text-xs text-ink-muted">
										{#if ev.pending}
											<span>Saving</span>
										{:else}
											<time datetime={new Date(ev.at * 1000).toISOString()}>{dateTime(ev.at)}</time>
										{/if}
										<span class="font-mono">{ev.kind}</span>
										{#if ev.leg}<span>{ROLE[ev.leg] ?? humanize(ev.leg)}</span>{/if}
									</p>
								</div>
							</li>
						{/each}
					</ol>
				{/if}
			</section>

			<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-shipments">
				<h3 id="sheet-shipments" class="m-0 flex items-center gap-2 text-xs font-semibold tracking-wide text-ink-muted uppercase">
					<TruckIcon class="size-4" aria-hidden="true" /> Shipments
				</h3>
				{#if order.legs.length === 0}
					<p class="m-0 text-sm text-ink-muted">No carrier legs on this order.</p>
				{:else}
					<ul class="m-0 flex list-none flex-col divide-y divide-line rounded-md border border-line p-0">
						{#each order.legs as leg (leg.role)}
							<li class="flex flex-wrap items-center justify-between gap-2 px-4 py-3">
								<div class="flex min-w-0 flex-col gap-0.5">
									<span class="text-sm font-medium">{ROLE[leg.role] ?? humanize(leg.role)}</span>
									<span class="text-xs text-ink-muted">
										{legService(leg.carrier, leg.service)}
									</span>
									{#if leg.tracking_number}
										<span class="font-mono text-xs break-all">{leg.tracking_number}</span>
									{/if}
								</div>
								<span class={`inline-flex h-6 items-center rounded-full border px-2 text-xs font-medium whitespace-nowrap ${legTone(leg.status)}`}>
									{legStatus(leg.status)}
								</span>
							</li>
						{/each}
					</ul>
				{/if}
			</section>

			<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-payment">
				<h3 id="sheet-payment" class="m-0 flex items-center gap-2 text-xs font-semibold tracking-wide text-ink-muted uppercase">
					<ReceiptIcon class="size-4" aria-hidden="true" /> Payment
				</h3>
				{#if order.amount && order.amount.lines?.length}
					<dl class="m-0 flex flex-col gap-2">
						{#each order.amount.lines as line, i (i)}
							<div class="flex items-baseline justify-between gap-4 text-sm">
								<dt class="min-w-0 text-ink-muted">{line.label}</dt>
								<dd class="m-0 font-mono tabular-nums">{fmtCents(line.cents, order.amount.currency)}</dd>
							</div>
						{/each}
						<div class="flex items-baseline justify-between gap-4 border-t border-line pt-2 text-sm font-semibold">
							<dt>Total</dt>
							<dd class="m-0 font-mono tabular-nums">{total}</dd>
						</div>
					</dl>
				{/if}
				<p class="m-0 text-sm">
					{#if order.receipt?.refunded || order.status === 'refunded'}
						<span class="font-medium">Refunded in full.</span>
						<span class="text-ink-muted">Paid {dateTime(order.receipt?.paid_at ?? order.updated_at)}.</span>
					{:else if order.receipt}
						<span class="font-medium text-verified">Paid</span>
						<span class="text-ink-muted">{dateTime(order.receipt.paid_at)} · Stripe {order.mode === 'live' ? 'live' : 'test'} mode</span>
					{:else if order.kind === 'address_label'}
						<span class="text-ink-muted">Free printable label: no payment.</span>
					{:else}
						<span class="text-ink-muted">Not paid yet.</span>
					{/if}
				</p>
			</section>

			<section class="flex flex-col gap-3 px-6 py-5" aria-labelledby="sheet-customer">
				<h3 id="sheet-customer" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Customer</h3>
				<dl class="m-0 grid grid-cols-3 gap-x-4 gap-y-2 text-sm">
					<dt class="text-ink-muted">Reference</dt>
					<dd class="col-span-2 m-0 font-mono break-all">{order.customer_ref || '—'}</dd>
					{#if order.return_to && place(order)}
						<dt class="text-ink-muted">Returns to</dt>
						<dd class="col-span-2 m-0">{place(order)}</dd>
					{/if}
					{#if order.path_id}
						<dt class="text-ink-muted">Document</dt>
						<dd class="col-span-2 m-0">{humanize(order.path_id)}</dd>
					{/if}
				</dl>
			</section>
		{/if}
	</Sheet.Content>
</Sheet.Root>

<Dialog.Root bind:open={confirmOpen}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title class="text-h3 font-semibold">Refund {total}?</Dialog.Title>
			<Dialog.Description class="text-sm text-ink-muted">
				Stripe refunds order <span class="font-mono">{id}</span> in full and the customer's timeline
				shows it at once. This cannot be undone.
			</Dialog.Description>
		</Dialog.Header>
		<Dialog.Footer>
			<Button variant="outline" size="lg" onclick={() => (confirmOpen = false)}>Keep the payment</Button>
			<Button variant="destructive" size="lg" onclick={confirmRefund}>
				<UndoIcon aria-hidden="true" />
				Refund in full
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
