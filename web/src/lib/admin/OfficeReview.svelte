<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { fly } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { toast } from 'svelte-sonner';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import GuardList from './GuardList.svelte';
	import DecisionBar from './DecisionBar.svelte';
	import ComposePanel from './ComposePanel.svelte';
	import type { ComposeSummary } from './compose.svelte.js';
	import { OFFICE_ZONES, clock24, dayRange, decide, fetchOffices, type OfficeProposal } from './proposals.js';
	import { ApiError, errorText } from './admin-api.js';
	import MapPinIcon from '@lucide/svelte/icons/map-pin';
	import MailIcon from '@lucide/svelte/icons/mail';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import GlobeIcon from '@lucide/svelte/icons/globe';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

	// Offices tab: staff ask Muse Spark to structure accepted address,
	// hours and mail facts into office cards (ComposePanel, live over
	// `run:{id}`); guards check each value against its cited fact. A
	// timezone is kept only when quoted — otherwise staff confirm one
	// from the server's allowlist at accept. Accept → `offices_accepted`
	// on `authority:{slug}`, optimistic with rollback.
	let {
		onCount,
		initialSlug = '',
		initialRun = null
	}: { onCount?: (n: number) => void; initialSlug?: string; initialRun?: number | null } = $props();

	let offices = $state<OfficeProposal[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let acting = $state<number | null>(null);
	/** Staff timezone choice per office id ('' = none chosen). */
	let zones = $state<Record<number, string>>({});
	/** Offices the last finished run proposed (shown first, marked New). */
	let fresh = $state<number[]>([]);
	let freshRun = $state<number | null>(null);
	let composer = $state<ComposePanel | null>(null);
	let reduceMotion = $state(false);

	async function load(quiet = false) {
		if (!quiet) loadState = 'loading';
		try {
			const rows = await fetchOffices();
			// Zone state is seeded here (task context), never in render.
			for (const o of rows) {
				if (zones[o.id] === undefined) zones[o.id] = o.timezone ?? '';
			}
			offices = rows;
			loadState = 'ready';
			onCount?.(rows.filter((o) => o.status === 'proposed').length);
		} catch (err) {
			loadError = errorText(err, 'Offices failed to load.');
			if (!quiet || offices.length === 0) loadState = 'failed';
		}
	}

	async function onStructured(s: ComposeSummary, run: { runId: number | null; reopened: boolean }) {
		await load(true);
		fresh = s.proposed.length > 0 ? s.proposed : offices.filter((o) => o.run_id !== null && o.run_id === run.runId).map((o) => o.id);
		freshRun = run.runId;
		const first = ordered.find((o) => fresh.includes(o.id));
		// A reopened link keeps the page where it is; a live run brings
		// the new card into view.
		if (!first || run.reopened) return;
		await tick();
		document.getElementById(`office-${first.id}`)?.scrollIntoView({ block: 'nearest', behavior: reduceMotion ? 'auto' : 'smooth' });
	}

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		void load();
	});

	// Newly proposed first, then the server's newest-first order.
	const ordered = $derived(
		[...offices].sort((a, b) => Number(fresh.includes(b.id)) - Number(fresh.includes(a.id)))
	);
	const motion = $derived(reduceMotion ? 0 : 200);

	const VISIT: Record<string, string> = {
		walk_in: 'Walk-in',
		appointment: 'By appointment',
		mail_only: 'Mail only'
	};

	const zoneLabel = (value: string) => OFFICE_ZONES.find((z) => z.value === value)?.label ?? value;

	/** The accept body: a timezone only when staff chose or changed one. */
	function acceptExtra(o: OfficeProposal): Record<string, unknown> | undefined {
		const tz = zones[o.id] ?? '';
		if (!tz) return undefined;
		if (o.timezone_source === 'quoted' && tz === o.timezone) return undefined;
		return { timezone: tz };
	}

	async function act(o: OfficeProposal, action: 'accept' | 'reject', reason?: string) {
		if (acting !== null) return;
		acting = o.id;
		const prior = { status: o.status, timezone: o.timezone, source: o.timezone_source };
		const extra = action === 'accept' ? acceptExtra(o) : undefined;
		// Optimistic: the card reads decided (and confirmed) at once.
		o.status = action === 'accept' ? 'accepted' : 'rejected';
		if (extra) {
			o.timezone = String(extra.timezone);
			o.timezone_source = 'staff_confirmed';
		}
		try {
			o.status = await decide('offices', o.id, action, reason, extra);
			onCount?.(offices.filter((x) => x.status === 'proposed').length);
			toast.success(
				action === 'accept'
					? `${o.name} is live on ${o.authority_slug}${extra ? ` with ${zoneLabel(String(extra.timezone))} hours` : ''}.`
					: `${o.name} rejected.`
			);
		} catch (err) {
			o.status = prior.status;
			o.timezone = prior.timezone;
			o.timezone_source = prior.source;
			if (err instanceof ApiError && err.status === 409) {
				toast.info(`${o.name} was already decided — refreshing.`);
				void load(true);
			} else {
				toast.error(errorText(err, 'Decision failed — nothing changed.'));
			}
		} finally {
			acting = null;
		}
	}
