<script lang="ts">
	import { onMount } from 'svelte';
	import { fly } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { resolve } from '$app/paths';
	import Heading from '#lib/components/brand/Heading.svelte';
	import Notice from '#lib/public-ui/Notice.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import BarChart from '#lib/admin/BarChart.svelte';
	import ComposePanel from '#lib/admin/ComposePanel.svelte';
	import type { ComposeKind } from '#lib/admin/compose.svelte.js';
	import { getJson, errorText } from '#lib/admin/admin-api.js';
	import {
		activityFromOps,
		isEmpty,
		mergeActivity,
		normalizeStats,
		type Activity,
		type AdminStats
	} from '#lib/admin/stats.js';
	import {
		ago,
		clock,
		dayLabel,
		fmtCents,
		fmtCompact,
		fmtInt,
		fmtMs,
		fmtPct,
		hourLabel,
		modelName,
		dateTime
	} from '#lib/admin/format.js';
	import { subscribe } from '#lib/live.svelte.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import CreditCardIcon from '@lucide/svelte/icons/credit-card';
	import PackageIcon from '@lucide/svelte/icons/package';
	import FileCheckIcon from '@lucide/svelte/icons/file-check';
	import CpuIcon from '@lucide/svelte/icons/cpu';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import ArrowUpRightIcon from '@lucide/svelte/icons/arrow-up-right';
	import InboxIcon from '@lucide/svelte/icons/inbox';

	// Staff dashboard (MISSION WIN W1a): KPIs, Meta model telemetry,
	// two zero-dependency charts and an activity feed that updates live
	// from the staff-only `ops` topic (ids only: each event re-reads
	// /admin/stats, debounced).
	let stats = $state<AdminStats | null>(null);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let refreshing = $state(false);
	let live = $state<Activity[]>([]);
	let liveOn = $state(false);
	let nowSec = $state(Math.floor(Date.now() / 1000));
	let reduceMotion = $state(false);
	let refetchTimer: ReturnType<typeof setTimeout> | null = null;
	let composeKind = $state<ComposeKind>('plans');
	let composeBusy = $state(false);

	async function load(quiet = false) {
		if (quiet) refreshing = true;
		else {
			loadState = 'loading';
			loadError = null;
		}
		try {
			stats = normalizeStats(await getJson('/admin/stats', 'Dashboard stats'));
			loadState = 'ready';
			loadError = null;
		} catch (err) {
			loadError = errorText(err, 'Dashboard stats failed to load.');
			if (!quiet || !stats) loadState = 'failed';
		} finally {
			refreshing = false;
		}
	}

	function onOps(msg: { type?: unknown }) {
		liveOn = true;
		if (msg?.type === 'subscribed') return;
		const row = activityFromOps(msg);
		if (!row) return;
		live = [row, ...live].slice(0, 20);
		if (refetchTimer !== null) clearTimeout(refetchTimer);
		refetchTimer = setTimeout(() => {
			refetchTimer = null;
			void load(true);
		}, 1500);
	}

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		void load();
		const unsub = subscribe('ops', onOps);
		const tick = setInterval(() => (nowSec = Math.floor(Date.now() / 1000)), 30_000);
		return () => {
			unsub();
			clearInterval(tick);
			if (refetchTimer !== null) clearTimeout(refetchTimer);
		};
	});

	// Relative times read against the server snapshot when it is ahead
	// of this browser's clock (skewed staff laptops never show "-3 min").
	const now = $derived(Math.max(nowSec, stats?.generated_at ?? 0));
	const feed = $derived(stats ? mergeActivity(stats.activity, live) : []);
	const motion = $derived(reduceMotion ? 0 : 200);

	const hourBars = $derived(
		(stats?.series.calls_by_hour ?? []).map((p) => ({
			key: p.t,
			label: hourLabel(p.t),
			value: p.n
		}))
	);
	const dayBars = $derived(
		(stats?.series.orders_by_day ?? []).map((p) => ({
			key: p.t,
			label: dayLabel(p.t),
			value: p.n,
			detail: fmtCents(p.cents ?? 0)
		}))
	);
	const pipelineMax = $derived(Math.max(1, ...(stats?.meta.by_pipeline ?? []).map((p) => p.calls)));

	interface Tile {
		label: string;
		value: string;
		hint: string;
		href?: string;
	}

	const tiles = $derived<Tile[]>(
		stats
			? [
					{
						label: 'Proposals open',
						value: fmtInt(stats.kpis.proposals_open),
						hint: 'Facts, plans, offices, forms',
						href: resolve('/(admin)/review')
					},
					{
						label: 'Facts accepted',
						value: fmtInt(stats.kpis.facts_accepted),
						hint: `Across ${fmtInt(stats.kpis.authorities)} authorities`
					},
					{
						label: 'Page plans live',
						value: fmtInt(stats.kpis.plans_accepted),
						hint: 'Accepted plan versions',
						href: `${resolve('/(admin)/review')}?tab=plans`
					},
					{
						label: 'Offices verified',
						value: fmtInt(stats.kpis.offices_accepted),
						hint: 'Addresses, hours, mail',
						href: `${resolve('/(admin)/review')}?tab=offices`
					},
					{
						label: 'Forms active',
						value: fmtInt(stats.kpis.forms_active),
						hint: 'Mapped fillable PDFs',
						href: `${resolve('/(admin)/review')}?tab=forms`
					},
					{
						label: 'Orders today',
						value: fmtInt(stats.kpis.orders_today),
						hint: 'Pacific day'
					},
					{
						label: 'Paid, 7 days',
						value: fmtCents(stats.kpis.paid_cents_7d),
						hint: 'Captured payments'
					},
					{
						label: 'Model calls, 24 h',
						value: fmtInt(stats.meta.calls_24h),
						hint: `${fmtInt(stats.meta.errors_24h)} errors`,
						href: resolve('/(admin)/trace')
					}
				]
			: []
	);

	function cacheShare(m: { input_tokens: number; cached_tokens: number }): number {
		return m.input_tokens > 0 ? m.cached_tokens / m.input_tokens : 0;
	}

	const kindIcon = (kind: string) => {
		if (kind.startsWith('order.paid')) return CreditCardIcon;
		if (kind === 'kit.packed' || kind === 'order.shipped') return PackageIcon;
		if (kind === 'plan_accepted' || kind === 'fact_accepted' || kind === 'offices_accepted')
			return FileCheckIcon;
		if (kind === 'compile.done') return CpuIcon;
		if (kind === 'model_call.error' || kind === 'order.needs_staff') return TriangleAlertIcon;
		return ActivityIcon;
	};
	const kindTone = (kind: string) =>
		kind === 'model_call.error' || kind === 'order.needs_staff'
			? 'bg-warning-bg text-warning'
			: kind === 'order.paid' || kind.endsWith('_accepted')
				? 'bg-success-bg text-success'
				: 'bg-surface-2 text-ink-muted';
