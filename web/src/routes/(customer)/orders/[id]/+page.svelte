<script lang="ts">
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fly } from 'svelte/transition';
	import { prefersReducedMotion } from 'svelte/motion';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { toast } from 'svelte-sonner';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../../lib/components/brand/Heading.svelte';
	import { Skeleton } from '../../../../lib/components/ui/skeleton/index.js';
	import { Button } from '../../../../lib/components/ui/button/index.js';
	import StatusBadge from '../../../../lib/customer/StatusBadge.svelte';
	import StatusTrack from '../../../../lib/customer/StatusTrack.svelte';
	import LivePill from '../../../../lib/customer/LivePill.svelte';
	import { coalesce, liveTopics } from '../../../../lib/customer/live-topics.svelte';
	import {
		carrierName,
		fmtDate,
		fmtDateTime,
		formFillPdf,
		getJson,
		kindLabel,
		loginHref,
		money,
		postJson,
		roleLabel,
		safeHref,
		serviceName,
		type Leg,
		type Order
	} from '../../../../lib/customer/orders';
	import { ORDERS_COPY } from '../../../../lib/customer/orders-copy.js';
	import { site } from '../../../../lib/locale.svelte.js';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import FileDownIcon from '@lucide/svelte/icons/file-down';
	import ReceiptIcon from '@lucide/svelte/icons/receipt';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import MapPinIcon from '@lucide/svelte/icons/map-pin';
	import TruckIcon from '@lucide/svelte/icons/truck';
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';

	// W3b order detail: status track, legs with tracking and label PDFs,
	// the event timeline, receipt and form fill. Live on the order's
	// billing topic and each leg's shipment topic; events are ids only,
	// so every one triggers a re-read (bursts coalesce). No polling.
	const id = $derived(page.params.id ?? '');
	const c = $derived(ORDERS_COPY[site.locale]);
	const role = (r: string) => roleLabel(r, site.locale);

	let order = $state<Order | null>(null);
	let view = $state<'loading' | 'ready' | 'login' | 'missing' | 'error'>('loading');
	let fresh = $state<Set<string>>(new Set());

	async function load(first = false) {
		const r = await getJson<Order>(`/p2/orders/${encodeURIComponent(id)}`);
		if (r.status === 401) {
			view = 'login';
			return;
		}
		if (r.status === 404) {
			view = 'missing';
			return;
		}
		if (!r.ok || typeof r.body?.id !== 'string') {
			if (first) view = 'error';
			return;
		}
		apply(r.body, first);
	}

	function apply(next: Order, first: boolean) {
		if (!first && order) {
			const known = new Set(order.events.map(eventKey));
			const added = (next.events ?? []).map(eventKey).filter((k) => !known.has(k));
			if (added.length) fresh = new Set(added);
		}
		order = { ...next, legs: next.legs ?? [], events: next.events ?? [], actions: next.actions ?? [] };
		view = 'ready';
	}

	const eventKey = (e: Order['events'][number]) => `${e.at}:${e.kind}:${e.leg ?? ''}`;
	const refresh = coalesce(() => void load());

	const bus = liveTopics(
		() => (order ? [order.topic, ...order.legs.map((l) => l.topic)] : []),
		(topic, msg) => {
			if (!order) return;
			if (topic.startsWith('shipment:') && msg.type === 'tracking.update') {
				// Ids plus the new status: show it at once, then re-read.
				const sid = Number(msg.shipment_id);
				const status = typeof msg.status === 'string' ? msg.status : null;
				if (status) {
					order.legs = order.legs.map((l) => (l.shipment_id === sid ? { ...l, status } : l));
				}
				refresh();
				return;
			}
			if (msg.order_id === undefined || msg.order_id === order.id) refresh();
		}
	);

	onMount(() => {
		void (async () => {
			await load(true);
			if (view !== 'ready' || !order) return;
			if (page.url.searchParams.get('checkout') === 'success') {
				const r = await postJson<Order>(`/p2/orders/${encodeURIComponent(order.id)}/confirm`, {});
				if (r.ok && typeof r.body?.id === 'string') {
					apply(r.body, false);
					toast.success(c.paymentConfirmed);
				}
				const u = new URL(window.location.href);
				u.searchParams.delete('checkout');
				u.searchParams.delete('session_id');
				try {
					await goto(u, { shallow: true, replace: true });
				} catch {
					// The query string is cosmetic.
				}
			}
		})();
	});

	const events = $derived([...(order?.events ?? [])].sort((a, b) => b.at - a.at));
	const legOrder: Leg['role'][] = ['kit_to_customer', 'outbound', 'return'];
	const legs = $derived(
		[...(order?.legs ?? [])].sort((a, b) => legOrder.indexOf(a.role) - legOrder.indexOf(b.role))
	);
	const ms = (n: number) => (prefersReducedMotion.current ? 0 : n);

	async function copy(text: string) {
		try {
			await navigator.clipboard.writeText(text);
			toast.success(c.trackingCopied);
		} catch {
			toast.error(c.copyBlocked);
		}
	}
