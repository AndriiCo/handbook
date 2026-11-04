<script lang="ts">
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { prefersReducedMotion } from 'svelte/motion';
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { Button } from '../../../lib/components/ui/button/index.js';
	import StatusBadge from '../../../lib/customer/StatusBadge.svelte';
	import LivePill from '../../../lib/customer/LivePill.svelte';
	import { coalesce, liveTopics } from '../../../lib/customer/live-topics.svelte';
	import {
		fmtDate,
		kindLabel,
		loadOrders,
		loginHref,
		money,
		type OrderSummary
	} from '../../../lib/customer/orders';
	import { ORDERS_COPY } from '../../../lib/customer/orders-copy.js';
	import { site } from '../../../lib/locale.svelte.js';
	import PrinterIcon from '@lucide/svelte/icons/printer';
	import StampIcon from '@lucide/svelte/icons/stamp';
	import PackageIcon from '@lucide/svelte/icons/package';
	import TagIcon from '@lucide/svelte/icons/tag';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import PlusIcon from '@lucide/svelte/icons/plus';

	// W3b cabinet: the caller's orders, newest first, live on
	// billing:{customer_ref} (ids-only events → one re-read).
	const ICONS = { label_fedex: PrinterIcon, label_usps: StampIcon, mail_kit: PackageIcon, address_label: TagIcon };
	const c = $derived(ORDERS_COPY[site.locale]);

	let rows = $state<OrderSummary[] | null>(null);
	let topic = $state<string | null>(null);
	let loadError = $state(false);
	let needsLogin = $state(false);

	async function load() {
		const list = await loadOrders();
		if (list.kind === 'login') {
			needsLogin = true;
			rows = [];
			return;
		}
		if (list.kind === 'error') {
			if (rows === null) {
				loadError = true;
				rows = [];
			}
			return;
		}
		loadError = false;
		rows = list.orders;
		topic = list.topic;
	}

	const refresh = coalesce(() => void load());
	const bus = liveTopics(
		() => (topic ? [topic] : []),
		() => refresh()
	);

	onMount(() => {
		void load();
	});

	const ms = (n: number) => (prefersReducedMotion.current ? 0 : n);
</script>

<svelte:head>
	<title>{c.orders} — Apostille Handbook</title>
</svelte:head>

<div class="flex flex-wrap items-end justify-between gap-4">
	<div class="flex flex-col gap-2">
		<Heading level={1} eyebrow={c.eyebrow}>{c.orders}</Heading>
	</div>
	<div class="flex items-center gap-3">
		<LivePill live={bus.live} lastAt={bus.lastAt} />
		<Button href={resolve('/(customer)/order')} class="h-11 px-5">
			<PlusIcon aria-hidden="true" /> {c.newOrder}
		</Button>
	</div>
</div>

<div class="mt-8">
	{#if needsLogin}
		<Notice variant="info" heading={c.signInOrders}>
			<p>
				{c.ordersInAccount}
				<a class="underline underline-offset-4" href={loginHref(resolve('/(customer)/orders'))}>{c.signIn}</a>
				{c.or} <a class="underline underline-offset-4" href={resolve('/(customer)/order')}>{c.startNew}</a>.
			</p>
		</Notice>
	{:else if loadError}
		<Notice variant="error" heading={c.listUnavailable}><p>{c.listError}</p></Notice>
	{:else if rows === null}
		<div role="status" aria-label={c.loadingOrders} class="grid gap-3">
			<span class="sr-only">{c.loadingOrders}</span>
			<Skeleton class="h-24 w-full rounded-lg" />
			<Skeleton class="h-24 w-full rounded-lg" />
		</div>
	{:else if rows.length === 0}
		<div class="flex flex-col items-start gap-4 rounded-xl border border-dashed border-line bg-surface p-8">
			<p class="m-0 text-h3 font-semibold text-ink">{c.emptyTitle}</p>
			<p class="m-0 text-ink-muted">{c.emptyBody}</p>
			<Button href={resolve('/(customer)/order')} class="h-11 px-5">{c.startOrder}</Button>
		</div>
	{:else}
		<p class="m-0 text-sm text-ink-muted">{c.count(rows.length)}</p>
		<ul class="m-0 mt-3 flex list-none flex-col gap-3 p-0" aria-label={c.listLabel}>
			{#each rows as row (row.id)}
				{@const Icon = ICONS[row.kind] ?? TagIcon}
				<li animate:flip={{ duration: ms(250) }} in:fade={{ duration: ms(200) }}>
					<a
						href={resolve('/(customer)/orders/[id]', { id: row.id })}
						class="flex items-center gap-4 rounded-xl border border-line bg-bg p-4 text-ink no-underline shadow-sm transition-[box-shadow,border-color] duration-200 hover:border-brand-300 hover:shadow-md sm:p-5"
						data-order={row.id}
					>
						<span
							aria-hidden="true"
							class="hidden size-12 flex-none items-center justify-center rounded-lg bg-surface-2 text-accent-strong sm:flex"
						>
							<Icon class="size-6" />
						</span>
						<span class="flex min-w-0 flex-1 flex-col gap-1">
							<span class="flex flex-wrap items-center gap-x-3 gap-y-1">
								<span class="font-semibold">{kindLabel(row.kind, site.locale)}</span>
								<StatusBadge status={row.status} />
								{#if row.mode === 'test'}
									<span class="text-xs font-medium text-changed">{c.testMode}</span>
								{/if}
							</span>
							<span class="truncate text-sm text-ink-muted">{row.authority.name}</span>
							{#if row.next}
								<span class="text-sm font-medium text-accent-strong">{row.next}</span>
							{/if}
						</span>
						<span class="flex flex-none flex-col items-end gap-1 text-right">
							<span class="font-mono font-semibold">
								{row.total_cents === 0 ? c.free : money(row.total_cents, row.currency, {}, site.locale)}
							</span>
							<span class="text-xs text-ink-muted">{fmtDate(row.created_at, site.locale)}</span>
						</span>
						<ChevronRightIcon
							class="size-5 flex-none text-ink-muted"
							aria-hidden="true"
						/>
					</a>
				</li>
			{/each}
		</ul>
	{/if}
</div>
