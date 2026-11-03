<script lang="ts">
	import { onMount } from 'svelte';
	import { fly } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { toast } from 'svelte-sonner';
	import { resolve } from '$app/paths';
	import Heading from '#lib/components/brand/Heading.svelte';
	import Notice from '#lib/public-ui/Notice.svelte';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import OrderStatusBadge from '#lib/admin/OrderStatusBadge.svelte';
	import OrderSheet from '#lib/admin/OrderSheet.svelte';
	import { getJson, postJson, errorText, ApiError } from '#lib/admin/admin-api.js';
	import { fetchAuthorities } from '#lib/admin/authorities.js';
	import { ago, dateTime, fmtCents } from '#lib/admin/format.js';
	import { moveText } from '#lib/admin/order-moves.js';
	import {
		TABS,
		countTabs,
		inTab,
		normalizeOrder,
		normalizeRows,
		orderEventId,
		staffStatusLabel,
		tabFromParam,
		type AdminOrder,
		type AdminOrderRow,
		type OrdersTab
	} from '#lib/admin/orders.js';
	import { KIND_LABEL, humanize } from '#lib/customer/orders.js';
	import { subscribe } from '#lib/live.svelte.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import PackageSearchIcon from '@lucide/svelte/icons/package-search';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

	// Staff Orders console (MISSION WIN W1c). One read of
	// `GET /admin/orders?status=all` feeds every tab, so the counts
	// stay exact (the server counts only the rows it returns). The
	// staff-only `ops` topic carries `order.*` ids: each one re-reads
	// the list (debounced) and the open order. No polling.
	// Route note: `/orders` is the customer cabinet in this same
	// bundle, so the console lives at `/staff-orders` (as /staff-login).
	let rows = $state<AdminOrderRow[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let refreshing = $state(false);
	let tab = $state<OrdersTab>('all');
	let names = $state<Record<string, string>>({});
	let liveOn = $state(false);
	let flash = $state<Record<string, true>>({});
	let nowSec = $state(Math.floor(Date.now() / 1000));
	let reduceMotion = $state(false);

	let selectedId = $state<string | null>(null);
	let sheetOpen = $state(false);
	let detail = $state<AdminOrder | null>(null);
	let detailState = $state<'loading' | 'ready' | 'failed'>('loading');
	let detailError = $state<string | null>(null);
	let busy = $state<string | null>(null);

	let listTimer: ReturnType<typeof setTimeout> | null = null;
	let detailTimer: ReturnType<typeof setTimeout> | null = null;
	const flashTimers = new Map<string, ReturnType<typeof setTimeout>>();

	const counts = $derived(countTabs(rows));
	const visible = $derived(rows.filter((r) => inTab(r, tab)));
	const selectedRow = $derived(rows.find((r) => r.id === selectedId) ?? null);
	const motion = $derived(reduceMotion ? 0 : 200);
	// Ages read against the newest order when this browser's clock is
	// behind the server's (a skewed laptop never shows "in 3 min").
	const now = $derived(Math.max(nowSec, ...rows.map((r) => r.created_at)));
	const allTest = $derived(rows.length > 0 && rows.every((r) => r.mode === 'test'));

	const authorityName = (slug: string) => names[slug] ?? slug;
	// Amber while a live event just touched the row, muted while open.
	const rowTone = (id: string) => (flash[id] ? 'bg-changed-bg' : selectedId === id ? 'bg-muted' : '');
	const kindLabel = (kind: string) => (KIND_LABEL as Record<string, string>)[kind] ?? humanize(kind);

	async function load(quiet = false) {
		if (quiet) refreshing = true;
		else {
			loadState = 'loading';
			loadError = null;
		}
		try {
			rows = normalizeRows(await getJson('/admin/orders?status=all', 'Orders'));
			loadState = 'ready';
			loadError = null;
		} catch (err) {
			loadError = errorText(err, 'Orders failed to load.');
			if (!quiet || loadState !== 'ready') loadState = 'failed';
		} finally {
			refreshing = false;
		}
	}

	async function loadDetail(id: string, quiet = false) {
		if (!quiet) {
			detailState = 'loading';
			detailError = null;
		}
		try {
			const order = normalizeOrder(await getJson(`/admin/orders/${encodeURIComponent(id)}`, 'Order'));
			if (selectedId !== id) return;
			if (!order) throw new ApiError('The order answer was empty.', 0);
			detail = order;
			detailState = 'ready';
			patchRow(id, { status: order.status });
		} catch (err) {
			if (selectedId !== id) return;
			detailError = errorText(err, 'The order failed to load.');
			if (!quiet || !detail) detailState = 'failed';
		}
	}

	function patchRow(id: string, patch: Partial<AdminOrderRow>) {
		rows = rows.map((r) =>
			r.id === id ? { ...r, ...patch, needs_staff: (patch.status ?? r.status) === 'needs_staff' } : r
		);
	}

	function syncUrl() {
		const params = new URLSearchParams(location.search);
		if (tab !== 'all') params.set('status', tab);
		else params.delete('status');
		if (selectedId) params.set('order', selectedId);
		else params.delete('order');
		const query = params.toString();
		history.replaceState(history.state, '', query ? `?${query}` : location.pathname);
	}

	function setTab(value: string) {
		tab = tabFromParam(value);
		syncUrl();
	}

	function openOrder(id: string) {
		if (selectedId !== id) {
			detail = null;
			selectedId = id;
			void loadDetail(id);
		}
		sheetOpen = true;
		syncUrl();
	}

	function closed() {
		selectedId = null;
		detail = null;
		busy = null;
		syncUrl();
	}

	function markFlash(id: string) {
		flash = { ...flash, [id]: true };
		clearTimeout(flashTimers.get(id));
		flashTimers.set(
			id,
			setTimeout(() => {
				const next = { ...flash };
				delete next[id];
				flash = next;
				flashTimers.delete(id);
			}, 2400)
		);
	}

	function onOps(msg: { type?: unknown; order_id?: unknown }) {
		liveOn = true;
		const id = orderEventId(msg);
		if (!id) return;
		markFlash(id);
		if (loadState === 'ready') {
			if (listTimer !== null) clearTimeout(listTimer);
			listTimer = setTimeout(() => {
				listTimer = null;
				void load(true);
			}, 800);
		}
		if (id === selectedId && busy === null) {
			if (detailTimer !== null) clearTimeout(detailTimer);
			detailTimer = setTimeout(() => {
				detailTimer = null;
				if (selectedId === id) void loadDetail(id, true);
			}, 800);
		}
	}

	async function advance(to: string) {
		if (!detail || busy !== null) return;
		const id = detail.id;
		const before = detail;
		const prevStatus = selectedRow?.status ?? before.status;
		busy = to;
		detail = {
			...before,
			status: to,
			events: [...before.events, { at: Math.floor(Date.now() / 1000), kind: `order.${to}`, text: moveText(to), leg: null, pending: true }]
		};
		patchRow(id, { status: to });
		try {
			await postJson(`/admin/orders/${encodeURIComponent(id)}/advance`, { to }, 'Order update');
			toast.success(`${id}: ${staffStatusLabel(to)}. The customer's timeline updates live.`);
			markFlash(id);
			void loadDetail(id, true);
		} catch (err) {
			if (selectedId === id) detail = before;
			patchRow(id, { status: prevStatus });
			toast.error(errorText(err, 'The order could not be updated.'));
			if (err instanceof ApiError && err.status === 409) {
				void load(true);
				void loadDetail(id, true);
			}
		} finally {
			busy = null;
		}
	}

	async function refund() {
		if (!detail || busy !== null) return;
		const id = detail.id;
		const before = detail;
		const prevStatus = selectedRow?.status ?? before.status;
		busy = 'refund';
		detail = {
			...before,
			status: 'refunded',
			receipt: before.receipt ? { ...before.receipt, refunded: true } : null,
			events: [...before.events, { at: Math.floor(Date.now() / 1000), kind: 'order.refunded', text: 'Refunded in full', leg: null, pending: true }]
		};
		patchRow(id, { status: 'refunded' });
		try {
			await postJson(`/admin/orders/${encodeURIComponent(id)}/refund`, {}, 'Refund');
			toast.success(`${id} refunded in full through Stripe.`);
			markFlash(id);
			void loadDetail(id, true);
		} catch (err) {
			if (selectedId === id) detail = before;
			patchRow(id, { status: prevStatus });
			const msg =
				err instanceof ApiError && err.status === 503
					? 'Refunds need Stripe: payments are not configured on this host.'
					: errorText(err, 'The refund did not go through.');
			toast.error(msg);
			if (err instanceof ApiError && err.status === 409) void loadDetail(id, true);
		} finally {
			busy = null;
		}
	}

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		const params = new URLSearchParams(location.search);
		tab = tabFromParam(params.get('status'));
		const wanted = params.get('order');
		void load().then(() => {
			if (wanted) openOrder(wanted);
		});
		fetchAuthorities()
			.then((list) => (names = Object.fromEntries(list.map((a) => [a.slug, a.name]))))
			.catch(() => {
				// Names are a nicety: the table falls back to slugs.
			});
		const unsub = subscribe('ops', onOps);
		const tick = setInterval(() => (nowSec = Math.floor(Date.now() / 1000)), 30_000);
		return () => {
			unsub();
			clearInterval(tick);
			if (listTimer !== null) clearTimeout(listTimer);
			if (detailTimer !== null) clearTimeout(detailTimer);
			for (const t of flashTimers.values()) clearTimeout(t);
		};
	});

	// Inactive triggers in full muted ink (the primitive's 60% ink
	// misses 4.5:1 on the muted track), as on /review.
	const TRIGGER = 'h-8 gap-1.5 px-2 text-ink-muted data-active:text-foreground sm:px-3';
	const countCls = (id: OrdersTab, n: number) =>
		id === 'needs_staff' && n > 0
			? 'min-w-5 rounded-full bg-changed-bg px-1.5 text-xs font-semibold text-changed tabular-nums'
			: 'min-w-5 rounded-full bg-surface-2 px-1.5 text-xs font-semibold text-ink-muted tabular-nums';
