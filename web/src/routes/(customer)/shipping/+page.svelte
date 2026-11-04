<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { toast } from 'svelte-sonner';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { Button } from '../../../lib/components/ui/button/index.js';
	import StatusBadge from '../../../lib/customer/StatusBadge.svelte';
	import LivePill from '../../../lib/customer/LivePill.svelte';
	import { coalesce, liveTopics } from '../../../lib/customer/live-topics.svelte';
	import { site } from '../../../lib/locale.svelte.js';
	import { SHIPPING_COPY } from '../../../lib/customer/shipping-copy.js';
	import {
		UNPAID,
		carrierName,
		loadDetails,
		loadOrders,
		loginHref,
		safeHref,
		serviceName,
		type Order
	} from '../../../lib/customer/orders';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import FileDownIcon from '@lucide/svelte/icons/file-down';
	import TruckIcon from '@lucide/svelte/icons/truck';

	// W3b: every label and tracking number across the caller's orders,
	// in one place. Labels are bought by the order flow (/order); this
	// page only reads. Live on billing:{customer_ref} and each leg's
	// shipment:{id}.
	const c = $derived(SHIPPING_COPY[site.locale]);
	const kindLabel = (kind: string) => c.kinds[kind as keyof typeof c.kinds] ?? kind;
	const roleLabel = (role: string) => c.roles[role as keyof typeof c.roles] ?? role;

	let orders = $state<Order[] | null>(null);
	let topic = $state<string | null>(null);
	let view = $state<'loading' | 'ready' | 'login' | 'error'>('loading');

	async function load() {
		const list = await loadOrders();
		if (list.kind === 'login') {
			view = 'login';
			return;
		}
		if (list.kind === 'error') {
			if (orders === null) view = 'error';
			return;
		}
		topic = list.topic;
		const shipped = list.orders.filter((o) => !UNPAID.includes(o.status));
		const details = await loadDetails(shipped.map((o) => o.id));
		orders = details.filter((o) => o.legs.length > 0);
		view = 'ready';
	}

	const refresh = coalesce(() => void load());
	const bus = liveTopics(
		() => [...(topic ? [topic] : []), ...(orders ?? []).flatMap((o) => o.legs.map((l) => l.topic))],
		(t, msg) => {
			if (t.startsWith('shipment:') && typeof msg.status === 'string' && orders) {
				const sid = Number(msg.shipment_id);
				orders = orders.map((o) => ({
					...o,
					legs: o.legs.map((l) => (l.shipment_id === sid ? { ...l, status: String(msg.status) } : l))
				}));
			}
			refresh();
		}
	);

	onMount(() => {
		void load();
	});

	const legCount = $derived((orders ?? []).reduce((n, o) => n + o.legs.length, 0));

	async function copy(text: string) {
		try {
			await navigator.clipboard.writeText(text);
			toast.success(c.copied);
		} catch {
			toast.error(c.copyBlocked);
		}
	}
</script>

<svelte:head>
	<title>{c.title} — Apostille Handbook</title>
</svelte:head>

<div class="flex flex-wrap items-end justify-between gap-4">
	<div class="flex flex-col gap-2">
		<Heading level={1} eyebrow={c.eyebrow}>{c.title}</Heading>
	</div>
	<LivePill live={bus.live} lastAt={bus.lastAt} />
</div>

<div class="mt-8">
	{#if view === 'login'}
		<Notice variant="info" heading={c.signInHeading}>
			<p>
				{c.signInBefore}
				<a class="underline underline-offset-4" href={loginHref(resolve('/(customer)/shipping'))}>{c.signIn}</a> {c.signInAfter}
			</p>
		</Notice>
	{:else if view === 'error'}
		<Notice variant="error" heading={c.errorHeading}>
			<p>{c.tryLater}</p>
		</Notice>
	{:else if view === 'loading' || orders === null}
		<div role="status" aria-label={c.loading} class="grid gap-3">
			<span class="sr-only">{c.loading}</span>
			<Skeleton class="h-32 w-full rounded-lg" />
			<Skeleton class="h-32 w-full rounded-lg" />
		</div>
	{:else if orders.length === 0}
		<div class="flex flex-col items-start gap-4 rounded-xl border border-dashed border-line bg-surface p-8">
			<p class="m-0 text-h3 font-semibold text-ink">{c.empty}</p>
			<p class="m-0 text-ink-muted">{c.emptyBody}</p>
			<Button href={resolve('/(customer)/order')} class="h-11 px-5">{c.startOrder}</Button>
		</div>
	{:else}
		<p class="m-0 text-sm text-ink-muted">{c.summary(legCount, orders.length)}</p>
		<ul class="m-0 mt-3 flex list-none flex-col gap-4 p-0">
			{#each orders as o (o.id)}
				<li class="overflow-hidden rounded-xl border border-line bg-bg shadow-sm">
					<div class="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-surface px-4 py-3 sm:px-5">
						<span class="flex min-w-0 flex-col">
							<span class="font-semibold text-ink">{kindLabel(o.kind)} · {o.authority.name}</span>
							<a class="font-mono text-sm underline underline-offset-4" href={resolve('/(customer)/orders/[id]', { id: o.id })}>{o.id}</a>
						</span>
						<StatusBadge status={o.status} />
					</div>
					<ul class="m-0 list-none p-0">
						{#each o.legs as leg (leg.shipment_id)}
							{@const label = safeHref(leg.label_url)}
							{@const track = safeHref(leg.tracking_url)}
							<li class="flex flex-col gap-2 border-b border-line px-4 py-3 last:border-b-0 sm:px-5" data-leg={leg.role}>
								<div class="flex flex-wrap items-center justify-between gap-2">
									<span class="flex items-center gap-2 font-medium text-ink">
										<TruckIcon class="size-4 text-accent-strong" aria-hidden="true" />
										{roleLabel(leg.role)}
										<span class="font-normal text-ink-muted">· {serviceName(leg.service, leg.carrier) || carrierName(leg.carrier)}</span>
									</span>
									<StatusBadge status={leg.status} />
								</div>
								<div class="flex flex-wrap items-center gap-2">
									{#if leg.tracking_number}
										<span class="min-w-0 font-mono text-sm break-all text-ink">{leg.tracking_number}</span>
										<Button
											variant="ghost"
											size="icon"
											class="size-11"
											aria-label={c.copyTracking(roleLabel(leg.role))}
											onclick={() => copy(leg.tracking_number ?? '')}
										>
											<CopyIcon aria-hidden="true" />
										</Button>
									{:else}
										<span class="text-sm text-ink-muted">{c.noTracking}</span>
									{/if}
								</div>
								{#if track || label}
									<div class="flex flex-wrap gap-2">
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
				</li>
			{/each}
		</ul>
	{/if}
</div>
