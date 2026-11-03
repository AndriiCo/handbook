<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { Button } from '../../../lib/components/ui/button/index.js';
	import StatusBadge from '../../../lib/customer/StatusBadge.svelte';
	import LivePill from '../../../lib/customer/LivePill.svelte';
	import { coalesce, liveTopics } from '../../../lib/customer/live-topics.svelte';
	import {
		KIND_LABEL,
		UNPAID,
		fmtDate,
		loadDetails,
		loadOrders,
		loginHref,
		money,
		safeHref,
		type Order
	} from '../../../lib/customer/orders';
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';
	import ReceiptIcon from '@lucide/svelte/icons/receipt';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';

	// W3b: real receipts, read from the caller's paid orders. Live on
	// billing:{customer_ref} (order.paid / order.refunded re-read).
	let locale = $state<Locale>(resolveLocale());
	const t = $derived(STRINGS[locale]);

	let receipts = $state<Order[] | null>(null);
	let topic = $state<string | null>(null);
	let view = $state<'loading' | 'ready' | 'login' | 'error'>('loading');

	async function load() {
		const list = await loadOrders();
		if (list.kind === 'login') {
			view = 'login';
			return;
		}
		if (list.kind === 'error') {
			if (receipts === null) view = 'error';
			return;
		}
		topic = list.topic;
		const paid = list.orders.filter((o) => o.total_cents > 0 && !UNPAID.includes(o.status));
		const details = await loadDetails(paid.map((o) => o.id));
		receipts = details
			.filter((o) => o.receipt || o.amount)
			.sort((a, b) => (b.receipt?.paid_at ?? b.created_at) - (a.receipt?.paid_at ?? a.created_at));
		view = 'ready';
	}

	const refresh = coalesce(() => void load());
	const bus = liveTopics(
		() => (topic ? [topic] : []),
		() => refresh()
	);

	onMount(() => {
		void load();
	});

	const total = $derived.by(() => {
		const out = new Map<string, number>();
		for (const r of receipts ?? []) {
			if (!r.amount || r.status === 'refunded') continue;
			out.set(r.amount.currency, (out.get(r.amount.currency) ?? 0) + r.amount.total_cents);
		}
		return [...out.entries()];
	});
</script>

<svelte:head>
	<title>{t.billingLabel} — Apostille Handbook</title>
</svelte:head>

<div class="flex flex-wrap items-end justify-between gap-4">
	<div class="flex flex-col gap-2">
		<Heading level={1} eyebrow="Your account">{t.billingLabel}</Heading>
	</div>
	<LivePill live={bus.live} lastAt={bus.lastAt} />
</div>

<div class="mt-8">
	{#if view === 'login'}
		<Notice variant="info" heading="Sign in to see your receipts.">
			<p>
				Receipts live in your account.
				<a class="underline underline-offset-4" href={loginHref(resolve('/(customer)/billing'))}>Sign in</a> to continue.
			</p>
		</Notice>
	{:else if view === 'error'}
		<Notice variant="error" heading="Receipts are unavailable right now.">
			<p>Try again in a bit. Your orders and payments are safe.</p>
		</Notice>
	{:else if view === 'loading' || receipts === null}
		<div role="status" aria-label="Loading receipts" class="grid gap-3">
			<span class="sr-only">Loading receipts</span>
			<Skeleton class="h-28 w-full rounded-lg" />
			<Skeleton class="h-28 w-full rounded-lg" />
		</div>
	{:else if receipts.length === 0}
		<div class="flex flex-col items-start gap-4 rounded-xl border border-dashed border-line bg-surface p-8">
			<p class="m-0 text-h3 font-semibold text-ink">No receipts yet.</p>
			<p class="m-0 text-ink-muted">Paid orders show their receipt here, line by line.</p>
			<Button href={resolve('/(customer)/order')} class="h-11 px-5">Start an order</Button>
		</div>
	{:else}
		<p class="m-0 text-sm text-ink-muted">
			{receipts.length}
			{receipts.length === 1 ? 'receipt' : 'receipts'}{#each total as [cur, cents] (cur)}{' '}·
				{money(cents, cur)} paid{/each}.
		</p>
		<ul class="m-0 mt-3 flex list-none flex-col gap-3 p-0">
			{#each receipts as r (r.id)}
				{@const link = safeHref(r.receipt?.receipt_url)}
				<li class="rounded-xl border border-line bg-bg shadow-sm">
					<div class="flex flex-wrap items-start gap-4 p-4 sm:p-5">
						<span
							aria-hidden="true"
							class="hidden size-11 flex-none items-center justify-center rounded-lg bg-surface-2 text-accent-strong sm:flex"
						>
							<ReceiptIcon class="size-5" />
						</span>
						<div class="flex min-w-0 flex-1 flex-col gap-1">
							<span class="flex flex-wrap items-center gap-x-3 gap-y-1">
								<span class="font-semibold text-ink">{KIND_LABEL[r.kind] ?? r.kind}</span>
								{#if r.status === 'refunded'}<StatusBadge status={r.status} />{/if}
								{#if r.mode === 'test'}<span class="text-xs font-medium text-changed">Test mode</span>{/if}
							</span>
							<span class="truncate text-sm text-ink-muted">{r.authority.name}</span>
							<span class="text-sm text-ink-muted">
								{r.receipt ? `Paid ${fmtDate(r.receipt.paid_at)}` : `Ordered ${fmtDate(r.created_at)}`}
								· <a class="font-mono underline underline-offset-4" href={resolve('/(customer)/orders/[id]', { id: r.id })}>{r.id}</a>
							</span>
						</div>
						{#if r.amount}
							<span class="font-mono text-h3 font-semibold text-ink">{money(r.amount.total_cents, r.amount.currency)}</span>
						{/if}
					</div>
					{#if r.amount && r.amount.lines.length > 0}
						<ul class="m-0 list-none border-t border-line px-4 py-2 text-sm sm:px-5">
							{#each r.amount.lines as line, i (`${i}-${line.label}`)}
								<li class="flex items-baseline justify-between gap-4 py-1">
									<span class="text-ink-muted">{line.label}</span>
									<span class="font-mono text-ink">{money(line.cents, r.amount.currency)}</span>
								</li>
							{/each}
						</ul>
					{/if}
					{#if link}
						<div class="border-t border-line px-4 py-2 sm:px-5">
							<a class="inline-flex min-h-11 items-center gap-1 text-sm" href={link} target="_blank" rel="noopener noreferrer">
								View receipt <ExternalLinkIcon class="size-3.5" aria-hidden="true" />
							</a>
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>