</script>

<svelte:head>
	<title>Orders — Admin — Apostille Handbook</title>
</svelte:head>

<div class="mt-6 flex flex-col gap-6 pb-12">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<Heading level={1} eyebrow="Commerce">Orders</Heading>
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Every customer order, newest first. Staff moves reach the customer's timeline live.
			</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			{#if allTest}
				<span class="inline-flex h-8 items-center rounded-full border border-dashed border-line px-3 font-mono text-xs text-ink-muted">
					test mode
				</span>
			{/if}
			<span
				class="inline-flex h-8 items-center gap-2 rounded-full border border-line px-3 text-sm text-ink-muted"
				role="status"
				aria-live="polite"
			>
				<span
					class={`size-2 rounded-full ${liveOn ? 'bg-verified motion-safe:animate-pulse' : 'bg-ink-muted'}`}
					aria-hidden="true"
				></span>
				{liveOn ? 'Live' : 'Connecting'}
			</span>
			<Button variant="outline" size="lg" onclick={() => load(true)} disabled={refreshing || loadState === 'loading'}>
				<RefreshCwIcon class={refreshing ? 'motion-safe:animate-spin' : ''} aria-hidden="true" />
				Refresh
			</Button>
		</div>
	</header>

	<Tabs.Root value={tab} onValueChange={setTab} class="flex-col gap-4">
		<div class="-mx-4 overflow-x-auto px-4 md:mx-0 md:px-0">
			<Tabs.List aria-label="Filter orders">
				{#each TABS as t (t.id)}
					<Tabs.Trigger value={t.id} class={TRIGGER}>
						{t.label}
						{#if loadState === 'ready'}<span class={countCls(t.id, counts[t.id])}>{counts[t.id]}</span>{/if}
					</Tabs.Trigger>
				{/each}
			</Tabs.List>
		</div>

		{#each TABS as t (t.id)}
			<Tabs.Content value={t.id}>
				{#if tab === t.id}
					{@render list()}
				{/if}
			</Tabs.Content>
		{/each}
	</Tabs.Root>
</div>

{#snippet list()}
	{#if loadState === 'loading'}
		<div class="overflow-hidden rounded-lg border border-line bg-card shadow-sm" role="status" aria-label="Loading orders">
			<div class="flex flex-col divide-y divide-line">
				{#each [0, 1, 2, 3, 4, 5] as i (i)}
					<div class="flex items-center gap-4 px-4 py-3">
						<Skeleton class="h-4 w-32 rounded-xs" />
						<Skeleton class="hidden h-4 flex-1 rounded-xs md:block" />
						<Skeleton class="h-6 w-24 rounded-full" />
						<Skeleton class="h-4 w-14 rounded-xs" />
					</div>
				{/each}
			</div>
		</div>
	{:else if loadState === 'failed'}
		<Notice variant="error" heading="Orders could not load.">
			<p class="m-0">{loadError}</p>
			<p class="mt-3 mb-0"><Button onclick={() => load()}>Try again</Button></p>
		</Notice>
	{:else if visible.length === 0}
		<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line bg-card px-6 py-12 text-center">
			{#if rows.length === 0}
				<InboxIcon class="size-8 text-ink-muted" aria-hidden="true" />
				<p class="m-0 text-h3 font-semibold">No orders yet</p>
				<p class="m-0 max-w-prose text-sm text-ink-muted">
					Orders appear here the moment a customer places one, and every payment and shipment
					event lands live.
				</p>
				<Button variant="outline" href={resolve('/(admin)/dashboard')}>Open the dashboard</Button>
			{:else}
				<PackageSearchIcon class="size-8 text-ink-muted" aria-hidden="true" />
				<p class="m-0 text-h3 font-semibold">
					{tab === 'needs_staff' ? 'Nothing needs staff' : `No orders ${tab === 'to_pack' ? 'to pack' : 'shipped'}`}
				</p>
				<p class="m-0 max-w-prose text-sm text-ink-muted">
					{counts.all} order{counts.all === 1 ? '' : 's'} in other states.
				</p>
				<Button variant="outline" onclick={() => setTab('all')}>Show all orders</Button>
			{/if}
		</div>
	{:else}
		<div class="overflow-hidden rounded-lg border border-line bg-card shadow-sm">
			<Table.Root aria-label="Orders">
				<Table.Header class="bg-surface">
					<tr class="border-b border-line">
						<Table.Head class="px-3 text-xs text-ink-muted sm:px-4">Order</Table.Head>
						<Table.Head class="hidden text-xs text-ink-muted lg:table-cell">Authority</Table.Head>
						<Table.Head class="hidden text-xs text-ink-muted md:table-cell">Customer</Table.Head>
						<Table.Head class="text-xs text-ink-muted">Status</Table.Head>
						<Table.Head class="pr-3 text-right text-xs text-ink-muted sm:pr-2">Total</Table.Head>
						<Table.Head class="hidden pr-4 text-right text-xs text-ink-muted sm:table-cell">Placed</Table.Head>
					</tr>
				</Table.Header>
				<Table.Body>
					{#each visible as row (row.id)}
						<!-- Mouse convenience only: the order id is the keyboard and screen-reader control. -->
						<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
						<tr
							class={`cursor-pointer border-b border-line transition-colors duration-300 last:border-b-0 hover:bg-muted/50 has-focus-visible:bg-muted/50 ${rowTone(row.id)}`}
							data-order={row.id}
							onclick={() => openOrder(row.id)}
							in:fly={{ y: -8, duration: motion }}
							animate:flip={{ duration: motion }}
						>
							<Table.Cell class="px-3 py-2.5 sm:px-4">
								<div class="flex min-w-0 flex-col gap-0.5">
									<button
										type="button"
										class="w-fit rounded-xs text-left font-mono text-sm font-medium text-ink outline-none hover:underline focus-visible:ring-2 focus-visible:ring-focus"
										aria-haspopup="dialog"
										onclick={(e) => {
											e.stopPropagation();
											openOrder(row.id);
										}}
									>
										{row.id}
									</button>
									<span class="flex gap-1.5 text-xs text-ink-muted">
										<span>{kindLabel(row.kind)}</span>
										<span class="hidden sm:inline lg:hidden" aria-hidden="true">·</span>
										<span class="hidden sm:inline lg:hidden">{row.authority_slug}</span>
									</span>
									{#if row.status === 'needs_staff' && row.reason}
										<span class="hidden max-w-sm items-center gap-1 truncate text-xs text-changed md:flex" title={row.reason}>
											<TriangleAlertIcon class="size-3 flex-none" aria-hidden="true" />
											<span class="truncate">{row.reason}</span>
										</span>
									{/if}
								</div>
							</Table.Cell>
							<Table.Cell class="hidden max-w-xs lg:table-cell">
								<span class="block truncate text-sm" title={authorityName(row.authority_slug)}>{authorityName(row.authority_slug)}</span>
							</Table.Cell>
							<Table.Cell class="hidden font-mono text-xs text-ink-muted md:table-cell">{row.customer}</Table.Cell>
							<Table.Cell><OrderStatusBadge status={row.status} /></Table.Cell>
							<Table.Cell class="pr-3 text-right font-mono text-sm tabular-nums sm:pr-2">
								{#if row.total_cents === 0}<span class="text-ink-muted">Free</span>{:else}{fmtCents(row.total_cents, row.currency)}{/if}
							</Table.Cell>
							<Table.Cell class="hidden pr-4 text-right text-xs text-ink-muted sm:table-cell">
								<time datetime={new Date(row.created_at * 1000).toISOString()} title={dateTime(row.created_at)}>{ago(row.created_at, now)}</time>
							</Table.Cell>
						</tr>
					{/each}
				</Table.Body>
			</Table.Root>
		</div>
		<p class="m-0 mt-3 text-xs text-ink-muted">
			{visible.length} of {counts.all} order{counts.all === 1 ? '' : 's'} · select an order to open it.
			{#if loadError}<span role="status"> Last refresh failed: {loadError}</span>{/if}
		</p>
	{/if}
{/snippet}

<OrderSheet
	bind:open={sheetOpen}
	order={detail}
	row={selectedRow}
	loadState={detailState}
	error={detailError}
	{busy}
	authorityName={detail?.authority?.name || authorityName(selectedRow?.authority_slug ?? '')}
	{motion}
	onAdvance={advance}
	onRefund={refund}
	onRetry={() => selectedId && loadDetail(selectedId)}
	onClosed={closed}
/>