</script>

<svelte:head>
	<title>Dashboard — Admin — Apostille Handbook</title>
</svelte:head>

<div class="mt-6 flex flex-col gap-8 pb-12">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<Heading level={1} eyebrow="Staff console">Dashboard</Heading>
			<p class="m-0 text-sm text-ink-muted">
				{#if stats}
					Snapshot {clock(stats.generated_at)} · Meta models, review queue and orders at a glance.
				{:else}
					Meta models, review queue and orders at a glance.
				{/if}
			</p>
		</div>
		<div class="flex items-center gap-2">
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

	{#if loadState === 'loading'}
		<div role="status" aria-label="Loading dashboard" class="flex flex-col gap-6">
			<span class="sr-only">Loading dashboard</span>
			<div class="grid grid-cols-2 gap-3 md:grid-cols-4">
				{#each [0, 1, 2, 3, 4, 5, 6, 7] as i (i)}
					<Skeleton class="h-24 rounded-md" />
				{/each}
			</div>
			<div class="grid gap-3 md:grid-cols-3">
				{#each [0, 1, 2] as i (i)}
					<Skeleton class="h-40 rounded-md" />
				{/each}
			</div>
			<div class="grid gap-3 lg:grid-cols-2">
				<Skeleton class="h-56 rounded-md" />
				<Skeleton class="h-56 rounded-md" />
			</div>
		</div>
	{:else if loadState === 'failed'}
		<Notice variant="error" heading="The dashboard could not load.">
			<p class="m-0">{loadError}</p>
			<p class="mt-3 mb-0"><Button onclick={() => load()}>Try again</Button></p>
		</Notice>
	{:else if stats && isEmpty(stats)}
		<Card.Root class="items-center px-6 py-12 text-center">
			<InboxIcon class="size-8 text-ink-muted" aria-hidden="true" />
			<p class="m-0 text-h3 font-semibold">Nothing to report yet</p>
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Fetch an official source and let Muse Spark draft from it. Calls, proposals and orders
				show up here as they happen.
			</p>
			<Button href={resolve('/(admin)/workers')}>Fetch a source</Button>
		</Card.Root>
	{:else if stats}
		{#if stats.kpis.needs_staff > 0}
			<div
				class="flex flex-wrap items-center gap-3 rounded-md border border-warning bg-warning-bg px-4 py-3"
				role="status"
			>
				<TriangleAlertIcon class="size-5 flex-none text-warning" aria-hidden="true" />
				<p class="m-0 min-w-0 flex-1 text-sm font-semibold">
					{stats.kpis.needs_staff} order{stats.kpis.needs_staff === 1 ? '' : 's'} need{stats.kpis.needs_staff === 1 ? 's' : ''} staff
				</p>
				<a class="text-sm font-semibold text-ink underline underline-offset-4" href="/staff-orders?status=needs_staff">
					Open orders
				</a>
			</div>
		{/if}

		<section aria-labelledby="kpis" class="flex flex-col gap-3">
			<h2 id="kpis" class="sr-only">Key numbers</h2>
			<ul class="m-0 grid list-none grid-cols-2 gap-3 p-0 md:grid-cols-4">
				{#each tiles as tile (tile.label)}
					<li class="min-w-0">
						<svelte:element
							this={tile.href ? 'a' : 'div'}
							href={tile.href}
							class={`group flex h-full flex-col gap-1 rounded-md border border-line bg-card p-4 text-ink no-underline shadow-sm transition-shadow duration-150 ${tile.href ? 'hover:border-focus hover:shadow-md' : ''}`}
						>
							<span class="flex items-center justify-between gap-2 text-sm text-ink-muted">
								<span class="truncate">{tile.label}</span>
								{#if tile.href}
									<ArrowUpRightIcon
										class="size-4 flex-none opacity-0 transition-opacity duration-150 group-hover:opacity-100"
										aria-hidden="true"
									/>
								{/if}
							</span>
							<span class="text-h2 font-semibold tracking-tight tabular-nums">{tile.value}</span>
							<span class="truncate text-xs text-ink-muted">{tile.hint}</span>
						</svelte:element>
					</li>
				{/each}
			</ul>
		</section>

		<section aria-labelledby="compose" class="flex flex-col gap-3">
			<div class="flex flex-wrap items-end justify-between gap-2">
				<div class="flex min-w-0 flex-col gap-1">
					<h2 id="compose" class="m-0 text-h3 font-semibold">Compose with Meta models</h2>
					<p class="m-0 text-sm text-ink-muted">
						Start a Muse Spark run and watch it pass the validator into the review queue.
					</p>
				</div>
				<div class="inline-flex gap-1 rounded-md border border-line bg-card p-1" role="group" aria-label="What to compose">
					<Button
						size="sm"
						variant={composeKind === 'plans' ? 'secondary' : 'ghost'}
						aria-pressed={composeKind === 'plans'}
						disabled={composeBusy}
						onclick={() => (composeKind = 'plans')}
					>
						Page plan
					</Button>
					<Button
						size="sm"
						variant={composeKind === 'offices' ? 'secondary' : 'ghost'}
						aria-pressed={composeKind === 'offices'}
						disabled={composeBusy}
						onclick={() => (composeKind = 'offices')}
					>
						Offices
					</Button>
				</div>
			</div>
			{#key composeKind}
				<ComposePanel compact kind={composeKind} bind:busy={composeBusy} />
			{/key}
		</section>

		<section aria-labelledby="meta" class="flex flex-col gap-3">
			<div class="flex flex-wrap items-baseline justify-between gap-2">
				<h2 id="meta" class="m-0 text-h3 font-semibold">Meta models</h2>
				<p class="m-0 text-sm text-ink-muted">
					{fmtInt(stats.meta.calls_24h)} calls in 24 h · {fmtPct(stats.meta.cache_share)} of input
					tokens from cache
				</p>
			</div>
			<ul class="m-0 grid list-none gap-3 p-0 md:grid-cols-3">
				{#each stats.meta.by_model as m (m.model)}
					{@const share = cacheShare(m)}
					<li class="min-w-0">
						<Card.Root class="h-full gap-3 shadow-sm">
							<Card.Header>
								<Card.Title class="flex items-center justify-between gap-2">
									<span class="text-body font-semibold">{modelName(m.model)}</span>
									{#if m.errors > 0}
										<Badge variant="outline" class="border-warning text-warning">
											<TriangleAlertIcon aria-hidden="true" />
											{m.errors} error{m.errors === 1 ? '' : 's'}
										</Badge>
									{:else}
										<Badge variant="outline" class="border-verified text-verified">No errors</Badge>
									{/if}
								</Card.Title>
								<Card.Description class="font-mono text-xs">{m.model}</Card.Description>
							</Card.Header>
							<Card.Content class="flex flex-col gap-3">
								<dl class="m-0 grid grid-cols-3 gap-2">
									<div>
										<dt class="text-xs text-ink-muted">Calls</dt>
										<dd class="m-0 text-h3 font-semibold tabular-nums">{fmtInt(m.calls)}</dd>
									</div>
									<div>
										<dt class="text-xs text-ink-muted">p50</dt>
										<dd class="m-0 text-h3 font-semibold tabular-nums">{fmtMs(m.p50_ms)}</dd>
									</div>
									<div>
										<dt class="text-xs text-ink-muted">p95</dt>
										<dd class="m-0 text-h3 font-semibold tabular-nums">{fmtMs(m.p95_ms)}</dd>
									</div>
								</dl>
								{#if m.input_tokens > 0}
									<div class="flex flex-col gap-1">
										<div class="flex justify-between text-xs text-ink-muted">
											<span>Cache share</span>
											<span class="font-mono text-ink">{fmtPct(share)}</span>
										</div>
										<div
											class="h-2 overflow-hidden rounded-full bg-surface-2"
											role="meter"
											aria-label={`${modelName(m.model)} cache share`}
											aria-valuemin={0}
											aria-valuemax={100}
											aria-valuenow={Math.round(share * 100)}
										>
											<div class="h-full rounded-full bg-accent-strong" style={`width:${Math.round(share * 100)}%`}></div>
										</div>
									</div>
									<p class="m-0 font-mono text-xs text-ink-muted">
										{fmtCompact(m.input_tokens)} in · {fmtCompact(m.output_tokens)} out · {fmtCompact(m.reasoning_tokens)} reasoning
									</p>
								{:else}
									<p class="m-0 text-xs text-ink-muted">Vision calls: no text tokens.</p>
								{/if}
							</Card.Content>
						</Card.Root>
					</li>
				{:else}
					<li class="text-sm text-ink-muted">No model calls in the last 24 hours.</li>
				{/each}
			</ul>
		</section>

		<div class="grid gap-3 lg:grid-cols-2">
			<Card.Root class="gap-4 shadow-sm">
				<Card.Header>
					<Card.Title class="text-body font-semibold">Model calls by hour</Card.Title>
					<Card.Description>Last {hourBars.length} hours, every Meta model</Card.Description>
				</Card.Header>
				<Card.Content>
					{#if hourBars.length > 0}
						<BarChart bars={hourBars} label="Model calls by hour" unit="calls" />
					{:else}
						<p class="m-0 text-sm text-ink-muted">No calls recorded yet.</p>
					{/if}
				</Card.Content>
			</Card.Root>
			<Card.Root class="gap-4 shadow-sm">
				<Card.Header>
					<Card.Title class="text-body font-semibold">Orders by day</Card.Title>
					<Card.Description>Last {dayBars.length} days · {fmtCents(stats.kpis.paid_cents_7d)} paid</Card.Description>
				</Card.Header>
				<Card.Content>
					{#if dayBars.length > 0}
						<BarChart bars={dayBars} label="Orders by day" unit="orders" />
					{:else}
						<p class="m-0 text-sm text-ink-muted">No orders yet.</p>
					{/if}
				</Card.Content>
			</Card.Root>
		</div>

		<div class="grid gap-3 lg:grid-cols-3">
			<Card.Root class="gap-4 shadow-sm">
				<Card.Header>
					<Card.Title class="text-body font-semibold">Calls by pipeline</Card.Title>
					<Card.Description>Where model time goes, 24 h</Card.Description>
				</Card.Header>
				<Card.Content>
					<ul class="m-0 flex list-none flex-col gap-2 p-0">
						{#each stats.meta.by_pipeline as p (p.pipeline)}
							<li class="grid grid-cols-4 items-center gap-2 text-sm">
								<span class="truncate text-ink-muted">{p.pipeline}</span>
								<span class="col-span-2 h-2" aria-hidden="true">
									<span
										class="block h-full rounded-r-xs bg-accent-strong"
										style={`width:${Math.max(2, (p.calls / pipelineMax) * 100)}%`}
									></span>
								</span>
								<span class="text-right font-mono tabular-nums">{fmtInt(p.calls)}</span>
							</li>
						{:else}
							<li class="text-sm text-ink-muted">No calls yet.</li>
						{/each}
					</ul>
				</Card.Content>
			</Card.Root>

			<Card.Root class="gap-4 shadow-sm">
				<Card.Header>
					<Card.Title class="text-body font-semibold">A5 eval, gold set</Card.Title>
					<Card.Description>
						{#if stats.meta.a5}
							{stats.meta.a5.cases} cases · graded {dateTime(stats.meta.a5.graded_at)}
						{:else}
							No graded run in the last 72 hours
						{/if}
					</Card.Description>
				</Card.Header>
				<Card.Content>
					{#if stats.meta.a5}
						{@const a5 = stats.meta.a5}
						<ul class="m-0 flex list-none flex-col gap-3 p-0">
							{#each [{ label: 'Quote found', rate: a5.quote_found_rate }, { label: 'Value in quote', rate: a5.value_in_quote_rate }, { label: 'Topic accuracy', rate: a5.topic_accuracy }] as r (r.label)}
								<li class="flex flex-col gap-1">
									<span class="flex justify-between text-sm">
										<span class="text-ink-muted">{r.label}</span>
										<span class="font-mono font-semibold tabular-nums">{fmtPct(r.rate)}</span>
									</span>
									<span
										class="h-2 overflow-hidden rounded-full bg-surface-2"
										role="meter"
										aria-label={r.label}
										aria-valuemin={0}
										aria-valuemax={100}
										aria-valuenow={Math.round(r.rate * 100)}
									>
										<span class="block h-full rounded-full bg-accent-strong" style={`width:${Math.round(r.rate * 100)}%`}></span>
									</span>
								</li>
							{/each}
						</ul>
					{:else}
						<Button variant="outline" href={resolve('/(admin)/trace')}>Open the trace console</Button>
					{/if}
				</Card.Content>
			</Card.Root>

			<Card.Root class="gap-4 shadow-sm">
				<Card.Header>
					<Card.Title class="text-body font-semibold">Activity</Card.Title>
					<Card.Description>Audit rows plus the live ops feed</Card.Description>
				</Card.Header>
				<Card.Content>
					{#if feed.length === 0}
						<div class="flex flex-col items-start gap-2">
							<p class="m-0 text-sm text-ink-muted">Quiet so far today.</p>
							<Button variant="outline" href={resolve('/(admin)/review')}>Open the review queue</Button>
						</div>
					{:else}
						<ol class="m-0 flex list-none flex-col gap-3 p-0" aria-label="Recent activity" aria-live="polite">
							{#each feed as row (`${row.kind}|${row.at}|${row.text}`)}
								{@const Icon = kindIcon(row.kind)}
								<li
									class="flex items-start gap-3"
									in:fly={{ y: -8, duration: motion }}
									animate:flip={{ duration: motion }}
								>
									<span class={`mt-0.5 inline-flex size-7 flex-none items-center justify-center rounded-sm ${kindTone(row.kind)}`}>
										<Icon class="size-4" aria-hidden="true" />
									</span>
									<span class="flex min-w-0 flex-1 flex-col">
										{#if row.href}
											<a class="text-sm text-ink no-underline hover:underline" href={row.href}>{row.text}</a>
										{:else}
											<span class="text-sm">{row.text}</span>
										{/if}
										<span class="flex items-center gap-2 text-xs text-ink-muted">
											<time datetime={new Date(row.at * 1000).toISOString()}>{ago(row.at, now)}</time>
											{#if row.live}
												<Badge variant="outline" class="h-4 border-verified px-1 text-verified">Live</Badge>
											{/if}
										</span>
									</span>
								</li>
							{/each}
						</ol>
					{/if}
				</Card.Content>
			</Card.Root>
		</div>
		{#if loadError}
			<p class="m-0 text-sm text-ink-muted" role="status">Last refresh failed: {loadError}</p>
		{/if}
	{/if}
</div>