</script>

<svelte:head>
	<title>{order ? `${kindLabel(order.kind, site.locale)} ${order.id}` : c.order} — Apostille Handbook</title>
</svelte:head>

<nav aria-label={c.breadcrumb} class="mb-4 text-sm">
	<a class="inline-flex min-h-11 items-center gap-1" href={resolve('/(customer)/orders')}>
		<ChevronLeftIcon class="size-4" aria-hidden="true" /> {c.allOrders}
	</a>
</nav>

{#if view === 'loading'}
	<Heading level={1} eyebrow={c.order}>{c.yourOrder}</Heading>
	<div role="status" aria-label={c.loadingOrder} class="mt-6 grid gap-3">
		<span class="sr-only">{c.loadingOrder}</span>
		<Skeleton class="h-6 w-1/2" />
		<Skeleton class="h-16 w-full rounded-lg" />
		<Skeleton class="h-40 w-full rounded-lg" />
	</div>
{:else if view === 'login'}
	<Heading level={1} eyebrow={c.order}>{c.yourOrder}</Heading>
	<div class="mt-6">
		<Notice variant="info" heading={c.signInOrder}>
			<p>
				{c.orderInAccount}
				<a class="underline underline-offset-4" href={loginHref(resolve('/(customer)/orders/[id]', { id }))}>{c.signIn}</a> {c.toContinue}
			</p>
		</Notice>
	</div>
{:else if view === 'missing' || view === 'error' || !order}
	<Heading level={1} eyebrow={c.order}>{c.notFound}</Heading>
	<div class="mt-6">
		<Notice variant={view === 'error' ? 'error' : 'info'} heading={view === 'error' ? c.orderUnavailable : c.noSuchOrder}>
			<p><a class="underline underline-offset-4" href={resolve('/(customer)/orders')}>{c.backToOrders}</a>.</p>
		</Notice>
	</div>
{:else}
	<div class="flex flex-col gap-3">
		<Heading level={1} eyebrow={c.order}>{kindLabel(order.kind, site.locale)}</Heading>
		<p class="m-0 text-body-lg text-ink-muted">
			{c.for}
			<a class="underline underline-offset-4" href={resolve('/(public)/authority/[...trail]', { trail: order.authority.slug })}>{order.authority.name}</a>
			· {c.placed(fmtDate(order.created_at, site.locale))} · <span class="font-mono text-sm">{order.id}</span>
		</p>
		<div class="flex flex-wrap items-center gap-2 text-sm">
			<StatusBadge status={order.status} />
			{#if order.mode === 'test'}
				<span class="inline-flex h-7 items-center rounded-full border border-changed px-3 font-medium text-changed">
					{c.testMode}
				</span>
			{/if}
			<LivePill live={bus.live} lastAt={bus.lastAt} />
		</div>
	</div>

	<div class="mt-6 rounded-xl border border-line bg-bg p-4 shadow-sm sm:p-5">
		<StatusTrack kind={order.kind} status={order.status} />
		{#if order.status === 'needs_staff'}
			<p class="mt-4 mb-0 rounded-md bg-changed-bg px-3 py-2 text-sm text-ink">
				{c.needsStaff}
			</p>
		{:else if order.status === 'refunded'}
			<p class="mt-4 mb-0 rounded-md bg-surface px-3 py-2 text-sm text-ink">
				{c.refunded}
			</p>
		{/if}
		{#if order.actions.length > 0}
			<div class="mt-4 flex flex-wrap gap-2">
				{#each order.actions as a (a.id)}
					<Button href="#shipments" class="h-11 px-5">
						{#if a.id === 'download_labels'}<FileDownIcon aria-hidden="true" />{/if}
						{a.label}
					</Button>
				{/each}
			</div>
		{/if}
	</div>

	<div class="mt-8 grid gap-8 lg:grid-cols-12">
		<div class="flex min-w-0 flex-col gap-8 lg:col-span-7">
			<div id="shipments" class="scroll-mt-20">
				<Heading level={2}>{c.shipments}</Heading>
				{#if legs.length === 0}
					<p class="mt-3 mb-0 rounded-lg border border-dashed border-line bg-surface p-4 text-sm text-ink-muted">
						{c.noLegs}
					</p>
				{:else}
					<ul class="m-0 mt-3 flex list-none flex-col gap-3 p-0">
						{#each legs as leg (leg.shipment_id)}
							{@const label = safeHref(leg.label_url)}
							{@const track = safeHref(leg.tracking_url)}
							<li class="rounded-lg border border-line bg-bg p-4 shadow-sm" data-leg={leg.role}>
								<div class="flex flex-wrap items-center justify-between gap-2">
									<span class="flex items-center gap-2 font-semibold text-ink">
										<TruckIcon class="size-4 text-accent-strong" aria-hidden="true" />
										{role(leg.role)}
									</span>
									<StatusBadge status={leg.status} />
								</div>
								<p class="mt-1 mb-0 text-sm text-ink-muted">{serviceName(leg.service, leg.carrier) || carrierName(leg.carrier)}</p>
								{#if leg.tracking_number}
									<div class="mt-3 flex flex-wrap items-center gap-2">
										<span class="min-w-0 font-mono text-sm break-all text-ink">{leg.tracking_number}</span>
										<Button
											variant="ghost"
											size="icon"
											class="size-11"
											aria-label={c.copyTracking(role(leg.role))}
											onclick={() => copy(leg.tracking_number ?? '')}
										>
											<CopyIcon aria-hidden="true" />
										</Button>
									</div>
								{/if}
								{#if track || label}
									<div class="mt-2 flex flex-wrap gap-2">
										{#if track}
											<Button variant="outline" class="h-11 px-4" href={track} target="_blank" rel="noopener noreferrer">
												{c.trackWith(carrierName(leg.carrier))} <ExternalLinkIcon aria-hidden="true" />
											</Button>
										{/if}
										{#if label}
											<Button variant="outline" class="h-11 px-4" href={label} download>
												<FileDownIcon aria-hidden="true" /> {c.labelPdf}
											</Button>
										{/if}
									</div>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</div>

			<div>
				<Heading level={2}>{c.timeline}</Heading>
				{#if events.length === 0}
					<p class="mt-3 mb-0 text-sm text-ink-muted">{c.noEvents}</p>
				{:else}
					<ol class="relative m-0 mt-4 flex list-none flex-col gap-0 p-0" aria-label={c.timelineLabel}>
						{#each events as e, i (eventKey(e))}
							<li
								class="relative flex gap-4 pb-5 last:pb-0"
								animate:flip={{ duration: ms(250) }}
								in:fly={{ y: -8, duration: ms(250) }}
							>
								{#if i < events.length - 1}
									<span aria-hidden="true" class="absolute top-4 bottom-0 left-1.5 w-px bg-line"></span>
								{/if}
								<span
									aria-hidden="true"
									class={`relative mt-1.5 size-3 flex-none rounded-full border-2 ${
										i === 0 ? 'border-verified bg-verified' : 'border-line bg-bg'
									} ${fresh.has(eventKey(e)) ? 'ring-4 ring-verified/30' : ''}`}
								></span>
								<div class="flex min-w-0 flex-col">
									<span class={`text-ink ${i === 0 ? 'font-semibold' : ''}`}>{e.text}</span>
									<span class="text-sm text-ink-muted">
										{fmtDateTime(e.at, site.locale)}{#if e.leg} · {role(e.leg)}{/if}
									</span>
								</div>
							</li>
						{/each}
					</ol>
				{/if}
			</div>
		</div>

		<div class="flex min-w-0 flex-col gap-4 lg:col-span-5">
			<section class="rounded-xl border border-line bg-bg p-5 shadow-sm" aria-labelledby="receipt-h">
				<h2 id="receipt-h" class="m-0 flex items-center gap-2 text-h3 font-semibold">
					<ReceiptIcon class="size-5 text-accent-strong" aria-hidden="true" /> {c.receipt}
				</h2>
				{#if order.amount && order.amount.lines.length > 0}
					<ul class="m-0 mt-3 list-none p-0 text-sm">
						{#each order.amount.lines as line, i (`${i}-${line.label}`)}
							<li class="flex items-baseline justify-between gap-4 border-b border-line py-2">
								<span class="text-ink">{line.label}</span>
								<span class="font-mono text-ink">{money(line.cents, order.amount.currency, {}, site.locale)}</span>
							</li>
						{/each}
					</ul>
					<p class="m-0 mt-3 flex items-baseline justify-between gap-4">
						<span class="font-semibold">{c.total}</span>
						<span class="font-mono text-h3 font-semibold">{money(order.amount.total_cents, order.amount.currency, {}, site.locale)}</span>
					</p>
				{:else}
					<p class="mt-3 mb-0 text-sm text-ink-muted">{c.nothingToPay}</p>
				{/if}
				{#if order.receipt}
					<p class="mt-3 mb-0 text-sm text-ink-muted">
						{c.paidAt(fmtDateTime(order.receipt.paid_at, site.locale))}{#if order.receipt.demo}{' '}{c.demoNote}{:else if order.mode === 'test'}{' '}{c.testNote}{/if}.
						{#if safeHref(order.receipt.receipt_url)}
							<a class="underline underline-offset-4" href={safeHref(order.receipt.receipt_url)} target="_blank" rel="noopener noreferrer">{c.viewReceipt}</a>
						{/if}
					</p>
				{/if}
			</section>

			{#if order.form_fill_id}
				<section class="rounded-xl border border-line bg-bg p-5 shadow-sm" aria-labelledby="form-h">
					<h2 id="form-h" class="m-0 flex items-center gap-2 text-h3 font-semibold">
						<FileTextIcon class="size-5 text-accent-strong" aria-hidden="true" /> {c.yourForm}
					</h2>
					<p class="mt-2 mb-3 text-sm text-ink-muted">{c.formReady}</p>
					<Button variant="outline" class="h-11 px-4" href={formFillPdf(order.form_fill_id)} download>
						<FileDownIcon aria-hidden="true" /> {c.formPdf}
					</Button>
				</section>
			{/if}

			{#if order.return_to}
				<section class="rounded-xl border border-line bg-bg p-5 shadow-sm" aria-labelledby="return-h">
					<h2 id="return-h" class="m-0 flex items-center gap-2 text-h3 font-semibold">
						<MapPinIcon class="size-5 text-accent-strong" aria-hidden="true" /> {c.returnAddress}
					</h2>
					<address class="mt-2 text-sm text-ink not-italic">
						{order.return_to.name}<br />
						{#if order.return_to.line1}{order.return_to.line1}<br />{/if}
						{order.return_to.city}{order.return_to.region ? `, ${order.return_to.region}` : ''}
						{order.return_to.postal_code}<br />
						{order.return_to.country}
					</address>
				</section>
			{/if}
		</div>
	</div>
{/if}