</script>

<div class="flex flex-col gap-4">
	<ComposePanel bind:this={composer} kind="offices" {initialSlug} {initialRun} onDone={onStructured} />

	{#if loadState === 'loading'}
		<div class="grid gap-4 md:grid-cols-2" role="status" aria-label="Loading offices">
			<Skeleton class="h-64 rounded-md" />
			<Skeleton class="h-64 rounded-md" />
		</div>
	{:else if loadState === 'failed'}
		<Notice variant="warning" heading="Office proposals are unavailable.">
			<p class="m-0">{loadError}</p>
			<p class="mt-3 mb-0"><Button variant="outline" onclick={() => load()}>Try again</Button></p>
		</Notice>
	{:else if offices.length === 0}
		<Card.Root class="items-start px-6 py-10">
			<p class="m-0 text-h3 font-semibold">No offices waiting</p>
			<p class="m-0 text-sm text-ink-muted">Office cards are proposed after address and hours facts are accepted.</p>
			<Button variant="outline" onclick={() => composer?.focusPicker()}>Choose an authority to structure</Button>
		</Card.Root>
	{:else}
		<ul class="m-0 grid list-none gap-4 p-0 lg:grid-cols-2">
			{#each ordered as o (o.id)}
				{@const failing = o.guards.filter((g) => !g.ok && g.id !== 'timezone')}
				{@const tzGuard = o.guards.find((g) => g.id === 'timezone')}
				{@const quoted = o.timezone_source === 'quoted' && tzGuard?.ok !== false}
				<li class="min-w-0" id={`office-${o.id}`} animate:flip={{ duration: motion }} in:fly={{ y: -8, duration: motion }}>
					<Card.Root class={`h-full gap-4 shadow-sm ${fresh.includes(o.id) ? 'ring-2 ring-focus' : ''}`}>
						<Card.Header>
							<Card.Title class="flex flex-wrap items-center gap-2">
								<h2 class="m-0 text-body font-semibold">{o.name}</h2>
								{#if o.visit}<Badge variant="secondary">{VISIT[o.visit.kind] ?? o.visit.kind}</Badge>{/if}
								{#if fresh.includes(o.id)}
									<Badge variant="outline" class="border-focus text-ink">New{freshRun ? ` · run ${freshRun}` : ''}</Badge>
								{/if}
							</Card.Title>
							<Card.Description class="font-mono text-xs">{o.authority_slug} · office #{o.id}</Card.Description>
						</Card.Header>
						<Card.Content class="flex flex-col gap-4">
							<dl class="m-0 grid gap-3 text-sm sm:grid-cols-2">
								{#if o.address}
									<div>
										<dt class="flex items-center gap-1 text-xs text-ink-muted"><MapPinIcon class="size-3" aria-hidden="true" />Counter</dt>
										<dd class="m-0">
											{o.address.line1}{o.address.line2 ? `, ${o.address.line2}` : ''}<br />
											{o.address.city}, {o.address.region} {o.address.postal_code}
											{#if o.address.fact_id}<span class="font-mono text-xs whitespace-nowrap text-ink-muted"> · fact #{o.address.fact_id}</span>{/if}
										</dd>
									</div>
								{/if}
								{#if o.mail}
									<div>
										<dt class="flex items-center gap-1 text-xs text-ink-muted"><MailIcon class="size-3" aria-hidden="true" />Mail{o.mail.usps_only ? ' (USPS only)' : ''}</dt>
										<dd class="m-0">
											{o.mail.line1}<br />
											{o.mail.city}, {o.mail.region} {o.mail.postal_code}
											{#if o.mail.fact_id}<span class="font-mono text-xs whitespace-nowrap text-ink-muted"> · fact #{o.mail.fact_id}</span>{/if}
										</dd>
									</div>
								{/if}
								{#if o.hours.length > 0}
									<div>
										<dt class="flex items-center gap-1 text-xs text-ink-muted"><ClockIcon class="size-3" aria-hidden="true" />Hours</dt>
										{#each o.hours as h, hi (hi)}
											<dd class="m-0">
												{dayRange(h.days)} {clock24(h.open)}–{clock24(h.close)}
												{#if h.fact_id}<span class="font-mono text-xs whitespace-nowrap text-ink-muted"> · fact #{h.fact_id}</span>{/if}
											</dd>
										{/each}
									</div>
								{/if}
								{#if o.visit?.notes}
									<div>
										<dt class="text-xs text-ink-muted">Visit notes</dt>
										<dd class="m-0">{o.visit.notes}</dd>
									</div>
								{/if}
							</dl>

							<div class="flex flex-col gap-2 rounded-md border border-line px-3 py-2">
								<p class="m-0 flex flex-wrap items-center gap-2 text-sm">
									<GlobeIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
									<span id={`tz-label-${o.id}`} class="font-medium">Timezone</span>
									{#if o.timezone && o.timezone_source}
										<span class="font-mono text-xs text-ink-muted">{o.timezone} · {o.timezone_source.replace('_', ' ')}</span>
									{:else}
										<span class="text-xs text-ink-muted">Not quoted in any cited fact</span>
									{/if}
								</p>
								{#if o.status === 'proposed'}
									<Select.Root type="single" bind:value={zones[o.id]} disabled={acting === o.id}>
										<Select.Trigger
											id={`tz-${o.id}`}
											class="w-full"
											aria-labelledby={`tz-label-${o.id} tz-${o.id}`}
										>
											{zones[o.id] ? zoneLabel(zones[o.id]) : 'Choose a timezone to confirm'}
										</Select.Trigger>
										<Select.Content>
											{#each OFFICE_ZONES as z (z.value)}
												<Select.Item value={z.value} label={z.label}>
													<span class="flex min-w-0 flex-col">
														<span>{z.label}</span>
														<span class="font-mono text-xs text-ink-muted">{z.value}</span>
													</span>
												</Select.Item>
											{/each}
										</Select.Content>
									</Select.Root>
									<p class="m-0 text-xs text-ink-muted">
										{#if quoted && zones[o.id] === o.timezone}
											Quoted by the source. Accept keeps it.
										{:else if zones[o.id]}
											Accept records {zones[o.id]} as staff confirmed, so the dossier can show “Open now”.
										{:else}
											Without a timezone the dossier lists hours but never “Open now”.
										{/if}
									</p>
								{/if}
							</div>

							{#if failing.length > 0 && o.status === 'proposed'}
								<div class="flex items-start gap-2 rounded-sm bg-changed-bg px-3 py-2 text-sm" role="note">
									<TriangleAlertIcon class="mt-0.5 size-4 flex-none text-changed" aria-hidden="true" />
									<span>Accepting confirms {failing.map((g) => g.id.replaceAll('_', ' ')).join(', ')} on staff authority.</span>
								</div>
							{/if}
							<GuardList guards={o.guards} label={`Guards for ${o.name}`} />
							{#if o.directions_url}
								<a class="text-sm" href={o.directions_url} rel="noopener noreferrer" target="_blank">Check on the map</a>
							{/if}
							<DecisionBar
								noun="office"
								id={`office-decision-${o.id}`}
								status={o.status}
								acting={acting === o.id}
								onAccept={() => act(o, 'accept')}
								onReject={(reason) => act(o, 'reject', reason)}
							/>
						</Card.Content>
					</Card.Root>
				</li>
			{/each}
		</ul>
	{/if}
</div>
